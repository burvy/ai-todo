//! Persistence for v2 tasks (`docs/ARCHITECTURE.md` section 7).
//!
//! The server holds a `Store`, not a database. [`MemoryStore`] and
//! [`SqliteStore`] satisfy the same [`Store`] trait and the same test suite
//! runs against both, so the HTTP tests never touch a disk.
//!
//! v2 tables are named `v2_*` and live in their own file (`RAMBL_DATA`). They
//! deliberately share no table with v1's `tasks` / `busy_blocks`, so pointing
//! both at one file cannot corrupt the old data.

use std::path::Path;
use std::sync::Mutex;

use super::LockExt;
use super::tuning::{ACTIVITY_THROTTLE, DB_BUSY_TIMEOUT, USAGE_RETENTION_DAYS};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use todo_core::{
    DaySettings, ExceptionKind, Label, Priority, Series, SeriesException, SeriesId, Source, Task,
    TaskId, When,
};

fn parse_plan(value: &str) -> super::auth::Plan {
    match value {
        "anon" => super::auth::Plan::Anon,
        "paid" => super::auth::Plan::Paid,
        _ => super::auth::Plan::Free,
    }
}

/// The `file:line` of the call site, recorded in a [`Failure`].
macro_rules! at {
    () => {
        concat!(file!(), ":", line!())
    };
}

/// Where the v2 database lives when `RAMBL_DATA` is not set.
pub const DEFAULT_DATA_PATH: &str = "data/rambl.db";

/// The database file named by `RAMBL_DATA`, or [`DEFAULT_DATA_PATH`].
pub fn data_path_from_env() -> std::path::PathBuf {
    std::env::var("RAMBL_DATA")
        .map_or_else(|_| std::path::PathBuf::from(DEFAULT_DATA_PATH), Into::into)
}

/// A store that could not answer.
///
/// Reads and writes are both fallible here: the caller is `App`, which runs a
/// load-mutate-plan inside one critical section, and it needs to know the
/// difference between "no such task" and "the database is unhappy".
#[derive(Debug)]
pub enum StoreError {
    /// No task with that id (or, for `find_by_uid`, no task with that UID).
    NotFound,
    /// The write was refused: most often two tasks claiming one
    /// `external_uid`.
    Conflict(String),
    /// The database itself failed.
    Backend(Failure),
}

/// A database failure: where it happened, SQLite's extended error code when it gave
/// one, and its message. Kept whole so a log line says which call failed and why.
#[derive(Debug)]
pub struct Failure {
    /// `file:line` of the call that failed.
    pub at: &'static str,
    /// SQLite's extended result code, such as 266 for `SQLITE_IOERR_READ`.
    pub code: Option<i32>,
    pub message: String,
}

impl Failure {
    /// A failure that did not come from SQLite.
    pub fn plain(at: &'static str, message: String) -> Self {
        Self {
            at,
            code: None,
            message,
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.at)?;
        if let Some(code) = self.code {
            write!(f, " (sqlite {code})")?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::NotFound => write!(f, "no such task"),
            StoreError::Conflict(why) => write!(f, "conflict: {why}"),
            StoreError::Backend(failure) => write!(f, "storage failed at {failure}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Persistence for tasks.
///
/// Implementors must be safe to share across axum's worker threads.
pub trait Store: Send + Sync {
    /// Every task belonging to this user, in display order.
    ///
    /// The user id is a parameter rather than something the caller passes to a
    /// filter afterwards, so a handler cannot forget it and show one person's tasks to
    /// another.
    fn tasks(&self, user: &str) -> Result<Vec<Task>, StoreError>;

    /// Store `task` under its own id for this user, replacing any task already there.
    ///
    /// `put` is an upsert so it is total: the caller does not have to know
    /// whether the id is new. Use [`Store::find_by_uid`] before putting an
    /// imported task, so a re-import updates that task instead of colliding
    /// with it.
    fn put(&self, user: &str, task: Task) -> Result<(), StoreError>;

    /// Forget this user's task with that id. Reports whether there was one, so the
    /// caller can answer 404 rather than always 204.
    fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError>;

    /// Make this user's order exactly `ids`, first to `0`, `1`, ... Ids that are not
    /// stored are skipped. One transaction, so a reorder never half-applies.
    fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError>;

    /// This user's task imported from this iCalendar UID, if there is one.
    ///
    /// This is how a re-import updates instead of duplicating.
    fn find_by_uid(&self, user: &str, uid: &str) -> Result<Option<Task>, StoreError>;

    /// Every series belonging to this user, in display order.
    fn series(&self, user: &str) -> Result<Vec<Series>, StoreError>;

    /// Store `series` under its own id for this user, replacing any series
    /// already there. An upsert, like [`Store::put`].
    fn put_series(&self, user: &str, series: Series) -> Result<(), StoreError>;

    /// Forget this user's series with that id, and its exceptions with it.
    /// Reports whether there was one.
    fn delete_series(&self, user: &str, id: &SeriesId) -> Result<bool, StoreError>;

    /// Every exception this user has recorded on this series.
    fn exceptions(&self, user: &str, id: &SeriesId) -> Result<Vec<SeriesException>, StoreError>;

    /// Record (or replace) one occurrence's exception on this user's series.
    fn put_exception(
        &self,
        user: &str,
        id: &SeriesId,
        exception: SeriesException,
    ) -> Result<(), StoreError>;

    /// Forget one occurrence's exception, so the day falls back to the rule.
    /// Reports whether there was one.
    fn delete_exception(
        &self,
        user: &str,
        id: &SeriesId,
        date: NaiveDate,
    ) -> Result<bool, StoreError>;

    /// Create or refresh the user and open a session, in one transaction. `now` is the
    /// app's clock, recorded as when the user and the session were created.
    fn upsert_user_and_session(
        &self,
        user: &super::auth::Me,
        token: &str,
        now: chrono::DateTime<Utc>,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError>;

    /// End one session. Signing out is not signing in everywhere else.
    fn delete_session(&self, token: &str) -> Result<(), StoreError>;

    /// The user a live session token belongs to. `None` once it has expired.
    fn session_user(
        &self,
        token: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<super::auth::Me>, StoreError>;

    /// Forget a person, their sessions and every task they own.
    fn delete_user(&self, user: &str) -> Result<(), StoreError>;

    /// How many captures this user has run today, for the daily Jev cap.
    fn captures_today(&self, user: &str, day: chrono::NaiveDate) -> Result<u32, StoreError>;

    /// Move anonymous tasks to a signed-in user, keeping the signed-in copy for
    /// duplicate external UIDs.
    fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError>;
    fn user_plan(&self, user: &str) -> Result<super::auth::Plan, StoreError>;
    fn set_user_plan(&self, user: &str, plan: super::auth::Plan) -> Result<(), StoreError>;

    /// This user's day-only times, or the defaults for one who never changed them
    /// (or has no account row).
    fn day_settings(&self, user: &str) -> Result<DaySettings, StoreError>;
    /// Replace this user's day-only times. `NotFound` when the user has no account.
    fn set_day_settings(&self, user: &str, settings: DaySettings) -> Result<(), StoreError>;
    fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError>;
    /// Create the anonymous user `user`, first seen at `now`, unless it exists.
    fn ensure_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError>;

    /// Record that the anonymous user `user` was seen at `now`. Does nothing for a
    /// signed-in user (they never expire) and writes at most once per
    /// [`ACTIVITY_THROTTLE`], so a busy page is not a write per request.
    fn touch_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError>;

    /// Delete every anonymous user last seen at or before `cutoff`, with their tasks,
    /// in one transaction. Returns how many users went. Signed-in users are never
    /// touched.
    fn expire_anonymous(&self, cutoff: DateTime<Utc>) -> Result<usize, StoreError>;

    /// Count one capture by this user from this client address on this day.
    fn record_capture(&self, user: &str, ip: &str, day: NaiveDate) -> Result<(), StoreError>;

    /// Captures from this client address today, across every user.
    fn ip_captures_today(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError>;

    /// Count one more anonymous id issued to this client address today. Returns the
    /// count including this one, so the caller can refuse past a limit.
    fn record_anon_issued(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError>;

    /// Block a user or an address. Blocked traffic still works, but capture falls back
    /// to `FakeJudge`.
    fn block(&self, target: &Block) -> Result<(), StoreError>;

    /// Lift a block. Reports whether there was one.
    fn unblock(&self, target: &Block) -> Result<bool, StoreError>;

    /// Whether this user or this address is blocked.
    fn is_blocked(&self, user: &str, ip: &str) -> Result<bool, StoreError>;

    fn blocks(&self) -> Result<Vec<Block>, StoreError>;

    /// The busiest users and addresses today, most captures first.
    fn usage_today(&self, day: NaiveDate, top: usize) -> Result<Usage, StoreError>;

    /// Accounts ranked by Jev spend, most first. [`UsageOrder`] is the seam: a new
    /// ordering is a new variant, not a new query shape per caller.
    fn usage_ranking(&self, order: UsageOrder, top: usize) -> Result<Vec<UsageRow>, StoreError>;

    /// Whether this user's account has been paused, which is a block on the user id.
    /// An address block is a different thing and is not what this asks about.
    fn is_user_blocked(&self, user: &str) -> Result<bool, StoreError>;

    /// Whether an account with this id exists at all, so `pause` can say "no such
    /// account" instead of leaving a block on an id nobody holds.
    fn account_exists(&self, user: &str) -> Result<bool, StoreError>;

    /// The signed-in user with this email, if there is one.
    fn user_id_for_email(&self, email: &str) -> Result<Option<String>, StoreError>;
}

/// Something an administrator can block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    User(String),
    Ip(String),
}

/// Today's busiest users and client addresses, with their capture counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub users: Vec<(String, u32)>,
    pub ips: Vec<(String, u32)>,
}

/// How to rank accounts by Jev spend. A new ordering is a new variant here, so every
/// store answers it from one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageOrder {
    /// Every call the account has ever made.
    AllTime,
    /// Calls made on this local day.
    Day(NaiveDate),
}

/// One account's row in a spend ranking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageRow {
    pub user: String,
    /// Empty for an account with no email, which is what an anonymous visitor has.
    pub email: String,
    pub plan: super::auth::Plan,
    pub calls: u32,
}

mod schema;
mod sqlite;
// The tests reach the backend's schema and row mapping through this module.
#[cfg(test)]
use self::{schema::*, sqlite::*};
pub use sqlite::SqliteStore;
#[cfg(test)]
mod memory;
#[cfg(test)]
pub use memory::MemoryStore;

impl<S: Store + ?Sized> Store for Box<S> {
    fn tasks(&self, user: &str) -> Result<Vec<Task>, StoreError> {
        (**self).tasks(user)
    }

    fn put(&self, user: &str, task: Task) -> Result<(), StoreError> {
        (**self).put(user, task)
    }

    fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError> {
        (**self).delete(user, id)
    }

    fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError> {
        (**self).set_order(user, ids)
    }

    fn find_by_uid(&self, user: &str, uid: &str) -> Result<Option<Task>, StoreError> {
        (**self).find_by_uid(user, uid)
    }

    fn series(&self, user: &str) -> Result<Vec<Series>, StoreError> {
        (**self).series(user)
    }

    fn put_series(&self, user: &str, series: Series) -> Result<(), StoreError> {
        (**self).put_series(user, series)
    }

    fn delete_series(&self, user: &str, id: &SeriesId) -> Result<bool, StoreError> {
        (**self).delete_series(user, id)
    }

    fn exceptions(&self, user: &str, id: &SeriesId) -> Result<Vec<SeriesException>, StoreError> {
        (**self).exceptions(user, id)
    }

    fn put_exception(
        &self,
        user: &str,
        id: &SeriesId,
        exception: SeriesException,
    ) -> Result<(), StoreError> {
        (**self).put_exception(user, id, exception)
    }

    fn delete_exception(
        &self,
        user: &str,
        id: &SeriesId,
        date: NaiveDate,
    ) -> Result<bool, StoreError> {
        (**self).delete_exception(user, id, date)
    }

    fn upsert_user_and_session(
        &self,
        user: &super::auth::Me,
        token: &str,
        now: chrono::DateTime<Utc>,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        (**self).upsert_user_and_session(user, token, now, expires_at)
    }

    fn delete_session(&self, token: &str) -> Result<(), StoreError> {
        (**self).delete_session(token)
    }

    fn session_user(
        &self,
        token: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<super::auth::Me>, StoreError> {
        (**self).session_user(token, now)
    }

    fn delete_user(&self, user: &str) -> Result<(), StoreError> {
        (**self).delete_user(user)
    }

    fn captures_today(&self, user: &str, day: chrono::NaiveDate) -> Result<u32, StoreError> {
        (**self).captures_today(user, day)
    }

    fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError> {
        (**self).adopt_anonymous(anonymous, user)
    }
    fn user_plan(&self, user: &str) -> Result<super::auth::Plan, StoreError> {
        (**self).user_plan(user)
    }
    fn set_user_plan(&self, user: &str, plan: super::auth::Plan) -> Result<(), StoreError> {
        (**self).set_user_plan(user, plan)
    }
    fn day_settings(&self, user: &str) -> Result<DaySettings, StoreError> {
        (**self).day_settings(user)
    }
    fn set_day_settings(&self, user: &str, settings: DaySettings) -> Result<(), StoreError> {
        (**self).set_day_settings(user, settings)
    }
    fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError> {
        (**self).anonymous_exists(user)
    }
    fn ensure_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
        (**self).ensure_anonymous(user, now)
    }
    fn touch_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
        (**self).touch_anonymous(user, now)
    }
    fn expire_anonymous(&self, cutoff: DateTime<Utc>) -> Result<usize, StoreError> {
        (**self).expire_anonymous(cutoff)
    }
    fn record_capture(&self, user: &str, ip: &str, day: NaiveDate) -> Result<(), StoreError> {
        (**self).record_capture(user, ip, day)
    }
    fn ip_captures_today(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError> {
        (**self).ip_captures_today(ip, day)
    }
    fn record_anon_issued(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError> {
        (**self).record_anon_issued(ip, day)
    }
    fn block(&self, target: &Block) -> Result<(), StoreError> {
        (**self).block(target)
    }
    fn unblock(&self, target: &Block) -> Result<bool, StoreError> {
        (**self).unblock(target)
    }
    fn is_blocked(&self, user: &str, ip: &str) -> Result<bool, StoreError> {
        (**self).is_blocked(user, ip)
    }
    fn blocks(&self) -> Result<Vec<Block>, StoreError> {
        (**self).blocks()
    }
    fn usage_today(&self, day: NaiveDate, top: usize) -> Result<Usage, StoreError> {
        (**self).usage_today(day, top)
    }
    fn usage_ranking(&self, order: UsageOrder, top: usize) -> Result<Vec<UsageRow>, StoreError> {
        (**self).usage_ranking(order, top)
    }
    fn is_user_blocked(&self, user: &str) -> Result<bool, StoreError> {
        (**self).is_user_blocked(user)
    }
    fn account_exists(&self, user: &str) -> Result<bool, StoreError> {
        (**self).account_exists(user)
    }
    fn user_id_for_email(&self, email: &str) -> Result<Option<String>, StoreError> {
        (**self).user_id_for_email(email)
    }
}

// ---- series (ARCHITECTURE.md 7.1) ----

#[cfg(test)]
mod tests;

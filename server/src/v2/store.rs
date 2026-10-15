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

/// Bumped whenever [`MIGRATIONS`] grows an entry. The count on disk is
/// `PRAGMA user_version`; anything below this has the remaining migrations
/// applied on open.
const SCHEMA_VERSION: i64 = 13;

/// Ordered by `user_version`. Entry n applies to a database sitting at
/// version n - 1, so the list is append-only and never reordered.
const MIGRATIONS: &[&str] = &[
    MIGRATION_1,
    MIGRATION_2,
    MIGRATION_3,
    MIGRATION_4,
    MIGRATION_5,
    MIGRATION_6,
    MIGRATION_7,
    MIGRATION_8,
    MIGRATION_9,
    MIGRATION_10,
    MIGRATION_11,
    MIGRATION_12,
    MIGRATION_13,
];

const MIGRATION_1: &str = "
CREATE TABLE IF NOT EXISTS v2_tasks (
    id           TEXT PRIMARY KEY,
    title        TEXT NOT NULL,
    duration_min INTEGER NOT NULL,
    -- `When` is a tagged enum on the wire ({\"kind\": ..., \"time\": ...});
    -- in SQL it is the kind plus the instant it may carry.
    when_kind    TEXT NOT NULL,
    when_time    TEXT,
    done         INTEGER NOT NULL DEFAULT 0,
    \"order\"      INTEGER NOT NULL DEFAULT 0,
    label        TEXT NOT NULL,
    priority     TEXT NOT NULL,
    source       TEXT NOT NULL,
    created      TEXT NOT NULL,
    -- iCalendar UID of an imported event; NULL for everything else.
    external_uid TEXT
);
CREATE INDEX IF NOT EXISTS v2_tasks_order_idx ON v2_tasks (\"order\");
-- Re-importing the same UID must find the same task, so a UID can never be
-- held by two rows. Partial, because most tasks have no UID at all.
CREATE UNIQUE INDEX IF NOT EXISTS v2_tasks_uid_idx
    ON v2_tasks (external_uid) WHERE external_uid IS NOT NULL;
";

/// Accounts. A task belongs to a `user_id`; every read and write is scoped to one, so
/// two people never see each other's tasks. Existing rows get `LOCAL_USER` so a
/// database written before sign-in keeps its tasks instead of becoming invisible.
const MIGRATION_2: &str = "
CREATE TABLE IF NOT EXISTS v2_users (
    id         TEXT PRIMARY KEY,
    email      TEXT NOT NULL,
    name       TEXT NOT NULL,
    picture    TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS v2_sessions (
    token      TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS v2_sessions_user_idx ON v2_sessions (user_id);
-- Tasks belong to a user. SQLite cannot add a NOT NULL column with a REFERENCES
-- constraint and a default in one statement, so `delete_user` removes the tasks in
-- the same transaction instead of leaning on the engine to cascade.
ALTER TABLE v2_tasks ADD COLUMN user_id TEXT NOT NULL DEFAULT 'local';
CREATE INDEX IF NOT EXISTS v2_tasks_user_idx ON v2_tasks (user_id);
INSERT OR IGNORE INTO v2_users (id, email, name, picture, created_at)
    VALUES ('local', 'local@localhost', 'Local user', NULL, '1970-01-01T00:00:00Z');
";
const MIGRATION_3: &str = "ALTER TABLE v2_users ADD COLUMN plan TEXT NOT NULL DEFAULT 'free';";
/// Per-user capture counts per client address per day, for the Jev caps and `usage`.
/// Old days are pruned on write, so the table holds about a month.
const MIGRATION_4: &str = "
CREATE TABLE IF NOT EXISTS v2_capture_counts (
    user_id TEXT NOT NULL,
    ip      TEXT NOT NULL,
    day     TEXT NOT NULL,
    n       INTEGER NOT NULL,
    PRIMARY KEY (user_id, ip, day)
);
CREATE INDEX IF NOT EXISTS v2_capture_counts_ip_idx ON v2_capture_counts (ip, day);
CREATE TABLE IF NOT EXISTS v2_anon_issued (
    ip  TEXT NOT NULL,
    day TEXT NOT NULL,
    n   INTEGER NOT NULL,
    PRIMARY KEY (ip, day)
);
CREATE TABLE IF NOT EXISTS v2_blocks (
    kind  TEXT NOT NULL CHECK (kind IN ('user', 'ip')),
    value TEXT NOT NULL,
    PRIMARY KEY (kind, value)
);
";

/// Calendar UIDs are unique per person, not across the database: two people can import the
/// same event from the same export. The old index was global and made the second import a
/// conflict.
const MIGRATION_5: &str = "
DROP INDEX IF EXISTS v2_tasks_uid_idx;
CREATE UNIQUE INDEX IF NOT EXISTS v2_tasks_uid_idx
    ON v2_tasks (user_id, external_uid) WHERE external_uid IS NOT NULL;
";

/// `updated_at` for last-write-wins edits, and `last_activity_at` for expiring idle
/// anonymous users. SQLite refuses a non-constant `DEFAULT` on `ADD COLUMN`, so each
/// column gets a constant placeholder and the existing rows are then filled in: a task's
/// version is when it was created, an anonymous user's last visit is the migration time
/// (nobody is expired for being idle before this existed). Times are written as
/// `format_ts` writes them, so the text compares correctly.
const MIGRATION_6: &str = "
ALTER TABLE v2_tasks ADD COLUMN updated_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00+00:00';
UPDATE v2_tasks SET updated_at = created;
ALTER TABLE v2_users ADD COLUMN last_activity_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00+00:00';
UPDATE v2_users SET last_activity_at = strftime('%Y-%m-%dT%H:%M:%S+00:00', 'now');
";

const MIGRATION_7: &str = "ALTER TABLE v2_tasks ADD COLUMN due_tz TEXT;";

/// Weekly series (ARCHITECTURE.md 7.1). The rows never list the weeks; the
/// server expands occurrences at snapshot time. `weekdays` holds the
/// `chrono::Weekday` numbers Monday=1..Sunday=7 as digits (`"124"` = Mon/Tue/Thu).
/// A deviation from the rule lives in `v2_series_exceptions`, one per local
/// date: `done` / `skipped` / `moved`, with the moved target as the same JSON
/// a task's `When` uses on the wire.
const MIGRATION_8: &str = "
CREATE TABLE IF NOT EXISTS v2_series (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL,
    title        TEXT NOT NULL,
    duration_min INTEGER NOT NULL,
    weekdays     TEXT NOT NULL,
    until        TEXT,
    start_day    TEXT NOT NULL,
    time_of_day  TEXT,
    zone         TEXT NOT NULL,
    external_uid TEXT,
    created      TEXT NOT NULL,
    \"order\"      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS v2_series_user_idx ON v2_series (user_id);
CREATE TABLE IF NOT EXISTS v2_series_exceptions (
    series_id  TEXT NOT NULL,
    user_id    TEXT NOT NULL,
    date       TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('done', 'skipped', 'moved')),
    moved_when TEXT,
    moved_tz   TEXT,
    PRIMARY KEY (user_id, series_id, date)
);
CREATE INDEX IF NOT EXISTS v2_series_exceptions_series_idx
    ON v2_series_exceptions (user_id, series_id);
";

/// How many Jev calls each account has ever made.
///
/// `v2_capture_counts` keeps one row per day and is pruned after 30 days, so it can
/// answer "today" but not "all time". The admin console ranks by all time, so the
/// running total is kept separately and never pruned.
const MIGRATION_9: &str = "
CREATE TABLE IF NOT EXISTS v2_usage_totals (
    user_id TEXT PRIMARY KEY,
    calls  INTEGER NOT NULL DEFAULT 0
);
";

/// Session tokens are stored hashed. The rows already there are hashed in Rust, right
/// after this runs (`hash_plain_sessions`), because SQLite has no SHA-256 of its own.
const MIGRATION_10: &str = "SELECT 1;";

/// The two times of day a day-only task depends on (`DaySettings`), as `HH:MM`. NULL
/// means the default, so a row nobody has changed follows the default if it moves.
const MIGRATION_11: &str = "
ALTER TABLE v2_users ADD COLUMN day_deadline TEXT;
ALTER TABLE v2_users ADD COLUMN day_placement TEXT;
";

/// The user took a task off the planner (ARCHITECTURE.md 4.2). The default is 0, so a
/// row written before the flag existed is still planned, as it always was.
const MIGRATION_12: &str =
    "ALTER TABLE v2_tasks ADD COLUMN off_planner INTEGER NOT NULL DEFAULT 0;";

/// How a day-setting time is written in the database and on the wire.
const DAY_TIME_FORMAT: &str = "%H:%M";

/// Interval column for series: every N weeks (weekly) or every N days (daily).
const MIGRATION_13: &str = "
ALTER TABLE v2_series ADD COLUMN interval INTEGER NOT NULL DEFAULT 1;
";

/// A stored day-setting time, or `fallback` when the column is NULL or unreadable
/// (a hand-edited row should not take a user's planner down).
fn stored_time(value: Option<String>, fallback: chrono::NaiveTime) -> chrono::NaiveTime {
    value
        .and_then(|text| chrono::NaiveTime::parse_from_str(&text, DAY_TIME_FORMAT).ok())
        .unwrap_or(fallback)
}

/// What is stored for a session token: its SHA-256 in hex. A copy of the database, or of
/// a backup, then holds nothing that works as a login.
fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Hash every session token still stored as issued. Tokens are 32 hex characters and
/// hashes are 64, so a row already done is never hashed twice.
fn hash_plain_sessions(conn: &Connection) -> rusqlite::Result<()> {
    let plain: Vec<String> = conn
        .prepare("SELECT token FROM v2_sessions WHERE length(token) = 32")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for token in plain {
        conn.execute(
            "UPDATE v2_sessions SET token = ?1 WHERE token = ?2",
            params![hash_token(&token), token],
        )?;
    }
    Ok(())
}

/// Every column of `v2_tasks`, in one list.
///
/// Reads and writes are built from this string, so a field added here cannot
/// be added to the `SELECT` and forgotten in the `INSERT`. The index below is
/// checked against it by a test.
const TASK_COLUMNS: &str = "id, title, duration_min, when_kind, when_time, done, \"order\", \
                            label, priority, source, created, external_uid, updated_at, due_tz, off_planner";

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

/// Tasks in a SQLite file of its own, on tables of its own.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open the database named by `RAMBL_DATA`, or [`DEFAULT_DATA_PATH`].
    pub fn open_from_env() -> Result<Self, StoreError> {
        Self::open(&data_path_from_env())
    }

    /// Open the database at `path`, creating the folder and applying any
    /// migrations it is behind on.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                StoreError::Backend(Failure::plain(
                    at!(),
                    format!("could not create {}: {e}", parent.display()),
                ))
            })?;
        }
        let conn = Connection::open(path).map_err(|e| backend(e, at!()))?;
        // The admin console opens a second handle on the same file. Wait for a write
        // lock rather than failing a request straight away.
        conn.busy_timeout(DB_BUSY_TIMEOUT)
            .map_err(|e| backend(e, at!()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    /// A migrated database that lives only as long as this handle, for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let store = Self {
            conn: Mutex::new(Connection::open_in_memory().map_err(|e| backend(e, at!()))?),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Bring the schema up to [`SCHEMA_VERSION`]. Runs on every open; it is a
    /// no-op once the file is current.
    fn migrate(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock_recover();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| backend(e, at!()))?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| backend(e, at!()))?;
        // Only what this file has not seen: a migration is not safe to run twice (an
        // `ADD COLUMN` fails the second time), and the file already holds the first
        // `version` of them.
        for statement in MIGRATIONS.iter().skip(version as usize) {
            tx.execute_batch(statement).map_err(|e| backend(e, at!()))?;
        }
        if version < 10 {
            hash_plain_sessions(&tx).map_err(|e| backend(e, at!()))?;
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| backend(e, at!()))?;
        tx.commit().map_err(|e| backend(e, at!()))
    }

    /// Number of rows held, for the tests that count directly.
    #[cfg(test)]
    fn count(&self) -> i64 {
        self.conn
            .lock_recover()
            .query_row("SELECT COUNT(*) FROM v2_tasks", [], |row| row.get(0))
            .expect("count")
    }
}

impl Store for SqliteStore {
    fn tasks(&self, user: &str) -> Result<Vec<Task>, StoreError> {
        let conn = self.conn.lock_recover();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM v2_tasks WHERE user_id = ?1 \
                 ORDER BY \"order\" ASC, created ASC, id ASC"
            ))
            .map_err(|e| backend(e, at!()))?;
        let rows = stmt
            .query_map(params![user], read_task)
            .map_err(|e| backend(e, at!()))?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row.map_err(|e| backend(e, at!()))?);
        }
        Ok(tasks)
    }

    fn put(&self, user: &str, task: Task) -> Result<(), StoreError> {
        let (when_kind, when_time) = when_to_parts(task.when);
        let conn = self.conn.lock_recover();
        // The id is unique across every user, so an id already held by someone else is
        // refused, not taken over: the `WHERE` makes that upsert change nothing.
        let written = conn
            .execute(
                &format!(
                    "INSERT INTO v2_tasks ({TASK_COLUMNS}, user_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16) \
                 ON CONFLICT(id) DO UPDATE SET \
                     title = excluded.title, \
                     duration_min = excluded.duration_min, \
                     when_kind = excluded.when_kind, \
                     when_time = excluded.when_time, \
                     done = excluded.done, \
                     \"order\" = excluded.\"order\", \
                     label = excluded.label, \
                     priority = excluded.priority, \
                     source = excluded.source, \
                     created = excluded.created, \
                     external_uid = excluded.external_uid, \
                     updated_at = excluded.updated_at, \
                     due_tz = excluded.due_tz, \
                     off_planner = excluded.off_planner \
                 WHERE v2_tasks.user_id = excluded.user_id"
                ),
                params![
                    task.id.0,
                    task.title,
                    i64::from(task.duration_min),
                    when_kind,
                    when_time.map(format_ts),
                    i64::from(task.done),
                    task.order,
                    label_to_stored(task.label),
                    priority_to_stored(task.priority),
                    source_to_stored(task.source),
                    format_ts(task.created),
                    task.external_uid,
                    format_ts(task.updated_at),
                    task.due_tz,
                    i64::from(task.off_planner),
                    user,
                ],
            )
            .map_err(conflict)?;
        if written == 0 {
            return Err(StoreError::Conflict(format!(
                "task id {} belongs to another user",
                task.id.0
            )));
        }
        Ok(())
    }

    fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError> {
        let removed = self
            .conn
            .lock_recover()
            .execute(
                "DELETE FROM v2_tasks WHERE id = ?1 AND user_id = ?2",
                params![id.0, user],
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(removed > 0)
    }

    fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError> {
        let mut conn = self.conn.lock_recover();
        let tx = conn.transaction().map_err(|e| backend(e, at!()))?;
        {
            let mut stmt = tx
                .prepare("UPDATE v2_tasks SET \"order\" = ?3 WHERE id = ?1 AND user_id = ?2")
                .map_err(|e| backend(e, at!()))?;
            for (position, id) in ids.iter().enumerate() {
                stmt.execute(params![id.0, user, position as i64])
                    .map_err(|e| backend(e, at!()))?;
            }
        }
        tx.commit().map_err(|e| backend(e, at!()))
    }

    fn find_by_uid(&self, user: &str, uid: &str) -> Result<Option<Task>, StoreError> {
        let conn = self.conn.lock_recover();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM v2_tasks WHERE external_uid = ?1 AND user_id = ?2"
            ))
            .map_err(|e| backend(e, at!()))?;
        let mut rows = stmt
            .query(params![uid, user])
            .map_err(|e| backend(e, at!()))?;
        let Some(row) = rows.next().map_err(|e| backend(e, at!()))? else {
            return Ok(None);
        };
        Ok(Some(read_task(row).map_err(|e| backend(e, at!()))?))
    }

    fn series(&self, user: &str) -> Result<Vec<Series>, StoreError> {
        let conn = self.conn.lock_recover();
        let mut stmt = conn
            .prepare(
                "SELECT id, title, duration_min, weekdays, interval, until, start_day, time_of_day, zone, \
                 external_uid, created, \"order\" FROM v2_series WHERE user_id = ?1 \
                 ORDER BY \"order\" ASC, created ASC, id ASC",
            )
            .map_err(|e| backend(e, at!()))?;
        let rows = stmt
            .query_map(params![user], read_series)
            .map_err(|e| backend(e, at!()))?;
        let mut held = Vec::new();
        for row in rows {
            held.push(row.map_err(|e| backend(e, at!()))?);
        }
        Ok(held)
    }

    fn put_series(&self, user: &str, series: Series) -> Result<(), StoreError> {
        let conn = self.conn.lock_recover();
        // Same ownership rule as `put`: an id held by someone else is refused,
        // never taken over.
        let written = conn
            .execute(
                "INSERT INTO v2_series (id, user_id, title, duration_min, weekdays, interval, until, \
                 start_day, time_of_day, zone, external_uid, created, \"order\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
                 ON CONFLICT(id) DO UPDATE SET \
                     title = excluded.title, \
                     duration_min = excluded.duration_min, \
                     weekdays = excluded.weekdays, \
                     interval = excluded.interval, \
                     until = excluded.until, \
                     start_day = excluded.start_day, \
                     time_of_day = excluded.time_of_day, \
                     zone = excluded.zone, \
                     external_uid = excluded.external_uid, \
                     created = excluded.created, \
                     \"order\" = excluded.\"order\" \
                 WHERE v2_series.user_id = excluded.user_id",
                params![
                    series.id.0,
                    user,
                    series.title,
                    i64::from(series.duration_min),
                    weekdays_to_stored(&series.rule.weekdays),
                    i64::from(series.rule.interval),
                    series.rule.until.map(|day| day.to_string()),
                    series.start_day.to_string(),
                    series.time_of_day.map(format_time_of_day),
                    series.zone,
                    series.external_uid,
                    format_ts(series.created),
                    series.order,
                ],
            )
            .map_err(conflict)?;
        if written == 0 {
            return Err(StoreError::Conflict(format!(
                "series id {} belongs to another user",
                series.id.0
            )));
        }
        Ok(())
    }

    fn delete_series(&self, user: &str, id: &SeriesId) -> Result<bool, StoreError> {
        let mut conn = self.conn.lock_recover();
        let tx = conn.transaction().map_err(|e| backend(e, at!()))?;
        let removed = tx
            .execute(
                "DELETE FROM v2_series WHERE id = ?1 AND user_id = ?2",
                params![id.0, user],
            )
            .map_err(|e| backend(e, at!()))?;
        tx.execute(
            "DELETE FROM v2_series_exceptions WHERE series_id = ?1 AND user_id = ?2",
            params![id.0, user],
        )
        .map_err(|e| backend(e, at!()))?;
        tx.commit().map_err(|e| backend(e, at!()))?;
        Ok(removed > 0)
    }

    fn exceptions(&self, user: &str, id: &SeriesId) -> Result<Vec<SeriesException>, StoreError> {
        let conn = self.conn.lock_recover();
        let mut stmt = conn
            .prepare(
                "SELECT date, kind, moved_when, moved_tz FROM v2_series_exceptions \
                 WHERE user_id = ?1 AND series_id = ?2 ORDER BY date ASC",
            )
            .map_err(|e| backend(e, at!()))?;
        let rows = stmt
            .query_map(params![user, id.0], read_exception)
            .map_err(|e| backend(e, at!()))?;
        let mut held = Vec::new();
        for row in rows {
            held.extend(row.map_err(|e| backend(e, at!()))?);
        }
        Ok(held)
    }

    fn put_exception(
        &self,
        user: &str,
        id: &SeriesId,
        exception: SeriesException,
    ) -> Result<(), StoreError> {
        let (kind, moved_when, moved_tz) = exception_to_parts(&exception.kind);
        self.conn
            .lock_recover()
            .execute(
                "INSERT INTO v2_series_exceptions (series_id, user_id, date, kind, moved_when, moved_tz) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(user_id, series_id, date) DO UPDATE SET \
                     kind = excluded.kind, \
                     moved_when = excluded.moved_when, \
                     moved_tz = excluded.moved_tz",
                params![
                    id.0,
                    user,
                    exception.date.to_string(),
                    kind,
                    moved_when,
                    moved_tz,
                ],
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(())
    }

    fn delete_exception(
        &self,
        user: &str,
        id: &SeriesId,
        date: NaiveDate,
    ) -> Result<bool, StoreError> {
        let removed = self
            .conn
            .lock_recover()
            .execute(
                "DELETE FROM v2_series_exceptions \
                 WHERE user_id = ?1 AND series_id = ?2 AND date = ?3",
                params![user, id.0, date.to_string()],
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(removed > 0)
    }

    fn upsert_user_and_session(
        &self,
        user: &super::auth::Me,
        token: &str,
        now: chrono::DateTime<Utc>,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock_recover();
        let tx = conn.transaction().map_err(|e| backend(e, at!()))?;
        // First sign-in creates the user, later ones refresh the name and picture, so
        // a rename at Google reaches the app.
        tx.execute(
            "INSERT INTO v2_users (id, email, name, picture, created_at, plan) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(id) DO UPDATE SET \
                 email = excluded.email, name = excluded.name, picture = excluded.picture",
            params![
                user.id,
                user.email,
                user.name,
                user.picture,
                format_ts(now),
                user.plan.as_str()
            ],
        )
        .map_err(|e| backend(e, at!()))?;
        tx.execute(
            "INSERT INTO v2_sessions (token, user_id, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![
                hash_token(token),
                user.id,
                format_ts(expires_at),
                format_ts(now)
            ],
        )
        .map_err(|e| backend(e, at!()))?;
        tx.commit().map_err(|e| backend(e, at!()))
    }

    fn delete_session(&self, token: &str) -> Result<(), StoreError> {
        self.conn
            .lock_recover()
            .execute(
                "DELETE FROM v2_sessions WHERE token = ?1",
                params![hash_token(token)],
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(())
    }

    fn session_user(
        &self,
        token: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<super::auth::Me>, StoreError> {
        let conn = self.conn.lock_recover();
        let mut stmt = conn
            .prepare(
                "SELECT v2_users.id, v2_users.email, v2_users.name, v2_users.picture, v2_users.plan \
                 FROM v2_sessions JOIN v2_users ON v2_users.id = v2_sessions.user_id \
                 WHERE v2_sessions.token = ?1 AND v2_sessions.expires_at > ?2",
            )
            .map_err(|e| backend(e, at!()))?;
        let mut rows = stmt
            .query(params![hash_token(token), format_ts(now)])
            .map_err(|e| backend(e, at!()))?;
        let Some(row) = rows.next().map_err(|e| backend(e, at!()))? else {
            return Ok(None);
        };
        Ok(Some(super::auth::Me {
            id: row.get(0).map_err(|e| backend(e, at!()))?,
            email: row.get(1).map_err(|e| backend(e, at!()))?,
            name: row.get(2).map_err(|e| backend(e, at!()))?,
            picture: row.get(3).map_err(|e| backend(e, at!()))?,
            plan: parse_plan(&row.get::<_, String>(4).map_err(|e| backend(e, at!()))?),
        }))
    }

    fn delete_user(&self, user: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock_recover();
        let tx = conn.transaction().map_err(|e| backend(e, at!()))?;
        // Tasks go first: SQLite cannot cascade here because user_id is a column
        // added by migration with a default, not a declared constraint.
        tx.execute("DELETE FROM v2_tasks WHERE user_id = ?1", params![user])
            .map_err(|e| backend(e, at!()))?;
        tx.execute("DELETE FROM v2_series WHERE user_id = ?1", params![user])
            .map_err(|e| backend(e, at!()))?;
        tx.execute(
            "DELETE FROM v2_series_exceptions WHERE user_id = ?1",
            params![user],
        )
        .map_err(|e| backend(e, at!()))?;
        tx.execute("DELETE FROM v2_sessions WHERE user_id = ?1", params![user])
            .map_err(|e| backend(e, at!()))?;
        tx.execute("DELETE FROM v2_users WHERE id = ?1", params![user])
            .map_err(|e| backend(e, at!()))?;
        tx.commit().map_err(|e| backend(e, at!()))
    }

    fn captures_today(&self, user: &str, day: chrono::NaiveDate) -> Result<u32, StoreError> {
        let count: i64 = self
            .conn
            .lock_recover()
            .query_row(
                "SELECT COALESCE(SUM(n), 0) FROM v2_capture_counts WHERE user_id = ?1 AND day = ?2",
                params![user, day.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(count as u32)
    }

    fn record_capture(&self, user: &str, ip: &str, day: NaiveDate) -> Result<(), StoreError> {
        let conn = self.conn.lock_recover();
        conn.execute(
            "INSERT INTO v2_capture_counts (user_id, ip, day, n) VALUES (?1, ?2, ?3, 1)
             ON CONFLICT (user_id, ip, day) DO UPDATE SET n = n + 1",
            params![user, ip, day.to_string()],
        )
        .map_err(|e| backend(e, at!()))?;
        // The all-time total, which the per-day counts cannot answer for once they are
        // pruned.
        conn.execute(
            "INSERT INTO v2_usage_totals (user_id, calls) VALUES (?1, 1)
             ON CONFLICT (user_id) DO UPDATE SET calls = calls + 1",
            params![user],
        )
        .map_err(|e| backend(e, at!()))?;
        prune_before(&conn, day)
    }

    fn ip_captures_today(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError> {
        let count: i64 = self
            .conn
            .lock_recover()
            .query_row(
                "SELECT COALESCE(SUM(n), 0) FROM v2_capture_counts WHERE ip = ?1 AND day = ?2",
                params![ip, day.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(count as u32)
    }

    fn record_anon_issued(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError> {
        let conn = self.conn.lock_recover();
        let count: i64 = conn
            .query_row(
                "INSERT INTO v2_anon_issued (ip, day, n) VALUES (?1, ?2, 1)
                 ON CONFLICT (ip, day) DO UPDATE SET n = n + 1
                 RETURNING n",
                params![ip, day.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| backend(e, at!()))?;
        prune_before(&conn, day)?;
        Ok(count as u32)
    }

    fn block(&self, target: &Block) -> Result<(), StoreError> {
        let (kind, value) = block_parts(target);
        self.conn
            .lock_recover()
            .execute(
                "INSERT OR IGNORE INTO v2_blocks (kind, value) VALUES (?1, ?2)",
                params![kind, value],
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(())
    }

    fn unblock(&self, target: &Block) -> Result<bool, StoreError> {
        let (kind, value) = block_parts(target);
        let changed = self
            .conn
            .lock_recover()
            .execute(
                "DELETE FROM v2_blocks WHERE kind = ?1 AND value = ?2",
                params![kind, value],
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(changed > 0)
    }

    fn is_blocked(&self, user: &str, ip: &str) -> Result<bool, StoreError> {
        let found: bool = self
            .conn
            .lock_recover()
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM v2_blocks
                 WHERE (kind = 'user' AND value = ?1) OR (kind = 'ip' AND value = ?2))",
                params![user, ip],
                |row| row.get(0),
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(found)
    }

    fn blocks(&self) -> Result<Vec<Block>, StoreError> {
        let conn = self.conn.lock_recover();
        let mut stmt = conn
            .prepare("SELECT kind, value FROM v2_blocks ORDER BY kind, value")
            .map_err(|e| backend(e, at!()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| backend(e, at!()))?;
        rows.map(|row| {
            let (kind, value) = row.map_err(|e| backend(e, at!()))?;
            Ok(if kind == "user" {
                Block::User(value)
            } else {
                Block::Ip(value)
            })
        })
        .collect()
    }

    fn usage_today(&self, day: NaiveDate, top: usize) -> Result<Usage, StoreError> {
        let conn = self.conn.lock_recover();
        Ok(Usage {
            users: top_users_today(&conn, "user_id", day, top)?,
            ips: top_users_today(&conn, "ip", day, top)?,
        })
    }

    fn usage_ranking(&self, order: UsageOrder, top: usize) -> Result<Vec<UsageRow>, StoreError> {
        let conn = self.conn.lock_recover();
        // One query per ordering rather than one clever query: where the spend comes
        // from differs, and the rest — the join to an account, the order, the limit —
        // is the same. A new ordering adds a row here and nothing else.
        let (sql, params): (&str, Vec<Box<dyn rusqlite::ToSql>>) = match order {
            UsageOrder::AllTime => (
                "SELECT totals.user_id, COALESCE(v2_users.email, ''), v2_users.plan, totals.calls \
                 FROM v2_usage_totals totals \
                 LEFT JOIN v2_users ON v2_users.id = totals.user_id \
                 ORDER BY totals.calls DESC, totals.user_id ASC LIMIT ?1",
                vec![Box::new(top as i64)],
            ),
            UsageOrder::Day(day) => (
                "SELECT counts.user_id, COALESCE(v2_users.email, ''), v2_users.plan, \
                        SUM(counts.n) \
                 FROM v2_capture_counts counts \
                 LEFT JOIN v2_users ON v2_users.id = counts.user_id \
                 WHERE counts.day = ?2 \
                 GROUP BY counts.user_id \
                 ORDER BY SUM(counts.n) DESC, counts.user_id ASC LIMIT ?1",
                vec![Box::new(top as i64), Box::new(day.to_string())],
            ),
        };
        let mut stmt = conn.prepare(sql).map_err(|e| backend(e, at!()))?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params.iter().map(|value| value.as_ref())),
                |row| {
                    Ok(UsageRow {
                        user: row.get(0)?,
                        email: row.get(1)?,
                        plan: parse_plan(&row.get::<_, String>(2)?),
                        calls: row.get::<_, i64>(3)? as u32,
                    })
                },
            )
            .map_err(|e| backend(e, at!()))?;
        rows.map(|row| row.map_err(|e| backend(e, at!()))).collect()
    }

    fn is_user_blocked(&self, user: &str) -> Result<bool, StoreError> {
        let found: bool = self
            .conn
            .lock_recover()
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM v2_blocks WHERE kind = 'user' AND value = ?1)",
                params![user],
                |row| row.get(0),
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(found)
    }

    fn account_exists(&self, user: &str) -> Result<bool, StoreError> {
        let found: bool = self
            .conn
            .lock_recover()
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM v2_users WHERE id = ?1)",
                params![user],
                |row| row.get(0),
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(found)
    }

    fn user_id_for_email(&self, email: &str) -> Result<Option<String>, StoreError> {
        self.conn
            .lock_recover()
            .query_row(
                "SELECT id FROM v2_users WHERE email = ?1 AND plan <> 'anon'",
                params![email],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| backend(e, at!()))
    }
    fn user_plan(&self, user: &str) -> Result<super::auth::Plan, StoreError> {
        if user == super::auth::LOCAL_USER {
            return Ok(super::auth::Plan::Free);
        }
        // An id with no account row (an anonymous visitor who has since signed in or
        // expired, whose captures still count today) is anonymous, as in `MemoryStore`.
        let plan: Option<String> = self
            .conn
            .lock_recover()
            .query_row(
                "SELECT plan FROM v2_users WHERE id = ?1",
                params![user],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| backend(e, at!()))?;
        Ok(plan.map_or(super::auth::Plan::Anon, |plan| parse_plan(&plan)))
    }

    fn day_settings(&self, user: &str) -> Result<DaySettings, StoreError> {
        let stored: Option<(Option<String>, Option<String>)> = self
            .conn
            .lock_recover()
            .query_row(
                "SELECT day_deadline, day_placement FROM v2_users WHERE id = ?1",
                params![user],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| backend(e, at!()))?;
        let defaults = DaySettings::default();
        Ok(
            stored.map_or(defaults, |(deadline, placement)| DaySettings {
                deadline: stored_time(deadline, defaults.deadline),
                placement: stored_time(placement, defaults.placement),
            }),
        )
    }

    fn set_day_settings(&self, user: &str, settings: DaySettings) -> Result<(), StoreError> {
        let changed = self
            .conn
            .lock_recover()
            .execute(
                "UPDATE v2_users SET day_deadline = ?1, day_placement = ?2 WHERE id = ?3",
                params![
                    settings.deadline.format(DAY_TIME_FORMAT).to_string(),
                    settings.placement.format(DAY_TIME_FORMAT).to_string(),
                    user
                ],
            )
            .map_err(|e| backend(e, at!()))?;
        if changed == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    fn set_user_plan(&self, user: &str, plan: super::auth::Plan) -> Result<(), StoreError> {
        let changed = self
            .conn
            .lock_recover()
            .execute(
                "UPDATE v2_users SET plan = ?1 WHERE id = ?2",
                params![plan.as_str(), user],
            )
            .map_err(|e| backend(e, at!()))?;
        if changed == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock_recover();
        let tx = conn.transaction().map_err(|e| backend(e, at!()))?;
        let is_anonymous: bool = tx
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM v2_users WHERE id = ?1 AND plan = 'anon')",
                params![anonymous],
                |row| row.get(0),
            )
            .map_err(|e| backend(e, at!()))?;
        if !is_anonymous {
            return Ok(());
        }
        tx.execute(
            "DELETE FROM v2_tasks WHERE user_id = ?1 AND external_uid IS NOT NULL
             AND external_uid IN (
                 SELECT external_uid FROM v2_tasks
                 WHERE user_id = ?2 AND external_uid IS NOT NULL
             )",
            params![anonymous, user],
        )
        .map_err(|e| backend(e, at!()))?;
        tx.execute(
            "UPDATE v2_tasks SET user_id = ?1 WHERE user_id = ?2",
            params![user, anonymous],
        )
        .map_err(|e| backend(e, at!()))?;
        // Like tasks: a series the user already imported keeps the signed-in copy, and
        // the anonymous copy's exceptions go with it.
        const DUPLICATE_SERIES: &str = "SELECT id FROM v2_series
             WHERE user_id = ?1 AND external_uid IS NOT NULL
             AND external_uid IN (
                 SELECT external_uid FROM v2_series
                 WHERE user_id = ?2 AND external_uid IS NOT NULL
             )";
        tx.execute(
            &format!(
                "DELETE FROM v2_series_exceptions WHERE user_id = ?1
                 AND series_id IN ({DUPLICATE_SERIES})"
            ),
            params![anonymous, user],
        )
        .map_err(|e| backend(e, at!()))?;
        tx.execute(
            &format!("DELETE FROM v2_series WHERE id IN ({DUPLICATE_SERIES})"),
            params![anonymous, user],
        )
        .map_err(|e| backend(e, at!()))?;
        tx.execute(
            "UPDATE v2_series SET user_id = ?1 WHERE user_id = ?2",
            params![user, anonymous],
        )
        .map_err(|e| backend(e, at!()))?;
        tx.execute(
            "UPDATE v2_series_exceptions SET user_id = ?1 WHERE user_id = ?2",
            params![user, anonymous],
        )
        .map_err(|e| backend(e, at!()))?;
        // The anonymous id ends here: a copy of it held elsewhere must stop working.
        tx.execute(
            "DELETE FROM v2_users WHERE id = ?1 AND plan = 'anon'",
            params![anonymous],
        )
        .map_err(|e| backend(e, at!()))?;
        tx.commit().map_err(|e| backend(e, at!()))
    }
    fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError> {
        let found: Option<String> = self
            .conn
            .lock_recover()
            .query_row(
                "SELECT id FROM v2_users WHERE id = ?1 AND plan = 'anon'",
                params![user],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| backend(e, at!()))?;
        Ok(found.is_some())
    }

    fn ensure_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
        self.conn
            .lock_recover()
            .execute(
                "INSERT OR IGNORE INTO v2_users \
                     (id, email, name, picture, created_at, plan, last_activity_at) \
                 VALUES (?1, '', '', NULL, ?2, 'anon', ?2)",
                params![user, format_ts(now)],
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(())
    }

    fn touch_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
        self.conn
            .lock_recover()
            .execute(
                "UPDATE v2_users SET last_activity_at = ?2 \
                 WHERE id = ?1 AND plan = 'anon' AND last_activity_at < ?3",
                params![user, format_ts(now), format_ts(now - ACTIVITY_THROTTLE)],
            )
            .map_err(|e| backend(e, at!()))?;
        Ok(())
    }

    fn expire_anonymous(&self, cutoff: DateTime<Utc>) -> Result<usize, StoreError> {
        let mut conn = self.conn.lock_recover();
        let tx = conn.transaction().map_err(|e| backend(e, at!()))?;
        let cutoff = format_ts(cutoff);
        // The id, not the plan, says who is anonymous: an admin can put a signed-in
        // account on the anon plan, and that must never delete it.
        let stale = "SELECT id FROM v2_users WHERE plan = 'anon' AND id LIKE 'anon\\_%' ESCAPE '\\'
             AND last_activity_at <= ?1";
        // The series' own rows go first, for the same reason the tasks do: nothing
        // cascades here, so leaving them would keep a dead visitor's routines
        // forever with nobody left who could see or delete them.
        tx.execute(
            &format!("DELETE FROM v2_series_exceptions WHERE user_id IN ({stale})"),
            params![cutoff],
        )
        .map_err(|e| backend(e, at!()))?;
        tx.execute(
            &format!("DELETE FROM v2_series WHERE user_id IN ({stale})"),
            params![cutoff],
        )
        .map_err(|e| backend(e, at!()))?;
        tx.execute(
            &format!("DELETE FROM v2_tasks WHERE user_id IN ({stale})"),
            params![cutoff],
        )
        .map_err(|e| backend(e, at!()))?;
        let users = tx
            .execute(
                &format!("DELETE FROM v2_users WHERE id IN ({stale})"),
                params![cutoff],
            )
            .map_err(|e| backend(e, at!()))?;
        tx.commit().map_err(|e| backend(e, at!()))?;
        Ok(users)
    }
}

/// Drop per-day counts older than the retention window.
fn prune_before(conn: &Connection, today: NaiveDate) -> Result<(), StoreError> {
    let cutoff = (today - chrono::Duration::days(USAGE_RETENTION_DAYS)).to_string();
    for table in ["v2_capture_counts", "v2_anon_issued"] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE day < ?1"),
            params![cutoff],
        )
        .map_err(|e| backend(e, at!()))?;
    }
    Ok(())
}

fn block_parts(target: &Block) -> (&'static str, &str) {
    match target {
        Block::User(user) => ("user", user),
        Block::Ip(ip) => ("ip", ip),
    }
}

/// The `top` busiest values of `column` today, most captures first. `column` is one of
/// two fixed names chosen in code, never user input.
fn top_users_today(
    conn: &Connection,
    column: &str,
    day: NaiveDate,
    top: usize,
) -> Result<Vec<(String, u32)>, StoreError> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {column}, SUM(n) AS total FROM v2_capture_counts WHERE day = ?1
             GROUP BY {column} ORDER BY total DESC, {column} LIMIT ?2"
        ))
        .map_err(|e| backend(e, at!()))?;
    let rows = stmt
        .query_map(params![day.to_string(), top as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u32))
        })
        .map_err(|e| backend(e, at!()))?;
    rows.map(|row| row.map_err(|e| backend(e, at!()))).collect()
}

/// The `top` most frequent values, most frequent first, ties by name. The memory
/// store's version of [`top_users_today`].
#[cfg(test)]
fn top_counts<'a>(values: impl Iterator<Item = &'a str>, top: usize) -> Vec<(String, u32)> {
    let mut counts = std::collections::BTreeMap::<&str, u32>::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    let mut ranked: Vec<(String, u32)> = counts
        .into_iter()
        .map(|(value, count)| (value.to_owned(), count))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(top);
    ranked
}

/// Read one `TASK_COLUMNS` row into a [`Task`].
///
/// The column indexes are positional, so they follow `TASK_COLUMNS`. A test
/// checks that list against the real table, which catches a column being added
/// in one place and not the other.
fn read_task(row: &rusqlite::Row<'_>) -> Result<Task, rusqlite::Error> {
    let when_kind: String = row.get(3)?;
    let when_time: Option<String> = row.get(4)?;
    let done: i64 = row.get(5)?;
    let label: String = row.get(7)?;
    let priority: String = row.get(8)?;
    let source: String = row.get(9)?;
    let created: String = row.get(10)?;
    let updated_at: String = row.get(12)?;
    let due_tz: Option<String> = row.get(13)?;
    let off_planner: i64 = row.get(14)?;

    Ok(Task {
        id: TaskId(row.get(0)?),
        title: row.get(1)?,
        duration_min: row.get::<_, i64>(2)? as u16,
        when: when_from_parts(&when_kind, when_time.as_deref()),
        done: done != 0,
        order: row.get(6)?,
        label: label_from_stored(&label),
        priority: priority_from_stored(&priority),
        source: source_from_stored(&source),
        // A hand-edited timestamp reads as a fixed instant, never the wall clock: a
        // version that moved on every read would refuse every edit as stale.
        created: parse_ts(&created).unwrap_or(DateTime::UNIX_EPOCH),
        updated_at: parse_ts(&updated_at).unwrap_or(DateTime::UNIX_EPOCH),
        due_tz,
        external_uid: row.get(11)?,
        off_planner: off_planner != 0,
    })
}

fn backend(error: rusqlite::Error, at: &'static str) -> StoreError {
    let code = match &error {
        rusqlite::Error::SqliteFailure(ffi, _) => Some(ffi.extended_code),
        _ => None,
    };
    StoreError::Backend(Failure {
        at,
        code,
        message: error.to_string(),
    })
}

// ---- series (ARCHITECTURE.md 7.1) ----

fn read_series(row: &rusqlite::Row<'_>) -> Result<Series, rusqlite::Error> {
    let weekdays: String = row.get(3)?;
    let interval: i64 = row.get(4)?;
    let until: Option<String> = row.get(5)?;
    let start_day: String = row.get(6)?;
    let time_of_day: Option<String> = row.get(7)?;
    let created = parse_ts(&row.get::<_, String>(10)?).unwrap_or(DateTime::UNIX_EPOCH);
    Ok(Series {
        id: SeriesId(row.get(0)?),
        title: row.get(1)?,
        duration_min: row.get::<_, i64>(2)? as u16,
        rule: todo_core::Recurrence {
            weekdays: weekdays_from_stored(&weekdays),
            interval: u32::try_from(interval).unwrap_or(1),
            until: until.and_then(|day| NaiveDate::parse_from_str(&day, "%Y-%m-%d").ok()),
        },
        // An unreadable start day falls back to when the series was made, not today:
        // "today" would move the series' start every day it is read.
        start_day: NaiveDate::parse_from_str(&start_day, "%Y-%m-%d")
            .unwrap_or_else(|_| created.date_naive()),
        time_of_day: time_of_day.and_then(|text| parse_time_of_day(&text)),
        zone: row.get(8)?,
        external_uid: row.get(9)?,
        created,
        order: row.get(11)?,
    })
}

/// Weekdays as the digits Monday=1..Sunday=7 (`"124"` = Mon/Tue/Thu).
fn weekdays_to_stored(weekdays: &[chrono::Weekday]) -> String {
    weekdays
        .iter()
        .map(|day| {
            char::from(b'0' + u8::try_from(day.number_from_monday()).expect("Monday=1..Sunday=7"))
        })
        .collect()
}

/// Digits back to weekdays. Unknown digits (a hand-edit) are skipped, never a
/// load failure.
fn weekdays_from_stored(text: &str) -> Vec<chrono::Weekday> {
    const DAYS: [chrono::Weekday; 7] = [
        chrono::Weekday::Mon,
        chrono::Weekday::Tue,
        chrono::Weekday::Wed,
        chrono::Weekday::Thu,
        chrono::Weekday::Fri,
        chrono::Weekday::Sat,
        chrono::Weekday::Sun,
    ];
    text.chars()
        .filter_map(|digit| digit.to_digit(10))
        .filter(|n| (1..=7).contains(n))
        .filter_map(|n| DAYS.get(usize::try_from(n - 1).ok()?).copied())
        .collect()
}

/// `HH:MM`, the wall-clock time every occurrence of a series shares.
fn format_time_of_day(time: NaiveTime) -> String {
    time.format("%H:%M").to_string()
}

fn parse_time_of_day(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text, "%H:%M").ok()
}

/// The columns that hold one exception.
fn exception_to_parts(kind: &ExceptionKind) -> (&'static str, Option<String>, Option<String>) {
    match kind {
        ExceptionKind::Done => ("done", None, None),
        ExceptionKind::Skipped => ("skipped", None, None),
        ExceptionKind::Moved { to, to_tz } => {
            ("moved", serde_json::to_string(to).ok(), to_tz.clone())
        }
    }
}

/// `None` for a kind that will not parse: read as "no exception" rather than
/// failing the load, the same way `when_from_parts` degrades.
fn exception_from_parts(
    kind: &str,
    moved_when: Option<String>,
    moved_tz: Option<String>,
) -> Option<ExceptionKind> {
    match kind {
        "done" => Some(ExceptionKind::Done),
        "skipped" => Some(ExceptionKind::Skipped),
        "moved" => moved_when
            .and_then(|json| serde_json::from_str(&json).ok())
            .map(|to| ExceptionKind::Moved {
                to,
                to_tz: moved_tz,
            }),
        _ => None,
    }
}

/// `None` for a row whose date will not parse: an exception is about one day, and
/// guessing the day (today, say) would mark or skip a day the user never touched.
fn read_exception(row: &rusqlite::Row<'_>) -> Result<Option<SeriesException>, rusqlite::Error> {
    let date: String = row.get(0)?;
    let kind: String = row.get(1)?;
    let moved_when: Option<String> = row.get(2)?;
    let moved_tz: Option<String> = row.get(3)?;
    let Ok(date) = NaiveDate::parse_from_str(&date, "%Y-%m-%d") else {
        return Ok(None);
    };
    Ok(Some(SeriesException {
        date,
        // A moved row whose `When` will not parse degrades to skipped: the day
        // stays out of the way instead of failing the load.
        kind: exception_from_parts(&kind, moved_when, moved_tz).unwrap_or(ExceptionKind::Skipped),
    }))
}

/// A uniqueness violation is a conflict the caller can act on; anything else
/// is the database misbehaving.
fn conflict(error: rusqlite::Error) -> StoreError {
    match error {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::ConstraintViolation,
                ..
            },
            message,
        ) => StoreError::Conflict(message.unwrap_or_else(|| "constraint violated".into())),
        other => backend(other, at!()),
    }
}

/// SQLite has no date type; instants are stored as RFC 3339 text.
fn format_ts(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339()
}

fn parse_ts(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// `When` as the two columns that hold it.
fn when_to_parts(when: When) -> (&'static str, Option<DateTime<Utc>>) {
    match when {
        When::Anytime => ("anytime", None),
        When::DueBy(deadline) => ("due_by", Some(deadline)),
        When::At(start) => ("at", Some(start)),
    }
}

fn when_from_parts(kind: &str, time: Option<&str>) -> When {
    let instant = time.and_then(parse_ts);
    match (kind, instant) {
        ("due_by", Some(deadline)) => When::DueBy(deadline),
        ("at", Some(start)) => When::At(start),
        // An unknown kind, or a kind whose instant will not parse, is read as
        // "anytime": a hand-edited row must not stop the server starting, and
        // an unscheduled task is a safe thing to show.
        _ => When::Anytime,
    }
}

fn label_to_stored(label: Label) -> &'static str {
    match label {
        Label::School => "school",
        Label::Work => "work",
        Label::Errands => "errands",
        Label::Health => "health",
        Label::Personal => "personal",
        Label::Other => "other",
    }
}

fn label_from_stored(text: &str) -> Label {
    match text {
        "school" => Label::School,
        "work" => Label::Work,
        "errands" => Label::Errands,
        "health" => Label::Health,
        "personal" => Label::Personal,
        _ => Label::Other,
    }
}

fn priority_to_stored(priority: Priority) -> &'static str {
    match priority {
        Priority::Low => "low",
        Priority::Medium => "medium",
        Priority::High => "high",
        Priority::Critical => "critical",
    }
}

fn priority_from_stored(text: &str) -> Priority {
    match text {
        "low" => Priority::Low,
        "high" => Priority::High,
        "critical" => Priority::Critical,
        // "medium", and anything a hand-edit invented, is Medium.
        _ => Priority::Medium,
    }
}

fn source_to_stored(source: Source) -> &'static str {
    match source {
        Source::Voice => "voice",
        Source::Pdf => "pdf",
        Source::Email => "email",
        Source::Text => "text",
        Source::Calendar => "calendar",
        Source::Series => "series",
    }
}

fn source_from_stored(text: &str) -> Source {
    match text {
        "voice" => Source::Voice,
        "pdf" => Source::Pdf,
        "email" => Source::Email,
        "calendar" => Source::Calendar,
        "series" => Source::Series,
        _ => Source::Text,
    }
}

#[cfg(test)]
mod tests;

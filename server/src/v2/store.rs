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
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use todo_core::{
    ExceptionKind, Label, Priority, Series, SeriesException, SeriesId, Source, Task, TaskId, When,
};

fn parse_plan(value: &str) -> super::auth::Plan {
    match value {
        "anon" => super::auth::Plan::Anon,
        "paid" => super::auth::Plan::Paid,
        _ => super::auth::Plan::Free,
    }
}

/// An anonymous user's `last_activity_at` is refreshed at most this often.
const ACTIVITY_THROTTLE: Duration = Duration::hours(1);

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
const SCHEMA_VERSION: i64 = 10;

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
                            label, priority, source, created, external_uid, updated_at, due_tz";

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
    Backend(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::NotFound => write!(f, "no such task"),
            StoreError::Conflict(why) => write!(f, "conflict: {why}"),
            StoreError::Backend(why) => write!(f, "storage failed: {why}"),
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

    /// Create or refresh the user and open a session, in one transaction.
    fn upsert_user_and_session(
        &self,
        user: &super::auth::Me,
        token: &str,
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

/// Tasks held in memory, with their owner. The HTTP tests use this so they never
/// touch a disk, and it has to model ownership exactly as the SQL store does or the
/// tests would not be testing the scoping that matters.
#[derive(Default)]
#[cfg(test)]
pub struct MemoryStore {
    tasks: Mutex<Vec<(String, Task)>>,
    series: Mutex<Vec<(String, Series)>>,
    exceptions: Mutex<Vec<(String, SeriesId, SeriesException)>>,
    users: Mutex<Vec<super::auth::Me>>,
    sessions: Mutex<Vec<(String, String, chrono::DateTime<Utc>)>>,
    captures: Mutex<Vec<(String, String, NaiveDate)>>,
    /// Every call each account has ever made, which the per-day captures cannot answer
    /// for once they are pruned away.
    usage_totals: Mutex<std::collections::HashMap<String, u32>>,
    anon_issued: Mutex<Vec<(String, NaiveDate)>>,
    blocks: Mutex<Vec<Block>>,
    /// When each anonymous user was last seen.
    activity: Mutex<std::collections::HashMap<String, DateTime<Utc>>>,
}

#[cfg(test)]
impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Forwards to whichever store was chosen. This is how `App` stays generic
/// over "SQLite or memory" without naming either.
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
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        (**self).upsert_user_and_session(user, token, expires_at)
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

#[cfg(test)]
impl Store for MemoryStore {
    fn tasks(&self, user: &str) -> Result<Vec<Task>, StoreError> {
        let mut stored: Vec<Task> = self
            .tasks
            .lock_recover()
            .iter()
            .filter(|(owner, _)| owner == user)
            .map(|(_, task)| task.clone())
            .collect();
        stored.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then_with(|| a.created.cmp(&b.created))
                .then_with(|| a.id.0.cmp(&b.id.0))
        });
        Ok(stored)
    }

    fn put(&self, user: &str, task: Task) -> Result<(), StoreError> {
        let mut stored = self.tasks.lock_recover();
        if stored
            .iter()
            .any(|(owner, other)| owner != user && other.id == task.id)
        {
            return Err(StoreError::Conflict(format!(
                "task id {} belongs to another user",
                task.id.0
            )));
        }
        if let Some(uid) = task.external_uid.as_deref()
            && let Some(clash) = stored
                .iter()
                .filter(|(owner, _)| owner == user)
                .map(|(_, other)| other)
                .find(|other| other.id != task.id && other.external_uid.as_deref() == Some(uid))
        {
            return Err(StoreError::Conflict(format!(
                "{} and {} both claim uid {uid}",
                task.id.0, clash.id.0
            )));
        }
        match stored
            .iter_mut()
            .find(|(owner, other)| owner == user && other.id == task.id)
        {
            Some((_, existing)) => *existing = task,
            None => stored.push((user.to_owned(), task)),
        }
        Ok(())
    }

    fn user_plan(&self, user: &str) -> Result<super::auth::Plan, StoreError> {
        if user == super::auth::LOCAL_USER {
            return Ok(super::auth::Plan::Free);
        }
        Ok(self
            .users
            .lock_recover()
            .iter()
            .find(|held| held.id == user)
            .map(|held| held.plan)
            .unwrap_or(super::auth::Plan::Anon))
    }

    fn set_user_plan(&self, user: &str, plan: super::auth::Plan) -> Result<(), StoreError> {
        let mut users = self.users.lock_recover();
        let Some(found) = users.iter_mut().find(|held| held.id == user) else {
            return Err(StoreError::NotFound);
        };
        found.plan = plan;
        Ok(())
    }

    fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError> {
        Ok(self
            .users
            .lock_recover()
            .iter()
            .any(|held| held.id == user && held.plan == super::auth::Plan::Anon))
    }

    fn ensure_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
        let mut users = self.users.lock_recover();
        if !users.iter().any(|held| held.id == user) {
            self.activity.lock_recover().insert(user.to_owned(), now);
            users.push(super::auth::Me {
                id: user.to_owned(),
                email: String::new(),
                name: String::new(),
                picture: None,
                plan: super::auth::Plan::Anon,
            });
        }
        Ok(())
    }

    fn touch_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
        if let Some(seen) = self.activity.lock_recover().get_mut(user)
            && *seen < now - ACTIVITY_THROTTLE
        {
            *seen = now;
        }
        Ok(())
    }

    fn expire_anonymous(&self, cutoff: DateTime<Utc>) -> Result<usize, StoreError> {
        let mut activity = self.activity.lock_recover();
        let gone: Vec<String> = activity
            .iter()
            .filter(|(_, seen)| **seen <= cutoff)
            .map(|(user, _)| user.clone())
            .collect();
        for user in &gone {
            activity.remove(user);
        }
        self.tasks
            .lock_recover()
            .retain(|(owner, _)| !gone.contains(owner));
        // The series go with them: nothing cascades here either, so leaving them would
        // keep a dead visitor's routines with nobody left who could see them.
        self.series
            .lock_recover()
            .retain(|(owner, _)| !gone.contains(owner));
        self.exceptions
            .lock_recover()
            .retain(|(owner, _, _)| !gone.contains(owner));
        self.users
            .lock_recover()
            .retain(|held| !gone.contains(&held.id));
        Ok(gone.len())
    }

    fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError> {
        let mut stored = self.tasks.lock_recover();
        let before = stored.len();
        stored.retain(|(owner, task)| !(owner == user && &task.id == id));
        Ok(stored.len() != before)
    }

    fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError> {
        let mut stored = self.tasks.lock_recover();
        for (position, id) in ids.iter().enumerate() {
            if let Some((_, task)) = stored
                .iter_mut()
                .find(|(owner, task)| owner == user && &task.id == id)
            {
                task.order = position as i64;
            }
        }
        Ok(())
    }

    fn find_by_uid(&self, user: &str, uid: &str) -> Result<Option<Task>, StoreError> {
        Ok(self
            .tasks
            .lock_recover()
            .iter()
            .find(|(owner, task)| owner == user && task.external_uid.as_deref() == Some(uid))
            .map(|(_, task)| task.clone()))
    }

    fn series(&self, user: &str) -> Result<Vec<Series>, StoreError> {
        let mut held: Vec<Series> = self
            .series
            .lock_recover()
            .iter()
            .filter(|(owner, _)| owner == user)
            .map(|(_, series)| series.clone())
            .collect();
        held.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then_with(|| a.created.cmp(&b.created))
                .then_with(|| a.id.0.cmp(&b.id.0))
        });
        Ok(held)
    }

    fn put_series(&self, user: &str, series: Series) -> Result<(), StoreError> {
        let mut held = self.series.lock_recover();
        if held
            .iter()
            .any(|(owner, other)| owner != user && other.id == series.id)
        {
            return Err(StoreError::Conflict(format!(
                "series id {} belongs to another user",
                series.id.0
            )));
        }
        match held
            .iter_mut()
            .find(|(owner, other)| owner == user && other.id == series.id)
        {
            Some((_, existing)) => *existing = series,
            None => held.push((user.to_owned(), series)),
        }
        Ok(())
    }

    fn delete_series(&self, user: &str, id: &SeriesId) -> Result<bool, StoreError> {
        let mut held = self.series.lock_recover();
        let before = held.len();
        held.retain(|(owner, series)| !(owner == user && &series.id == id));
        let removed = held.len() != before;
        if removed {
            self.exceptions
                .lock_recover()
                .retain(|(owner, series, _)| !(owner == user && series == id));
        }
        Ok(removed)
    }

    fn exceptions(&self, user: &str, id: &SeriesId) -> Result<Vec<SeriesException>, StoreError> {
        let mut held: Vec<SeriesException> = self
            .exceptions
            .lock_recover()
            .iter()
            .filter(|(owner, series, _)| owner == user && series == id)
            .map(|(_, _, exception)| exception.clone())
            .collect();
        held.sort_by_key(|exception| exception.date);
        Ok(held)
    }

    fn put_exception(
        &self,
        user: &str,
        id: &SeriesId,
        exception: SeriesException,
    ) -> Result<(), StoreError> {
        let mut held = self.exceptions.lock_recover();
        match held.iter_mut().find(|(owner, series, held)| {
            owner == user && series == id && held.date == exception.date
        }) {
            Some((_, _, existing)) => *existing = exception,
            None => held.push((user.to_owned(), id.clone(), exception)),
        }
        Ok(())
    }

    fn delete_exception(
        &self,
        user: &str,
        id: &SeriesId,
        date: NaiveDate,
    ) -> Result<bool, StoreError> {
        let mut held = self.exceptions.lock_recover();
        let before = held.len();
        held.retain(|(owner, series, exception)| {
            !(owner == user && series == id && exception.date == date)
        });
        Ok(held.len() != before)
    }

    fn upsert_user_and_session(
        &self,
        user: &super::auth::Me,
        token: &str,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut users = self.users.lock_recover();
        match users.iter_mut().find(|held| held.id == user.id) {
            // A returning sign-in keeps the plan the database already holds.
            Some(existing) => {
                existing.email.clone_from(&user.email);
                existing.name.clone_from(&user.name);
                existing.picture.clone_from(&user.picture);
            }
            None => users.push(user.clone()),
        }
        drop(users);
        self.sessions
            .lock_recover()
            .push((token.to_owned(), user.id.clone(), expires_at));
        Ok(())
    }

    fn delete_session(&self, token: &str) -> Result<(), StoreError> {
        self.sessions
            .lock_recover()
            .retain(|(held, _, _)| held != token);
        Ok(())
    }

    fn session_user(
        &self,
        token: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<super::auth::Me>, StoreError> {
        let sessions = self.sessions.lock_recover();
        let Some((_, user_id, _expires_at)) = sessions
            .iter()
            .find(|(held, _, expires_at)| held == token && *expires_at > now)
        else {
            return Ok(None);
        };
        let user_id = user_id.clone();
        drop(sessions);
        Ok(self
            .users
            .lock_recover()
            .iter()
            .find(|user| user.id == user_id)
            .cloned())
    }

    fn delete_user(&self, user: &str) -> Result<(), StoreError> {
        self.tasks.lock_recover().retain(|(owner, _)| owner != user);
        self.series
            .lock_recover()
            .retain(|(owner, _)| owner != user);
        self.exceptions
            .lock_recover()
            .retain(|(owner, _, _)| owner != user);
        self.users.lock_recover().retain(|held| held.id != user);
        self.sessions
            .lock_recover()
            .retain(|(_, owner, _)| owner != user);
        Ok(())
    }

    fn captures_today(&self, user: &str, day: chrono::NaiveDate) -> Result<u32, StoreError> {
        Ok(self
            .captures
            .lock_recover()
            .iter()
            .filter(|(owner, _, when)| owner == user && *when == day)
            .count() as u32)
    }

    fn record_capture(&self, user: &str, ip: &str, day: NaiveDate) -> Result<(), StoreError> {
        self.captures
            .lock_recover()
            .push((user.to_owned(), ip.to_owned(), day));
        // The all-time total, which the per-day counts cannot answer for once they
        // are pruned away.
        *self
            .usage_totals
            .lock_recover()
            .entry(user.to_owned())
            .or_default() += 1;
        Ok(())
    }

    fn ip_captures_today(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError> {
        Ok(self
            .captures
            .lock_recover()
            .iter()
            .filter(|(_, from, when)| from == ip && *when == day)
            .count() as u32)
    }

    fn record_anon_issued(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError> {
        let mut issued = self.anon_issued.lock_recover();
        issued.push((ip.to_owned(), day));
        Ok(issued
            .iter()
            .filter(|(from, when)| from == ip && *when == day)
            .count() as u32)
    }

    fn block(&self, target: &Block) -> Result<(), StoreError> {
        let mut blocks = self.blocks.lock_recover();
        if !blocks.contains(target) {
            blocks.push(target.clone());
        }
        Ok(())
    }

    fn unblock(&self, target: &Block) -> Result<bool, StoreError> {
        let mut blocks = self.blocks.lock_recover();
        let before = blocks.len();
        blocks.retain(|held| held != target);
        Ok(blocks.len() != before)
    }

    fn is_blocked(&self, user: &str, ip: &str) -> Result<bool, StoreError> {
        Ok(self
            .blocks
            .lock_recover()
            .iter()
            .any(|held| *held == Block::User(user.into()) || *held == Block::Ip(ip.into())))
    }

    fn blocks(&self) -> Result<Vec<Block>, StoreError> {
        Ok(self.blocks.lock_recover().clone())
    }

    fn usage_today(&self, day: NaiveDate, top: usize) -> Result<Usage, StoreError> {
        let captures = self.captures.lock_recover();
        let today = captures.iter().filter(|(_, _, when)| *when == day);
        Ok(Usage {
            users: top_counts(today.clone().map(|(user, _, _)| user.as_str()), top),
            ips: top_counts(today.map(|(_, ip, _)| ip.as_str()), top),
        })
    }

    fn usage_ranking(&self, order: UsageOrder, top: usize) -> Result<Vec<UsageRow>, StoreError> {
        let mut calls: Vec<(String, u32)> = match order {
            UsageOrder::AllTime => self
                .usage_totals
                .lock_recover()
                .iter()
                .map(|(user, calls)| (user.clone(), *calls))
                .collect(),
            UsageOrder::Day(day) => {
                let captures = self.captures.lock_recover();
                let mut counted: std::collections::HashMap<&str, u32> =
                    std::collections::HashMap::new();
                for (user, _, _when) in captures.iter().filter(|(_, _, _when)| *_when == day) {
                    *counted.entry(user.as_str()).or_default() += 1;
                }
                counted
                    .into_iter()
                    .map(|(user, calls)| (user.to_owned(), calls))
                    .collect()
            }
        };
        // Most spend first, then by id, so equal spend has one order on every store.
        calls.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        calls.truncate(top);
        let users = self.users.lock_recover();
        calls
            .into_iter()
            .map(|(user, calls)| {
                let account = users.iter().find(|held| held.id == user);
                Ok(UsageRow {
                    user,
                    // An account with no email is an anonymous visitor, and says so.
                    email: account.map(|held| held.email.clone()).unwrap_or_default(),
                    plan: account.map_or(super::auth::Plan::Anon, |held| held.plan),
                    calls,
                })
            })
            .collect()
    }

    fn is_user_blocked(&self, user: &str) -> Result<bool, StoreError> {
        Ok(self
            .blocks
            .lock_recover()
            .contains(&Block::User(user.to_owned())))
    }

    fn account_exists(&self, user: &str) -> Result<bool, StoreError> {
        Ok(self.users.lock_recover().iter().any(|held| held.id == user))
    }

    fn user_id_for_email(&self, email: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .users
            .lock_recover()
            .iter()
            .find(|held| held.plan != super::auth::Plan::Anon && held.email == email)
            .map(|held| held.id.clone()))
    }

    fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError> {
        if !self.anonymous_exists(anonymous)? {
            return Ok(());
        }
        self.users
            .lock_recover()
            .retain(|held| held.id != anonymous);
        let mut stored = self.tasks.lock_recover();
        let existing_uids: std::collections::HashSet<_> = stored
            .iter()
            .filter(|(owner, _)| owner == user)
            .filter_map(|(_, task)| task.external_uid.clone())
            .collect();
        let mut moved = Vec::new();
        stored.retain(|(owner, task)| {
            if owner != anonymous {
                return true;
            }
            if task
                .external_uid
                .as_deref()
                .is_some_and(|uid| existing_uids.contains(uid))
            {
                false
            } else {
                moved.push(task.clone());
                false
            }
        });
        stored.extend(moved.into_iter().map(|task| (user.to_owned(), task)));
        drop(stored);
        let mut series = self.series.lock_recover();
        let moved_series: Vec<Series> = series
            .iter()
            .filter(|(owner, _)| owner == anonymous)
            .map(|(_, held)| held.clone())
            .collect();
        series.retain(|(owner, _)| owner != anonymous);
        drop(series);
        let mut exceptions = self.exceptions.lock_recover();
        let moved_exceptions: Vec<(SeriesId, SeriesException)> = exceptions
            .iter()
            .filter(|(owner, _, _)| owner == anonymous)
            .map(|(_, series, exception)| (series.clone(), exception.clone()))
            .collect();
        exceptions.retain(|(owner, _, _)| owner != anonymous);
        drop(exceptions);
        for held in moved_series {
            self.put_series(user, held)?;
        }
        for (id, exception) in moved_exceptions {
            self.put_exception(user, &id, exception)?;
        }
        Ok(())
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
                StoreError::Backend(format!("could not create {}: {e}", parent.display()))
            })?;
        }
        let conn = Connection::open(path).map_err(|e| StoreError::Backend(e.to_string()))?;
        // The admin console opens a second handle on the same file. Wait for a write
        // lock rather than failing a request straight away.
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| StoreError::Backend(e.to_string()))?;
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
            conn: Mutex::new(
                Connection::open_in_memory().map_err(|e| StoreError::Backend(e.to_string()))?,
            ),
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
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        // Only what this file has not seen: a migration is not safe to run twice (an
        // `ADD COLUMN` fails the second time), and the file already holds the first
        // `version` of them.
        for statement in MIGRATIONS.iter().skip(version as usize) {
            tx.execute_batch(statement)
                .map_err(|e| StoreError::Backend(e.to_string()))?;
        }
        if version < 10 {
            hash_plain_sessions(&tx).map_err(|e| StoreError::Backend(e.to_string()))?;
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        tx.commit().map_err(|e| StoreError::Backend(e.to_string()))
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
            .map_err(backend)?;
        let rows = stmt.query_map(params![user], read_task).map_err(backend)?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row.map_err(backend)?);
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
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
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
                     due_tz = excluded.due_tz \
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
            .map_err(backend)?;
        Ok(removed > 0)
    }

    fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError> {
        let mut conn = self.conn.lock_recover();
        let tx = conn.transaction().map_err(backend)?;
        {
            let mut stmt = tx
                .prepare("UPDATE v2_tasks SET \"order\" = ?3 WHERE id = ?1 AND user_id = ?2")
                .map_err(backend)?;
            for (position, id) in ids.iter().enumerate() {
                stmt.execute(params![id.0, user, position as i64])
                    .map_err(backend)?;
            }
        }
        tx.commit().map_err(backend)
    }

    fn find_by_uid(&self, user: &str, uid: &str) -> Result<Option<Task>, StoreError> {
        let conn = self.conn.lock_recover();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM v2_tasks WHERE external_uid = ?1 AND user_id = ?2"
            ))
            .map_err(backend)?;
        let mut rows = stmt.query(params![uid, user]).map_err(backend)?;
        let Some(row) = rows.next().map_err(backend)? else {
            return Ok(None);
        };
        Ok(Some(read_task(row).map_err(backend)?))
    }

    fn series(&self, user: &str) -> Result<Vec<Series>, StoreError> {
        let conn = self.conn.lock_recover();
        let mut stmt = conn
            .prepare(
                "SELECT id, title, duration_min, weekdays, until, start_day, time_of_day, zone, \
                 external_uid, created, \"order\" FROM v2_series WHERE user_id = ?1 \
                 ORDER BY \"order\" ASC, created ASC, id ASC",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![user], read_series)
            .map_err(backend)?;
        let mut held = Vec::new();
        for row in rows {
            held.push(row.map_err(backend)?);
        }
        Ok(held)
    }

    fn put_series(&self, user: &str, series: Series) -> Result<(), StoreError> {
        let conn = self.conn.lock_recover();
        // Same ownership rule as `put`: an id held by someone else is refused,
        // never taken over.
        let written = conn
            .execute(
                "INSERT INTO v2_series (id, user_id, title, duration_min, weekdays, until, \
                 start_day, time_of_day, zone, external_uid, created, \"order\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
                 ON CONFLICT(id) DO UPDATE SET \
                     title = excluded.title, \
                     duration_min = excluded.duration_min, \
                     weekdays = excluded.weekdays, \
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
        let tx = conn.transaction().map_err(backend)?;
        let removed = tx
            .execute(
                "DELETE FROM v2_series WHERE id = ?1 AND user_id = ?2",
                params![id.0, user],
            )
            .map_err(backend)?;
        tx.execute(
            "DELETE FROM v2_series_exceptions WHERE series_id = ?1 AND user_id = ?2",
            params![id.0, user],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)?;
        Ok(removed > 0)
    }

    fn exceptions(&self, user: &str, id: &SeriesId) -> Result<Vec<SeriesException>, StoreError> {
        let conn = self.conn.lock_recover();
        let mut stmt = conn
            .prepare(
                "SELECT date, kind, moved_when, moved_tz FROM v2_series_exceptions \
                 WHERE user_id = ?1 AND series_id = ?2 ORDER BY date ASC",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![user, id.0], read_exception)
            .map_err(backend)?;
        let mut held = Vec::new();
        for row in rows {
            held.push(row.map_err(backend)?);
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
            .map_err(backend)?;
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
            .map_err(backend)?;
        Ok(removed > 0)
    }

    fn upsert_user_and_session(
        &self,
        user: &super::auth::Me,
        token: &str,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock_recover();
        let tx = conn.transaction().map_err(backend)?;
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
                format_ts(Utc::now()),
                user.plan.as_str()
            ],
        )
        .map_err(backend)?;
        tx.execute(
            "INSERT INTO v2_sessions (token, user_id, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![
                hash_token(token),
                user.id,
                format_ts(expires_at),
                format_ts(Utc::now())
            ],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)
    }

    fn delete_session(&self, token: &str) -> Result<(), StoreError> {
        self.conn
            .lock_recover()
            .execute(
                "DELETE FROM v2_sessions WHERE token = ?1",
                params![hash_token(token)],
            )
            .map_err(backend)?;
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
            .map_err(backend)?;
        let mut rows = stmt
            .query(params![hash_token(token), format_ts(now)])
            .map_err(backend)?;
        let Some(row) = rows.next().map_err(backend)? else {
            return Ok(None);
        };
        Ok(Some(super::auth::Me {
            id: row.get(0).map_err(backend)?,
            email: row.get(1).map_err(backend)?,
            name: row.get(2).map_err(backend)?,
            picture: row.get(3).map_err(backend)?,
            plan: parse_plan(&row.get::<_, String>(4).map_err(backend)?),
        }))
    }

    fn delete_user(&self, user: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock_recover();
        let tx = conn.transaction().map_err(backend)?;
        // Tasks go first: SQLite cannot cascade here because user_id is a column
        // added by migration with a default, not a declared constraint.
        tx.execute("DELETE FROM v2_tasks WHERE user_id = ?1", params![user])
            .map_err(backend)?;
        tx.execute("DELETE FROM v2_series WHERE user_id = ?1", params![user])
            .map_err(backend)?;
        tx.execute(
            "DELETE FROM v2_series_exceptions WHERE user_id = ?1",
            params![user],
        )
        .map_err(backend)?;
        tx.execute("DELETE FROM v2_sessions WHERE user_id = ?1", params![user])
            .map_err(backend)?;
        tx.execute("DELETE FROM v2_users WHERE id = ?1", params![user])
            .map_err(backend)?;
        tx.commit().map_err(backend)
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
            .map_err(backend)?;
        Ok(count as u32)
    }

    fn record_capture(&self, user: &str, ip: &str, day: NaiveDate) -> Result<(), StoreError> {
        let conn = self.conn.lock_recover();
        conn.execute(
            "INSERT INTO v2_capture_counts (user_id, ip, day, n) VALUES (?1, ?2, ?3, 1)
             ON CONFLICT (user_id, ip, day) DO UPDATE SET n = n + 1",
            params![user, ip, day.to_string()],
        )
        .map_err(backend)?;
        // The all-time total, which the per-day counts cannot answer for once they are
        // pruned.
        conn.execute(
            "INSERT INTO v2_usage_totals (user_id, calls) VALUES (?1, 1)
             ON CONFLICT (user_id) DO UPDATE SET calls = calls + 1",
            params![user],
        )
        .map_err(backend)?;
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
            .map_err(backend)?;
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
            .map_err(backend)?;
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
            .map_err(backend)?;
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
            .map_err(backend)?;
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
            .map_err(backend)?;
        Ok(found)
    }

    fn blocks(&self) -> Result<Vec<Block>, StoreError> {
        let conn = self.conn.lock_recover();
        let mut stmt = conn
            .prepare("SELECT kind, value FROM v2_blocks ORDER BY kind, value")
            .map_err(backend)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(backend)?;
        rows.map(|row| {
            let (kind, value) = row.map_err(backend)?;
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
        let mut stmt = conn.prepare(sql).map_err(backend)?;
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
            .map_err(backend)?;
        rows.map(|row| row.map_err(backend)).collect()
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
            .map_err(backend)?;
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
            .map_err(backend)?;
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
            .map_err(backend)
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
            .map_err(backend)?;
        Ok(plan.map_or(super::auth::Plan::Anon, |plan| parse_plan(&plan)))
    }

    fn set_user_plan(&self, user: &str, plan: super::auth::Plan) -> Result<(), StoreError> {
        let changed = self
            .conn
            .lock_recover()
            .execute(
                "UPDATE v2_users SET plan = ?1 WHERE id = ?2",
                params![plan.as_str(), user],
            )
            .map_err(backend)?;
        if changed == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock_recover();
        let tx = conn.transaction().map_err(backend)?;
        let is_anonymous: bool = tx
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM v2_users WHERE id = ?1 AND plan = 'anon')",
                params![anonymous],
                |row| row.get(0),
            )
            .map_err(backend)?;
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
        .map_err(backend)?;
        tx.execute(
            "UPDATE v2_tasks SET user_id = ?1 WHERE user_id = ?2",
            params![user, anonymous],
        )
        .map_err(backend)?;
        tx.execute(
            "UPDATE v2_series SET user_id = ?1 WHERE user_id = ?2",
            params![user, anonymous],
        )
        .map_err(backend)?;
        tx.execute(
            "UPDATE v2_series_exceptions SET user_id = ?1 WHERE user_id = ?2",
            params![user, anonymous],
        )
        .map_err(backend)?;
        // The anonymous id ends here: a copy of it held elsewhere must stop working.
        tx.execute(
            "DELETE FROM v2_users WHERE id = ?1 AND plan = 'anon'",
            params![anonymous],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)
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
            .map_err(backend)?;
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
            .map_err(backend)?;
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
            .map_err(backend)?;
        Ok(())
    }

    fn expire_anonymous(&self, cutoff: DateTime<Utc>) -> Result<usize, StoreError> {
        let mut conn = self.conn.lock_recover();
        let tx = conn.transaction().map_err(backend)?;
        let cutoff = format_ts(cutoff);
        let stale = "SELECT id FROM v2_users WHERE plan = 'anon' AND last_activity_at <= ?1";
        // The series' own rows go first, for the same reason the tasks do: nothing
        // cascades here, so leaving them would keep a dead visitor's routines
        // forever with nobody left who could see or delete them.
        tx.execute(
            &format!("DELETE FROM v2_series_exceptions WHERE user_id IN ({stale})"),
            params![cutoff],
        )
        .map_err(backend)?;
        tx.execute(
            &format!("DELETE FROM v2_series WHERE user_id IN ({stale})"),
            params![cutoff],
        )
        .map_err(backend)?;
        tx.execute(
            &format!("DELETE FROM v2_tasks WHERE user_id IN ({stale})"),
            params![cutoff],
        )
        .map_err(backend)?;
        let users = tx
            .execute(
                &format!("DELETE FROM v2_users WHERE id IN ({stale})"),
                params![cutoff],
            )
            .map_err(backend)?;
        tx.commit().map_err(backend)?;
        Ok(users)
    }
}

/// Usage counts are kept this many days; older rows are dropped on write.
const USAGE_RETENTION_DAYS: i64 = 30;

/// Drop per-day counts older than the retention window.
fn prune_before(conn: &Connection, today: NaiveDate) -> Result<(), StoreError> {
    let cutoff = (today - chrono::Duration::days(USAGE_RETENTION_DAYS)).to_string();
    for table in ["v2_capture_counts", "v2_anon_issued"] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE day < ?1"),
            params![cutoff],
        )
        .map_err(backend)?;
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
        .map_err(backend)?;
    let rows = stmt
        .query_map(params![day.to_string(), top as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u32))
        })
        .map_err(backend)?;
    rows.map(|row| row.map_err(backend)).collect()
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
        created: parse_ts(&created).unwrap_or_else(Utc::now),
        updated_at: parse_ts(&updated_at).unwrap_or_else(Utc::now),
        due_tz,
        external_uid: row.get(11)?,
    })
}

fn backend(error: rusqlite::Error) -> StoreError {
    StoreError::Backend(error.to_string())
}

// ---- series (ARCHITECTURE.md 7.1) ----

fn read_series(row: &rusqlite::Row<'_>) -> Result<Series, rusqlite::Error> {
    let weekdays: String = row.get(3)?;
    let until: Option<String> = row.get(4)?;
    let start_day: String = row.get(5)?;
    let time_of_day: Option<String> = row.get(6)?;
    let created: String = row.get(9)?;
    Ok(Series {
        id: SeriesId(row.get(0)?),
        title: row.get(1)?,
        duration_min: row.get::<_, i64>(2)? as u16,
        rule: todo_core::Recurrence {
            weekdays: weekdays_from_stored(&weekdays),
            until: until.and_then(|day| NaiveDate::parse_from_str(&day, "%Y-%m-%d").ok()),
        },
        start_day: NaiveDate::parse_from_str(&start_day, "%Y-%m-%d")
            .unwrap_or_else(|_| Utc::now().date_naive()),
        time_of_day: time_of_day.and_then(|text| parse_time_of_day(&text)),
        zone: row.get(7)?,
        external_uid: row.get(8)?,
        created: parse_ts(&created).unwrap_or_else(Utc::now),
        order: row.get(10)?,
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

fn read_exception(row: &rusqlite::Row<'_>) -> Result<SeriesException, rusqlite::Error> {
    let date: String = row.get(0)?;
    let kind: String = row.get(1)?;
    let moved_when: Option<String> = row.get(2)?;
    let moved_tz: Option<String> = row.get(3)?;
    Ok(SeriesException {
        date: NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .unwrap_or_else(|_| Utc::now().date_naive()),
        // A moved row whose `When` will not parse degrades to skipped: the day
        // stays out of the way instead of failing the load.
        kind: exception_from_parts(&kind, moved_when, moved_tz).unwrap_or(ExceptionKind::Skipped),
    })
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
        other => StoreError::Backend(other.to_string()),
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

/// Every spelling each enum can be stored as, for the round-trip test.
#[cfg(test)]
mod tests {
    const LABELS: [Label; 6] = [
        Label::School,
        Label::Work,
        Label::Errands,
        Label::Health,
        Label::Personal,
        Label::Other,
    ];
    const PRIORITIES: [Priority; 4] = [
        Priority::Low,
        Priority::Medium,
        Priority::High,
        Priority::Critical,
    ];
    const SOURCES: [Source; 6] = [
        Source::Voice,
        Source::Pdf,
        Source::Email,
        Source::Text,
        Source::Calendar,
        Source::Series,
    ];

    use super::*;
    use chrono::{TimeZone, Weekday};
    use todo_core::Recurrence;

    /// The user the contract suite writes as. Scoping is part of the contract now, so
    /// the tests have to name an owner like a real caller does.
    const USER: &str = "alice";
    const OTHER: &str = "bob";

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 16, hour, 0, 0).unwrap()
    }

    fn task(id: &str, order: i64) -> Task {
        Task {
            id: TaskId(id.into()),
            title: format!("Task {id}"),
            duration_min: 30,
            when: When::Anytime,
            done: false,
            order,
            label: Label::Work,
            priority: Priority::Medium,
            source: Source::Text,
            created: at(9),
            updated_at: at(9),
            due_tz: None,
            external_uid: None,
        }
    }

    /// One stored task, by id. Looked up by name so the checks read as claims
    /// about behaviour rather than about where a row happened to sort.
    fn read(store: &dyn Store, id: &TaskId) -> Task {
        store
            .tasks(USER)
            .expect("read tasks")
            .into_iter()
            .find(|task| &task.id == id)
            .unwrap_or_else(|| panic!("no stored task {id:?}"))
    }

    /// The deadline zone (docs/ARCHITECTURE.md 4.1) rides with the task through
    /// whichever store.
    fn due_tz_round_trips(store: &dyn Store) {
        let mut task = task("due", 0);
        task.when = When::DueBy(at(21));
        task.due_tz = Some("America/New_York".into());
        store.put(USER, task.clone()).unwrap();

        let stored = store.tasks(USER).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].when, When::DueBy(at(21)));
        assert_eq!(
            stored[0].due_tz.as_deref(),
            Some("America/New_York"),
            "a deadline's zone must survive a round trip"
        );
    }

    /// A Monday and Thursday series at 07:00, for the store contract.
    fn series(id: &str, order: i64) -> Series {
        Series {
            id: SeriesId(id.into()),
            title: format!("Series {id}"),
            duration_min: 45,
            rule: Recurrence::new([Weekday::Mon, Weekday::Thu], None).unwrap(),
            start_day: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            time_of_day: NaiveTime::from_hms_opt(7, 0, 0),
            zone: "America/New_York".into(),
            external_uid: None,
            created: at(9),
            order,
        }
    }

    fn on(ymd: (i32, u32, u32)) -> NaiveDate {
        NaiveDate::from_ymd_opt(ymd.0, ymd.1, ymd.2).unwrap()
    }

    /// A series and its exceptions survive a round trip through whichever store, one
    /// exception is held per date, and deleting the series takes them with it
    /// (docs/ARCHITECTURE.md 7.1).
    fn series_round_trip(store: &dyn Store) {
        let mut full = series("s1", 0);
        full.rule = Recurrence::new(
            [Weekday::Tue, Weekday::Fri],
            Some(NaiveDate::from_ymd_opt(2026, 12, 18).unwrap()),
        )
        .unwrap();
        full.external_uid = Some("series-uid@rambl".into());
        store.put_series(USER, full.clone()).unwrap();

        assert_eq!(
            store.series(USER).unwrap(),
            vec![full.clone()],
            "every field of a series must survive"
        );

        let moved = ExceptionKind::Moved {
            to: When::At(at(18)),
            to_tz: Some("America/Los_Angeles".into()),
        };
        for (date, kind) in [
            (on((2026, 10, 13)), ExceptionKind::Done),
            (on((2026, 10, 16)), ExceptionKind::Skipped),
            (on((2026, 10, 20)), moved.clone()),
        ] {
            store
                .put_exception(USER, &full.id, SeriesException { date, kind })
                .unwrap();
        }
        let held = store.exceptions(USER, &full.id).unwrap();
        assert_eq!(held.len(), 3);
        assert_eq!(
            held[2].kind, moved,
            "a moved occurrence keeps its new instant and its zone"
        );

        // One row per date: the same day answered twice is one answer, the later one.
        store
            .put_exception(
                USER,
                &full.id,
                SeriesException {
                    date: on((2026, 10, 13)),
                    kind: ExceptionKind::Skipped,
                },
            )
            .unwrap();
        let held = store.exceptions(USER, &full.id).unwrap();
        assert_eq!(held.len(), 3, "one date is never held twice");
        assert_eq!(held[0].kind, ExceptionKind::Skipped);

        // And clearing one puts the day back to the rule.
        store
            .delete_exception(USER, &full.id, on((2026, 10, 16)))
            .unwrap();
        assert_eq!(
            store.exceptions(USER, &full.id).unwrap().len(),
            2,
            "a cleared exception is gone"
        );

        assert!(store.delete_series(USER, &full.id).unwrap());
        assert!(store.series(USER).unwrap().is_empty());
        assert!(
            store.exceptions(USER, &full.id).unwrap().is_empty(),
            "a deleted series leaves no exceptions behind"
        );
        assert!(!store.delete_series(USER, &full.id).unwrap());
    }

    /// A series is owned like a task: another person sees none of it, cannot delete it
    /// by name, and cannot reach its exceptions.
    fn series_are_isolated(store: &dyn Store) {
        store.put_series(USER, series("mine", 0)).unwrap();
        store.put_series(OTHER, series("theirs", 0)).unwrap();
        store
            .put_exception(
                USER,
                &SeriesId("mine".into()),
                SeriesException {
                    date: on((2026, 10, 12)),
                    kind: ExceptionKind::Done,
                },
            )
            .unwrap();

        assert_eq!(store.series(USER).unwrap().len(), 1);
        assert_eq!(store.series(OTHER).unwrap().len(), 1);
        assert!(
            store
                .exceptions(OTHER, &SeriesId("mine".into()))
                .unwrap()
                .is_empty(),
            "bob read alice's exceptions"
        );

        assert!(
            !store
                .delete_series(OTHER, &SeriesId("mine".into()))
                .unwrap(),
            "bob cannot delete alice's series by name"
        );
        assert_eq!(
            store.series(USER).unwrap().len(),
            1,
            "alice kept her series"
        );
        assert_eq!(
            store
                .exceptions(USER, &SeriesId("mine".into()))
                .unwrap()
                .len(),
            1,
            "alice kept her exceptions"
        );
    }

    /// Two people must never see each other's tasks. This is the promise the whole
    /// user_id column exists for, and it is the one a missing filter would break.
    fn users_are_isolated(store: &dyn Store) {
        store.put(USER, task("mine", 0)).unwrap();
        store.put(OTHER, task("theirs", 0)).unwrap();

        // An id held by one user cannot be written, and so cannot be taken over, by
        // another: the first user's task stays where it is, untouched.
        let mut hijack = task("mine", 0);
        hijack.title = "hijacked".into();
        assert!(matches!(
            store.put(OTHER, hijack),
            Err(StoreError::Conflict(_))
        ));
        assert_eq!(store.tasks(USER).unwrap()[0].title, "Task mine");
        assert_eq!(store.tasks(OTHER).unwrap().len(), 1);

        let mine = store.tasks(USER).unwrap();
        assert_eq!(
            mine.len(),
            1,
            "alice should see only her own task, saw {:?}",
            mine.iter().map(|t| &t.title).collect::<Vec<_>>()
        );
        assert_eq!(mine[0].id, TaskId("mine".into()));

        let theirs = store.tasks(OTHER).unwrap();
        assert_eq!(theirs.len(), 1);
        assert_eq!(theirs[0].id, TaskId("theirs".into()));

        // Another person's task must not be reachable by its id, either.
        assert!(
            store.find_by_uid(USER, "theirs-uid").unwrap().is_none(),
            "alice reached bob's task by uid"
        );

        // And deleting only removes the caller's own.
        store.delete(OTHER, &TaskId("mine".into())).unwrap();
        assert_eq!(store.tasks(USER).unwrap().len(), 1, "alice lost her task");
    }
    /// The suite both stores must pass. Everything here goes through the
    /// [`Store`] trait, so a store that passes it behaves like the other one.
    fn store_contract(store: &dyn Store) {
        // -- an empty store has nothing in it
        assert_eq!(store.tasks(USER).unwrap().len(), 0, "a new store is empty");

        // -- put then read back
        store.put(USER, task("t1", 0)).unwrap();
        store.put(USER, task("t2", 1)).unwrap();
        let stored = store.tasks(USER).unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].id, TaskId("t1".into()));
        assert_eq!(stored[1].id, TaskId("t2".into()));

        // -- every field survives the round trip
        let mut original = task("t3", 2);
        original.title = "Read; notes, again".into();
        original.duration_min = 95;
        original.when = When::DueBy(at(17));
        original.done = true;
        original.order = 5;
        original.label = Label::School;
        original.priority = Priority::Critical;
        original.source = Source::Calendar;
        original.created = at(8);
        original.updated_at = at(10);
        original.external_uid = Some("abc-123@rambl".into());
        store.put(USER, original.clone()).unwrap();
        assert_eq!(read(store, &original.id), original);

        // -- each When variant survives
        for (index, when) in [When::Anytime, When::DueBy(at(23)), When::At(at(14))]
            .into_iter()
            .enumerate()
        {
            let mut pinned = task(&format!("w{index}"), 100 + index as i64);
            pinned.when = when;
            store.put(USER, pinned.clone()).unwrap();
            let read = store
                .tasks(USER)
                .unwrap()
                .into_iter()
                .find(|t| t.id == pinned.id)
                .expect("task is stored");
            assert_eq!(read.when, when, "{when:?} did not survive");
        }

        // -- every enum spelling survives
        for (index, label) in LABELS.into_iter().enumerate() {
            let mut labelled = task(&format!("l{index}"), 200 + index as i64);
            labelled.label = label;
            store.put(USER, labelled.clone()).unwrap();
            assert_eq!(read(store, &labelled.id).label, label);
        }
        for (index, priority) in PRIORITIES.into_iter().enumerate() {
            let mut ranked = task(&format!("p{index}"), 300 + index as i64);
            ranked.priority = priority;
            store.put(USER, ranked.clone()).unwrap();
            assert_eq!(read(store, &ranked.id).priority, priority);
        }
        for (index, source) in SOURCES.into_iter().enumerate() {
            let mut sourced = task(&format!("s{index}"), 400 + index as i64);
            sourced.source = source;
            store.put(USER, sourced.clone()).unwrap();
            assert_eq!(read(store, &sourced.id).source, source);
        }

        // -- put on a known id replaces, and does not duplicate
        let before = store.tasks(USER).unwrap().len();
        let mut edited = original.clone();
        edited.title = "Renamed".into();
        edited.done = false;
        store.put(USER, edited.clone()).unwrap();
        let all = store.tasks(USER).unwrap();
        assert_eq!(all.len(), before, "put must replace, never duplicate");
        assert_eq!(read(store, &edited.id).title, "Renamed");
        assert!(!read(store, &edited.id).done);

        // -- tasks come back sorted by order
        let orders: Vec<i64> = store.tasks(USER).unwrap().iter().map(|t| t.order).collect();
        let mut ascending = orders.clone();
        ascending.sort_unstable();
        assert_eq!(orders, ascending, "tasks must come back in order");

        // -- delete reports whether there was something to delete
        let before_delete = store.tasks(USER).unwrap().len();
        assert!(store.delete(USER, &TaskId("t1".into())).unwrap());
        assert!(
            !store.delete(USER, &TaskId("t1".into())).unwrap(),
            "gone already"
        );
        assert!(!store.delete(USER, &TaskId("never-existed".into())).unwrap());
        assert_eq!(store.tasks(USER).unwrap().len(), before_delete - 1);
        assert!(
            !store.tasks(USER).unwrap().iter().any(|t| t.id.0 == "t1"),
            "the deleted task must not come back"
        );

        // -- set_order rewrites the order
        let before: Vec<String> = store
            .tasks(USER)
            .unwrap()
            .into_iter()
            .map(|t| t.id.0)
            .collect();
        let mut reversed = before.clone();
        reversed.reverse();
        let reversed_ids: Vec<TaskId> = reversed.iter().cloned().map(TaskId).collect();
        store.set_order(USER, &reversed_ids).unwrap();
        let after: Vec<String> = store
            .tasks(USER)
            .unwrap()
            .into_iter()
            .map(|t| t.id.0)
            .collect();
        assert_eq!(after, reversed, "set_order is the new order");

        // -- set_order makes the positions 0, 1, ... in the order given
        let positions: Vec<i64> = store.tasks(USER).unwrap().iter().map(|t| t.order).collect();
        assert_eq!(
            positions,
            (0..positions.len() as i64).collect::<Vec<_>>(),
            "positions must be gapless and start at zero"
        );

        // -- an id that is not stored is skipped, not invented
        let count = store.tasks(USER).unwrap().len();
        store.set_order(USER, &[TaskId("nope".into())]).unwrap();
        let after_unknown = store.tasks(USER).unwrap();
        assert_eq!(
            after_unknown.len(),
            count,
            "set_order must not create tasks"
        );
        assert!(
            !after_unknown.iter().any(|t| t.id.0 == "nope"),
            "an unknown id must not become a task"
        );

        // -- a full order is the normal case and must be exact
        let wanted: Vec<String> = store
            .tasks(USER)
            .unwrap()
            .into_iter()
            .map(|t| t.id.0)
            .collect();
        let wanted_ids: Vec<TaskId> = wanted.iter().cloned().map(TaskId).collect();
        store.set_order(USER, &wanted_ids).unwrap();
        let restored: Vec<String> = store
            .tasks(USER)
            .unwrap()
            .into_iter()
            .map(|t| t.id.0)
            .collect();
        assert_eq!(restored, wanted, "set_order is the new order");

        // -- find_by_uid
        assert!(
            store
                .find_by_uid(USER, "abc-123@rambl")
                .unwrap()
                .is_some_and(|t| t.id.0 == "t3")
        );
        assert!(store.find_by_uid(USER, "never-imported").unwrap().is_none());
        assert!(store.find_by_uid(USER, "").unwrap().is_none());

        // -- a task with no uid is not found by any uid
        assert!(
            store
                .tasks(USER)
                .unwrap()
                .iter()
                .filter(|t| t.external_uid.is_none())
                .count()
                > 0
        );
        assert!(store.find_by_uid(USER, "no-such-uid").unwrap().is_none());
    }

    fn anonymous_task(id: &str, uid: Option<&str>) -> Task {
        Task {
            external_uid: uid.map(str::to_owned),
            source: Source::Calendar,
            ..task(id, 0)
        }
    }

    /// Signing in keeps the signed-in copy of a duplicated calendar event, moves the rest,
    /// and ends the anonymous id.
    fn adoption_moves_anonymous_tasks_and_ends_the_id(store: &dyn Store) {
        const ANON: &str = "anon_0123456789abcdef0123456789abcdef";
        store.ensure_anonymous(ANON, at(9)).unwrap();
        store
            .put(ANON, anonymous_task("a1", Some("uid-1")))
            .unwrap();
        store.put(ANON, anonymous_task("a2", None)).unwrap();
        store
            .put(USER, anonymous_task("u1", Some("uid-1")))
            .unwrap();
        store.put_series(ANON, series("anon-series", 0)).unwrap();
        store
            .put_exception(
                ANON,
                &SeriesId("anon-series".into()),
                SeriesException {
                    date: on((2026, 10, 12)),
                    kind: ExceptionKind::Done,
                },
            )
            .unwrap();

        store.adopt_anonymous(ANON, USER).unwrap();

        let mine: Vec<_> = store
            .tasks(USER)
            .unwrap()
            .into_iter()
            .map(|task| task.id.0)
            .collect();
        assert!(
            mine.contains(&"u1".to_owned()),
            "the signed-in copy is kept"
        );
        assert!(
            mine.contains(&"a2".to_owned()),
            "the other anonymous task moves"
        );
        assert!(
            !mine.contains(&"a1".to_owned()),
            "a duplicate of a signed-in event is dropped, not doubled"
        );
        assert!(store.tasks(ANON).unwrap().is_empty());
        assert_eq!(
            store.series(USER).unwrap().len(),
            1,
            "a series made while anonymous moves with the tasks"
        );
        assert_eq!(
            store
                .exceptions(USER, &SeriesId("anon-series".into()))
                .unwrap()
                .len(),
            1,
            "and its exceptions move with it"
        );
        assert!(
            !store.anonymous_exists(ANON).unwrap(),
            "the anonymous id must stop working once it is adopted"
        );
    }

    /// Only a real anonymous id adopts anything. A signed-in user's id, given as the
    /// anonymous header, moves nothing.
    fn adoption_ignores_ids_that_are_not_anonymous(store: &dyn Store) {
        store.put(OTHER, anonymous_task("b1", None)).unwrap();

        store.adopt_anonymous(OTHER, USER).unwrap();

        assert!(store.tasks(USER).unwrap().is_empty(), "bob's tasks moved");
        assert_eq!(store.tasks(OTHER).unwrap().len(), 1);
    }

    /// Blocks are per user and per address, and lifting one leaves the other.
    fn blocks_hold_until_lifted(store: &dyn Store) {
        let user = Block::User(USER.into());
        let ip = Block::Ip("203.0.113.9".into());
        store.block(&user).unwrap();
        store.block(&ip).unwrap();
        store.block(&ip).unwrap();

        assert!(store.is_blocked(USER, "198.51.100.1").unwrap());
        assert!(store.is_blocked(OTHER, "203.0.113.9").unwrap());
        assert!(!store.is_blocked(OTHER, "198.51.100.1").unwrap());
        assert_eq!(
            store.blocks().unwrap().len(),
            2,
            "blocking twice keeps one row"
        );

        assert!(store.unblock(&user).unwrap());
        assert!(!store.unblock(&user).unwrap(), "lifting nothing says so");
        assert!(!store.is_blocked(USER, "198.51.100.1").unwrap());
        assert!(store.is_blocked(OTHER, "203.0.113.9").unwrap());
    }

    /// Captures count per user and per address, and usage ranks them for the day.
    fn captures_are_counted_per_user_and_address(store: &dyn Store) {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let next = chrono::NaiveDate::from_ymd_opt(2026, 10, 13).unwrap();
        store.record_capture(USER, "203.0.113.1", day).unwrap();
        store.record_capture(USER, "203.0.113.1", day).unwrap();
        store.record_capture(USER, "203.0.113.2", day).unwrap();
        store.record_capture(OTHER, "203.0.113.1", day).unwrap();
        store.record_capture(USER, "203.0.113.1", next).unwrap();

        assert_eq!(store.captures_today(USER, day).unwrap(), 3);
        assert_eq!(store.captures_today(USER, next).unwrap(), 1);
        assert_eq!(store.ip_captures_today("203.0.113.1", day).unwrap(), 3);

        let usage = store.usage_today(day, 10).unwrap();
        assert_eq!(usage.users, vec![(USER.into(), 3), (OTHER.into(), 1)]);
        assert_eq!(
            usage.ips,
            vec![("203.0.113.1".into(), 3), ("203.0.113.2".into(), 1)]
        );
        assert_eq!(store.usage_today(day, 1).unwrap().users.len(), 1);
    }

    /// Anonymous ids are issued per address per day, and the count is what the caller
    /// checks against its limit.
    fn anonymous_ids_are_counted_per_address_per_day(store: &dyn Store) {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        assert_eq!(store.record_anon_issued("203.0.113.1", day).unwrap(), 1);
        assert_eq!(store.record_anon_issued("203.0.113.1", day).unwrap(), 2);
        assert_eq!(store.record_anon_issued("203.0.113.2", day).unwrap(), 1);
    }

    /// Sessions, plans, account lookups, user blocks, spend rankings and account
    /// deletion: the calls sign-in, the admin console and usage pages make.
    fn accounts_sessions_and_rankings(store: &dyn Store) {
        use super::super::auth::{Me, Plan};
        let me = |id: &str, plan: Plan| Me {
            id: id.to_owned(),
            email: format!("{id}@example.com"),
            name: id.to_owned(),
            picture: None,
            plan,
        };
        let expires = at(23);
        store
            .upsert_user_and_session(&me(USER, Plan::Free), "tok-alice", expires)
            .unwrap();
        store
            .upsert_user_and_session(&me(USER, Plan::Free), "tok-alice-2", expires)
            .unwrap();
        store
            .upsert_user_and_session(&me(OTHER, Plan::Paid), "tok-bob", expires)
            .unwrap();

        // -- a live session resolves to its user; an unknown or expired one to nobody
        let owner = |token: &str, now| store.session_user(token, now).unwrap().map(|m| m.id);
        assert_eq!(owner("tok-alice", at(12)), Some(USER.into()));
        assert_eq!(owner("no-such-token", at(12)), None);
        assert_eq!(owner("tok-bob", at(23) + chrono::Duration::hours(1)), None);

        // -- signing out one session leaves the user's other sessions alone
        store.delete_session("tok-alice").unwrap();
        assert_eq!(
            owner("tok-alice", at(12)),
            None,
            "signed-out token still works"
        );
        assert_eq!(owner("tok-alice-2", at(12)), Some(USER.into()));

        // -- plans are per user
        assert_eq!(store.user_plan(USER).unwrap(), Plan::Free);
        store.set_user_plan(USER, Plan::Paid).unwrap();
        assert_eq!(store.user_plan(USER).unwrap(), Plan::Paid);
        assert_eq!(store.user_plan(OTHER).unwrap(), Plan::Paid);

        // -- lookups: an account exists, and an email finds a signed-in user only
        assert!(store.account_exists(USER).unwrap());
        assert!(!store.account_exists("nobody").unwrap());
        assert_eq!(
            store.user_id_for_email("bob@example.com").unwrap(),
            Some(OTHER.into())
        );
        assert_eq!(store.user_id_for_email("nobody@example.com").unwrap(), None);
        store.ensure_anonymous(&anon_id(1), at(9)).unwrap();
        assert_eq!(
            store.user_id_for_email("").unwrap(),
            None,
            "an anonymous visitor has no email to find"
        );

        // -- a user block is not an address block
        store.block(&Block::Ip("203.0.113.9".into())).unwrap();
        assert!(!store.is_user_blocked(USER).unwrap());
        store.block(&Block::User(USER.into())).unwrap();
        assert!(store.is_user_blocked(USER).unwrap());
        assert!(!store.is_user_blocked(OTHER).unwrap());

        // -- rankings: all time, and today; ties go by id
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let earlier = chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        for _ in 0..3 {
            store.record_capture(USER, "203.0.113.1", day).unwrap();
        }
        store.record_capture(OTHER, "203.0.113.2", day).unwrap();
        for _ in 0..3 {
            store.record_capture(OTHER, "203.0.113.2", earlier).unwrap();
        }
        let ids = |rows: &[UsageRow]| rows.iter().map(|r| r.user.clone()).collect::<Vec<_>>();

        let all_time = store.usage_ranking(UsageOrder::AllTime, 10).unwrap();
        assert_eq!(
            ids(&all_time),
            vec![OTHER, USER],
            "bob has 4 calls, alice 3"
        );
        assert_eq!(all_time[0].calls, 4);
        assert_eq!(all_time[0].email, "bob@example.com");
        assert_eq!(all_time[0].plan, Plan::Paid);

        let today = store.usage_ranking(UsageOrder::Day(day), 10).unwrap();
        assert_eq!(ids(&today), vec![USER, OTHER]);
        assert_eq!(
            today.iter().map(|r| r.calls).collect::<Vec<_>>(),
            vec![3, 1]
        );

        assert_eq!(
            store.usage_ranking(UsageOrder::AllTime, 1).unwrap().len(),
            1,
            "top limits the rows"
        );

        // -- deleting a user takes their sessions and tasks, and nobody else's
        store.put(USER, task("alice-task", 0)).unwrap();
        store.put(OTHER, task("bob-task", 0)).unwrap();
        store.delete_user(USER).unwrap();
        assert!(
            !store.account_exists(USER).unwrap(),
            "deleted user still exists"
        );
        assert_eq!(
            owner("tok-alice-2", at(12)),
            None,
            "deleted user kept a session"
        );
        assert!(
            store.tasks(USER).unwrap().is_empty(),
            "deleted user kept tasks"
        );
        assert_eq!(
            store.tasks(OTHER).unwrap().len(),
            1,
            "deleting alice touched bob"
        );
        assert_eq!(owner("tok-bob", at(12)), Some(OTHER.into()));
    }

    /// Runs one contract on a fresh SQLite database in a folder of its own. The folder
    /// is deleted when the call returns, after the store is closed.
    fn on_disk(run: impl Fn(&dyn Store)) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = SqliteStore::open(&dir.path().join("rambl.db")).expect("open");
        run(&store);
    }

    #[test]
    fn memory_store_accounts_sessions_and_rankings() {
        accounts_sessions_and_rankings(&MemoryStore::new());
    }

    #[test]
    fn sqlite_store_accounts_sessions_and_rankings() {
        on_disk(accounts_sessions_and_rankings);
    }

    /// Every shared contract, each on a fresh file-backed SQLite store, so the on-disk
    /// path is tested as well as the in-memory one.
    #[test]
    fn sqlite_file_store_passes_every_contract() {
        on_disk(users_are_isolated);
        on_disk(store_contract);
        on_disk(due_tz_round_trips);
        on_disk(series_round_trip);
        on_disk(series_are_isolated);
        on_disk(adoption_moves_anonymous_tasks_and_ends_the_id);
        on_disk(adoption_ignores_ids_that_are_not_anonymous);
        on_disk(blocks_hold_until_lifted);
        on_disk(captures_are_counted_per_user_and_address);
        on_disk(anonymous_ids_are_counted_per_address_per_day);
        on_disk(idle_anonymous_accounts_expire);
        on_disk(visits_keep_anonymous_ids_alive_with_a_throttle);
        on_disk(accounts_sessions_and_rankings);
    }

    #[test]
    fn memory_store_adoption_and_scoping_rules() {
        adoption_moves_anonymous_tasks_and_ends_the_id(&MemoryStore::default());
        adoption_ignores_ids_that_are_not_anonymous(&MemoryStore::default());
    }

    #[test]
    fn sqlite_store_adoption_and_scoping_rules() {
        let open = || SqliteStore::open_in_memory().unwrap();
        adoption_moves_anonymous_tasks_and_ends_the_id(&open());
        adoption_ignores_ids_that_are_not_anonymous(&open());
    }

    #[test]
    fn memory_store_blocks_usage_and_anonymous_counts() {
        let store = MemoryStore::default();
        blocks_hold_until_lifted(&store);
        captures_are_counted_per_user_and_address(&store);
        anonymous_ids_are_counted_per_address_per_day(&store);
    }

    #[test]
    fn sqlite_store_blocks_usage_and_anonymous_counts() {
        let store = SqliteStore::open_in_memory().unwrap();
        blocks_hold_until_lifted(&store);
        captures_are_counted_per_user_and_address(&store);
        anonymous_ids_are_counted_per_address_per_day(&store);
    }

    /// An anonymous id is a database row, so a restart keeps it working.
    #[test]
    fn an_anonymous_id_survives_a_reopen() {
        let dir = unique_temp_dir();
        let path = dir.join("rambl.db");
        const ANON: &str = "anon_0123456789abcdef0123456789abcdef";
        {
            let store = SqliteStore::open(&path).unwrap();
            store.ensure_anonymous(ANON, at(9)).unwrap();
            store.put(ANON, anonymous_task("a1", None)).unwrap();
        }
        let reopened = SqliteStore::open(&path).unwrap();
        assert!(reopened.anonymous_exists(ANON).unwrap());
        assert_eq!(reopened.tasks(ANON).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn memory_store_passes_the_contract() {
        users_are_isolated(&MemoryStore::new());
        store_contract(&MemoryStore::new());
        due_tz_round_trips(&MemoryStore::new());
        series_round_trip(&MemoryStore::new());
        series_are_isolated(&MemoryStore::new());
    }

    #[test]
    fn sqlite_store_passes_the_contract() {
        users_are_isolated(&SqliteStore::open_in_memory().unwrap());
        store_contract(&SqliteStore::open_in_memory().unwrap());
        due_tz_round_trips(&SqliteStore::open_in_memory().unwrap());
        series_round_trip(&SqliteStore::open_in_memory().unwrap());
        series_are_isolated(&SqliteStore::open_in_memory().unwrap());
    }

    /// A session finds its own user, and only while it is live, on the real database.
    #[test]
    fn sqlite_session_finds_its_user_until_it_expires() {
        let store = SqliteStore::open_in_memory().unwrap();
        let me = super::super::auth::Me {
            id: "alice".to_owned(),
            email: "alice@example.com".to_owned(),
            name: "alice".to_owned(),
            picture: None,
            plan: super::super::auth::Plan::Free,
        };
        store
            .upsert_user_and_session(&me, "token-alice", at(12))
            .unwrap();
        let found = store.session_user("token-alice", at(1)).unwrap();
        assert_eq!(found.map(|me| me.id), Some("alice".to_owned()));
        assert!(store.session_user("token-alice", at(13)).unwrap().is_none());
        assert!(store.session_user("nope", at(1)).unwrap().is_none());
    }

    // -- SQLite specifics the contract cannot cover through the trait.

    #[test]
    fn a_file_store_keeps_its_tasks_across_a_reopen() {
        let path = unique_temp_dir().join("nested").join("rambl.db");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());

        {
            let store = SqliteStore::open(&path).unwrap();
            store.put(USER, task("t1", 0)).unwrap();
            store.put(USER, task("t2", 1)).unwrap();
        }

        // Fresh handle, same file: this is the restart case.
        let store = SqliteStore::open(&path).unwrap();
        let stored = store.tasks(USER).unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].id, TaskId("t1".into()));
        assert_eq!(stored[1].id, TaskId("t2".into()));

        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn reopening_a_current_database_applies_nothing_and_loses_nothing() {
        let path = unique_temp_dir().join("rambl.db");
        let _ = std::fs::remove_dir_all(&path);

        let store = SqliteStore::open(&path).unwrap();
        store.put(USER, task("t1", 0)).unwrap();
        drop(store);

        // Second open runs migrate() again; it must be a no-op, not a rebuild.
        let store = SqliteStore::open(&path).unwrap();
        assert_eq!(store.tasks(USER).unwrap().len(), 1);
        assert_eq!(store.count(), 1);

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn the_schema_is_at_the_version_the_code_expects() {
        let store = SqliteStore::open_in_memory().unwrap();
        let conn = store.conn.lock_recover();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn the_column_list_matches_the_table() {
        let store = SqliteStore::open_in_memory().unwrap();
        let conn = store.conn.lock_recover();
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info('v2_tasks')")
            .unwrap();
        let actual: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(stmt);

        let listed: Vec<&str> = TASK_COLUMNS
            .split(',')
            .map(|name| name.trim().trim_matches('"'))
            .collect();
        // `user_id` is the one column on the table that is not a task field: it
        // scopes every read and write rather than describing a task, so it is bound
        // separately rather than read into a `Task`.
        let mut expected = actual.clone();
        expected.retain(|name| name != "user_id");
        assert_eq!(
            listed, expected,
            "TASK_COLUMNS must list exactly the task columns, in order"
        );
        assert!(
            actual.contains(&"user_id".to_owned()),
            "tasks must be scoped to a user"
        );
    }

    #[test]
    fn two_tasks_cannot_claim_one_uid() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut first = task("t1", 0);
        first.external_uid = Some("shared".into());
        store.put(USER, first).unwrap();

        let mut second = task("t2", 1);
        second.external_uid = Some("shared".into());
        let error = store.put(USER, second).unwrap_err();
        assert!(
            matches!(error, StoreError::Conflict(_)),
            "expected a conflict, got {error:?}"
        );
        assert_eq!(store.count(), 1, "the clashing task was not stored");
    }

    #[test]
    fn memory_store_reports_the_same_uid_conflict() {
        let store = MemoryStore::new();
        let mut first = task("t1", 0);
        first.external_uid = Some("shared".into());
        store.put(USER, first).unwrap();

        let mut second = task("t2", 1);
        second.external_uid = Some("shared".into());
        assert!(matches!(
            store.put(USER, second),
            Err(StoreError::Conflict(_))
        ));
        assert_eq!(store.tasks(USER).unwrap().len(), 1);
    }

    #[test]
    fn re_importing_a_uid_updates_that_task_in_place() {
        let store = MemoryStore::new();
        let mut imported = task("t1", 0);
        imported.external_uid = Some("evt-1".into());
        imported.when = When::At(at(14));
        store.put(USER, imported).unwrap();

        // The event moved to Friday: same UID, new time.
        let existing = store.find_by_uid(USER, "evt-1").unwrap().expect("found");
        let mut moved = existing.clone();
        moved.when = When::At(at(17));
        store.put(USER, moved).unwrap();

        assert_eq!(
            store.find_by_uid(USER, "evt-1").unwrap().unwrap().when,
            When::At(at(17))
        );
    }

    #[test]
    fn re_importing_same_uid_updates_not_duplicates() {
        let store = MemoryStore::new();
        let mut first = task("first", 0);
        first.external_uid = Some("re-uid".into());
        store.put(USER, first).unwrap();
        let existing = store.find_by_uid(USER, "re-uid").unwrap().expect("found");
        let mut updated = existing.clone();
        updated.title = "updated".into();
        store.put(USER, updated).unwrap();
        assert_eq!(
            store.tasks(USER).unwrap().len(),
            1,
            "re-import must not duplicate"
        );
        assert_eq!(
            store.find_by_uid(USER, "re-uid").unwrap().unwrap().title,
            "updated"
        );
    }

    #[test]
    fn v2_never_touches_the_v1_tasks_table() {
        let store = SqliteStore::open_in_memory().unwrap();
        // A v1-shaped table in the same file must survive, untouched.
        store
            .conn
            .lock_recover()
            .execute_batch(
                "CREATE TABLE tasks (id TEXT PRIMARY KEY, title TEXT NOT NULL);
                 INSERT INTO tasks (id, title) VALUES ('v1-1', 'v1 task');",
            )
            .unwrap();

        store.put(USER, task("t1", 0)).unwrap();

        let conn = store.conn.lock_recover();
        let v1: String = conn
            .query_row("SELECT title FROM tasks WHERE id = 'v1-1'", [], |row| {
                row.get(0)
            })
            .expect("the v1 row is still there");
        assert_eq!(v1, "v1 task");
    }

    #[test]
    fn a_hand_edited_row_degrades_instead_of_failing_the_load() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .conn
            .lock_recover()
            .execute_batch(
                "INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, \
                                        done, \"order\", label, priority, source, created, external_uid, user_id) \
                 VALUES ('odd', 'Odd', 30, 'whenever', 'not-a-date', 0, 0, \
                         'mauve', 'enormous', 'telepathy', 'yesterday', NULL, 'alice');",
            )
            .unwrap();

        let stored = store.tasks(USER).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].when, When::Anytime);
        assert_eq!(stored[0].label, Label::Other);
        assert_eq!(stored[0].priority, Priority::Medium);
        assert_eq!(stored[0].source, Source::Text);
    }

    #[test]
    fn store_errors_read_as_something_a_user_can_see() {
        for error in [
            StoreError::NotFound,
            StoreError::Conflict("two tasks claim one uid".into()),
            StoreError::Backend("disk is full".into()),
        ] {
            let message = error.to_string();
            assert!(!message.is_empty(), "{error:?} rendered nothing");
            assert!(!message.contains('{'), "{error:?} leaked a format brace");
        }
    }

    #[test]
    fn when_round_trips_through_its_two_columns() {
        for when in [When::Anytime, When::DueBy(at(17)), When::At(at(14))] {
            let (kind, time) = when_to_parts(when);
            assert_eq!(when_from_parts(kind, time.map(format_ts).as_deref()), when);
        }
        assert_eq!(when_to_parts(When::Anytime), ("anytime", None));
    }

    const DAY: Duration = Duration::days(1);

    fn anon_id(n: u32) -> String {
        format!("anon_{n:032x}")
    }

    /// A signed-in user (plan `free`) with one task, so expiry has something to spare.
    fn sign_in(store: &dyn Store, id: &str) {
        let me = super::super::auth::Me {
            id: id.to_owned(),
            email: format!("{id}@example.com"),
            name: id.to_owned(),
            picture: None,
            plan: super::super::auth::Plan::Free,
        };
        store
            .upsert_user_and_session(&me, &format!("token-{id}"), at(23))
            .unwrap();
        store.put(id, task("keep", 0)).unwrap();
    }

    /// Idle anonymous accounts go, with their tasks and their series, and nobody else
    /// does.
    fn idle_anonymous_accounts_expire(store: &dyn Store) {
        let now = at(12);
        let (just_inside, exactly_old, long_gone) = (anon_id(1), anon_id(2), anon_id(3));
        for (id, last_seen) in [
            (&just_inside, now - DAY * 30 + Duration::hours(1)), // 29 days 23 hours
            (&exactly_old, now - DAY * 30),                      // 30 days
            (&long_gone, now - DAY * 90),
        ] {
            store.ensure_anonymous(id, last_seen).unwrap();
            store.put(id, task(&format!("{id}-1"), 0)).unwrap();
            store.put(id, task(&format!("{id}-2"), 1)).unwrap();
            store
                .put_series(id, series(&format!("{id}-series"), 0))
                .unwrap();
            store
                .put_exception(
                    id,
                    &SeriesId(format!("{id}-series")),
                    SeriesException {
                        date: on((2026, 10, 12)),
                        kind: ExceptionKind::Done,
                    },
                )
                .unwrap();
        }
        sign_in(store, "carol");

        let removed = store.expire_anonymous(now - DAY * 30).unwrap();

        assert_eq!(removed, 2, "30 days and older go; 29 days 23 hours stays");
        assert!(store.anonymous_exists(&just_inside).unwrap());
        assert_eq!(store.tasks(&just_inside).unwrap().len(), 2);
        assert_eq!(
            store.series(&just_inside).unwrap().len(),
            1,
            "a live visitor keeps their series"
        );
        for gone in [&exactly_old, &long_gone] {
            assert!(!store.anonymous_exists(gone).unwrap(), "{gone} survived");
            assert!(store.tasks(gone).unwrap().is_empty(), "{gone} kept tasks");
            assert!(
                store.series(gone).unwrap().is_empty(),
                "{gone} kept a series that nobody can reach"
            );
            assert!(
                store
                    .exceptions(gone, &SeriesId(format!("{gone}-series")))
                    .unwrap()
                    .is_empty(),
                "{gone} kept the exceptions of a series it no longer has"
            );
        }
        assert_eq!(
            store.tasks("carol").unwrap().len(),
            1,
            "a signed-in user is never expired"
        );
        assert_eq!(store.expire_anonymous(now - DAY * 30).unwrap(), 0);
    }

    /// A visit keeps an anonymous id alive, but is written at most once an hour.
    fn visits_keep_anonymous_ids_alive_with_a_throttle(store: &dyn Store) {
        let start = at(0);
        let id = anon_id(7);
        store.ensure_anonymous(&id, start).unwrap();

        // Within the hour nothing is written, so the old time still expires it.
        store
            .touch_anonymous(&id, start + Duration::minutes(30))
            .unwrap();
        let throttled = store
            .expire_anonymous(start + Duration::minutes(10))
            .unwrap();
        assert_eq!(
            throttled, 1,
            "a visit inside the throttle must not be written"
        );

        let id = anon_id(8);
        store.ensure_anonymous(&id, start).unwrap();
        store.touch_anonymous(&id, start + DAY * 20).unwrap();
        assert_eq!(
            store.expire_anonymous(start + DAY * 19).unwrap(),
            0,
            "a visit after the throttle keeps the id alive"
        );
        assert!(store.anonymous_exists(&id).unwrap());

        // Signed-in users and unknown ids are ignored, not an error.
        sign_in(store, "dave");
        store.touch_anonymous("dave", at(1)).unwrap();
        store.touch_anonymous("nobody", at(1)).unwrap();
    }

    #[test]
    fn both_stores_expire_idle_anonymous_accounts() {
        idle_anonymous_accounts_expire(&MemoryStore::new());
        idle_anonymous_accounts_expire(&SqliteStore::open_in_memory().unwrap());
    }

    #[test]
    fn both_stores_throttle_activity_writes() {
        visits_keep_anonymous_ids_alive_with_a_throttle(&MemoryStore::new());
        visits_keep_anonymous_ids_alive_with_a_throttle(&SqliteStore::open_in_memory().unwrap());
    }

    #[test]
    fn a_signed_in_user_is_kept_however_long_since_their_last_activity() {
        let store = SqliteStore::open_in_memory().unwrap();
        sign_in(&store, "erin");
        store
            .conn
            .lock_recover()
            .execute(
                "UPDATE v2_users SET last_activity_at = ?1 WHERE id = 'erin'",
                params![format_ts(at(12) - DAY * 60)],
            )
            .unwrap();

        assert_eq!(store.expire_anonymous(at(12) - DAY * 30).unwrap(), 0);
        assert_eq!(store.tasks("erin").unwrap().len(), 1);
    }

    /// A database as an earlier build left it: the first `version` migrations applied.
    fn database_at(path: &Path, version: usize) -> Connection {
        let conn = Connection::open(path).unwrap();
        for migration in &MIGRATIONS[..version] {
            conn.execute_batch(migration).unwrap();
        }
        conn.pragma_update(None, "user_version", version as i64)
            .unwrap();
        conn
    }

    #[test]
    fn a_version_5_database_migrates_and_keeps_its_data() {
        let path = unique_temp_dir().join("rambl.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = Utc::now();
        {
            let conn = database_at(&path, 5);
            conn.execute_batch(
                "INSERT INTO v2_users (id, email, name, picture, created_at, plan) VALUES
                     ('anon_old', '', '', NULL, '2026-01-01T00:00:00+00:00', 'anon'),
                     ('carol', 'c@example.com', 'Carol', NULL, '2026-01-01T00:00:00+00:00', 'free');
                 INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, done,
                     \"order\", label, priority, source, created, external_uid, user_id) VALUES
                     ('a1', 'Anon task', 30, 'anytime', NULL, 0, 0, 'work', 'medium', 'text',
                      '2026-02-03T04:05:06+00:00', NULL, 'anon_old'),
                     ('c1', 'Carol task', 30, 'anytime', NULL, 0, 0, 'work', 'medium', 'text',
                      '2026-02-03T04:05:06+00:00', NULL, 'carol');",
            )
            .unwrap();
        }

        let store = SqliteStore::open(&path).expect("a v5 database must open and migrate");

        let carol = store.tasks("carol").unwrap();
        assert_eq!(carol.len(), 1);
        assert_eq!(
            carol[0].updated_at, carol[0].created,
            "an existing task's version is when it was created"
        );
        assert_eq!(store.tasks("anon_old").unwrap().len(), 1);
        // Nobody is expired for the time before idleness was tracked: the migration
        // counts as their last visit.
        assert_eq!(store.expire_anonymous(before - DAY * 30).unwrap(), 0);
        assert!(store.anonymous_exists("anon_old").unwrap());
        assert_eq!(store.expire_anonymous(Utc::now() + DAY).unwrap(), 1);
        let version: i64 = store
            .conn
            .lock_recover()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A version-7 database gains the two series tables, keeps what it had, and can hold a
    /// series from then on.
    #[test]
    fn a_version_7_database_gains_the_series_tables() {
        let path = unique_temp_dir().join("rambl.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        {
            let conn = database_at(&path, 7);
            conn.execute_batch(
                "INSERT INTO v2_users (id, email, name, picture, created_at, plan) VALUES
                     ('dana', 'd@example.com', 'Dana', NULL, '2026-01-01T00:00:00+00:00', 'free');
                 INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, done,
                     \"order\", label, priority, source, created, external_uid, updated_at,
                     due_tz, user_id) VALUES
                     ('d1', 'Dana task', 30, 'anytime', NULL, 0, 0, 'work', 'medium', 'text',
                      '2026-02-03T04:05:06+00:00', NULL, '2026-02-03T04:05:06+00:00', NULL,
                      'dana');",
            )
            .unwrap();
        }

        let store = SqliteStore::open(&path).expect("a v7 database must open and migrate");
        assert_eq!(store.tasks("dana").unwrap().len(), 1, "her task survives");
        assert!(
            store.series("dana").unwrap().is_empty(),
            "a database written before series existed starts with none"
        );
        store.put_series("dana", series("s1", 0)).unwrap();
        assert_eq!(store.series("dana").unwrap().len(), 1, "and holds one now");
        let version: i64 = store
            .conn
            .lock_recover()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A version-8 database gains the v2_usage_totals table, keeps tasks and series.
    #[test]
    fn a_version_8_database_gains_usage_totals() {
        let path = unique_temp_dir().join("rambl.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        {
            let conn = database_at(&path, 8);
            conn.execute_batch(
                "INSERT INTO v2_users (id, email, name, picture, created_at, plan) VALUES
                     ('eve', 'e@example.com', 'Eve', NULL, '2026-01-01T00:00:00+00:00', 'free');
                 INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, done,
                     \"order\", label, priority, source, created, external_uid, updated_at,
                     due_tz, user_id) VALUES
                     ('e1', 'Eve task', 30, 'anytime', NULL, 0, 0, 'work', 'medium', 'text',
                      '2026-02-03T04:05:06+00:00', NULL, '2026-02-03T04:05:06+00:00', NULL,
                      'eve');
                 INSERT INTO v2_series (id, user_id, title, duration_min, weekdays, until,
                     start_day, time_of_day, zone, external_uid, created, \"order\") VALUES
                     ('ser1', 'eve', 'Weekly meeting', 60, '1', NULL, '2026-01-01',
                      '09:00:00', 'UTC', NULL, '2026-01-01T00:00:00+00:00', 0);",
            )
            .unwrap();
        }

        let store = SqliteStore::open(&path).expect("a v8 database must open and migrate");
        assert_eq!(store.tasks("eve").unwrap().len(), 1, "her task survives");
        assert_eq!(store.series("eve").unwrap().len(), 1, "her series survives");
        // v2_usage_totals table exists and is empty
        let has_table: bool = store
            .conn
            .lock_recover()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='v2_usage_totals')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(has_table, "v2_usage_totals table was created");
        let version: i64 = store
            .conn
            .lock_recover()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_version_6_database_migrates_its_tasks_without_a_due_zone() {
        let path = unique_temp_dir().join("rambl.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        {
            let conn = database_at(&path, 6);
            // A v6 row has no due_tz column at all: this is the shape an install
            // running before the feature wrote.
            conn.execute_batch(
                "INSERT INTO v2_users (id, email, name, picture, created_at, plan) VALUES
                     ('carol', 'c@example.com', 'Carol', NULL, '2026-01-01T00:00:00+00:00', 'free');
                 INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, done,
                     \"order\", label, priority, source, created, external_uid, updated_at,
                     user_id) VALUES
                     ('c1', 'Carol task', 30, 'due_by', '2026-11-03T04:59:59+00:00', 0, 0,
                      'work', 'medium', 'text', '2026-02-03T04:05:06+00:00', NULL,
                      '2026-02-03T04:05:06+00:00', 'carol');",
            )
            .unwrap();
        }

        let store = SqliteStore::open(&path).expect("a v6 database must open and migrate");
        let carol = store.tasks("carol").unwrap();
        assert_eq!(carol.len(), 1, "the pre-feature task survives");
        assert_eq!(
            carol[0].when,
            When::DueBy(Utc.with_ymd_and_hms(2026, 11, 3, 4, 59, 59).unwrap())
        );
        assert_eq!(
            carol[0].due_tz, None,
            "a task written before due_tz existed has no zone"
        );
        let version: i64 = store
            .conn
            .lock_recover()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_database_in_the_middle_of_the_history_applies_only_what_it_lacks() {
        let path = unique_temp_dir().join("rambl.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        drop(database_at(&path, 3));

        SqliteStore::open(&path).expect("migrating from version 3 must not redo version 2");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// An upgrade must not sign anyone out: a session issued before tokens were hashed
    /// keeps working, is stored hashed afterwards, and is not hashed again on a reopen.
    #[test]
    fn a_version_9_session_survives_the_move_to_hashed_tokens() {
        let path = unique_temp_dir().join("rambl.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let plain = "0123456789abcdef0123456789abcdef";
        {
            let conn = database_at(&path, 9);
            conn.execute_batch(&format!(
                "INSERT INTO v2_users (id, email, name, picture, created_at, plan) VALUES
                     ('dana', 'dana@example.com', 'Dana', NULL, '2026-01-01T00:00:00Z', 'free');
                 INSERT INTO v2_sessions (token, user_id, expires_at, created_at) VALUES
                     ('{plain}', 'dana', '2099-01-01T00:00:00Z', '2026-01-01T00:00:00Z');"
            ))
            .unwrap();
        }

        for _ in 0..2 {
            let store = SqliteStore::open(&path).unwrap();
            let me = store.session_user(plain, at(12)).unwrap();
            assert_eq!(me.map(|me| me.id), Some("dana".to_owned()));
            let stored: String = store
                .conn
                .lock_recover()
                .query_row("SELECT token FROM v2_sessions", [], |row| row.get(0))
                .unwrap();
            assert_eq!(stored, hash_token(plain));
            assert_ne!(stored, plain);
        }

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_token_hashes_to_its_known_sha256() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// A folder of its own, so two tests writing at once cannot collide.
    fn unique_temp_dir() -> std::path::PathBuf {
        let stem = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("rambl-store-{}-{stem}", std::process::id()))
    }
}

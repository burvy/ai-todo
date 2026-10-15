//! The SQLite schema: its version, its migrations in order, and the format of a stored day time.

/// Bumped whenever [`MIGRATIONS`] grows an entry. The count on disk is
/// `PRAGMA user_version`; anything below this has the remaining migrations
/// applied on open.
pub(super) const SCHEMA_VERSION: i64 = 13;

/// Ordered by `user_version`. Entry n applies to a database sitting at
/// version n - 1, so the list is append-only and never reordered.
pub(super) const MIGRATIONS: &[&str] = &[
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

pub(super) const MIGRATION_1: &str = "
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
pub(super) const MIGRATION_2: &str = "
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
pub(super) const MIGRATION_3: &str =
    "ALTER TABLE v2_users ADD COLUMN plan TEXT NOT NULL DEFAULT 'free';";
/// Per-user capture counts per client address per day, for the Jev caps and `usage`.
/// Old days are pruned on write, so the table holds about a month.
pub(super) const MIGRATION_4: &str = "
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
pub(super) const MIGRATION_5: &str = "
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
pub(super) const MIGRATION_6: &str = "
ALTER TABLE v2_tasks ADD COLUMN updated_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00+00:00';
UPDATE v2_tasks SET updated_at = created;
ALTER TABLE v2_users ADD COLUMN last_activity_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00+00:00';
UPDATE v2_users SET last_activity_at = strftime('%Y-%m-%dT%H:%M:%S+00:00', 'now');
";

pub(super) const MIGRATION_7: &str = "ALTER TABLE v2_tasks ADD COLUMN due_tz TEXT;";

/// Weekly series (ARCHITECTURE.md 7.1). The rows never list the weeks; the
/// server expands occurrences at snapshot time. `weekdays` holds the
/// `chrono::Weekday` numbers Monday=1..Sunday=7 as digits (`"124"` = Mon/Tue/Thu).
/// A deviation from the rule lives in `v2_series_exceptions`, one per local
/// date: `done` / `skipped` / `moved`, with the moved target as the same JSON
/// a task's `When` uses on the wire.
pub(super) const MIGRATION_8: &str = "
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
pub(super) const MIGRATION_9: &str = "
CREATE TABLE IF NOT EXISTS v2_usage_totals (
    user_id TEXT PRIMARY KEY,
    calls  INTEGER NOT NULL DEFAULT 0
);
";

/// Session tokens are stored hashed. The rows already there are hashed in Rust, right
/// after this runs (`hash_plain_sessions`), because SQLite has no SHA-256 of its own.
pub(super) const MIGRATION_10: &str = "SELECT 1;";

/// The two times of day a day-only task depends on (`DaySettings`), as `HH:MM`. NULL
/// means the default, so a row nobody has changed follows the default if it moves.
pub(super) const MIGRATION_11: &str = "
ALTER TABLE v2_users ADD COLUMN day_deadline TEXT;
ALTER TABLE v2_users ADD COLUMN day_placement TEXT;
";

/// The user took a task off the planner (ARCHITECTURE.md 4.2). The default is 0, so a
/// row written before the flag existed is still planned, as it always was.
pub(super) const MIGRATION_12: &str =
    "ALTER TABLE v2_tasks ADD COLUMN off_planner INTEGER NOT NULL DEFAULT 0;";

/// How a day-setting time is written in the database and on the wire.
pub(super) const DAY_TIME_FORMAT: &str = "%H:%M";

/// Interval column for series: every N weeks (weekly) or every N days (daily).
pub(super) const MIGRATION_13: &str = "
ALTER TABLE v2_series ADD COLUMN interval INTEGER NOT NULL DEFAULT 1;
";

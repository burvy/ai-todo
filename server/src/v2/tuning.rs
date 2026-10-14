//! Every number the server can be tuned by, in one place. See `docs/TUNING.md`.
//!
//! Each value says what it does, its unit and a safe range. Values a deployment
//! overrides with an environment variable are the defaults for that variable, which
//! is named beside them. Task lengths are shared with the crates and live in
//! `todo_core::tuning`.

use std::time::Duration;

/// The port the server listens on when neither `RAMBL_PORT` nor `PORT` is set. The
/// firewall and the Cloudflare tunnel on the server machine expect this one.
pub const DEFAULT_PORT: u16 = 3160;

// ---- accounts and sessions

/// How long a sign-in lasts before the session token stops working. Safe range 1 to
/// 365 days; shorter means signing in again more often.
pub const SESSION_LENGTH: Duration = Duration::from_secs(60 * 60 * 24 * 90);

/// How long a sign-in waits for Google's `tokeninfo` before giving up. Safe range 3
/// to 30 seconds: long enough for a slow network, short enough that a stuck Google
/// does not hold the request open.
pub const GOOGLE_TIMEOUT: Duration = Duration::from_secs(10);

/// An anonymous account unused for this long is deleted with its tasks, after the
/// daily backup. Safe range 7 to 365 days.
pub const ANONYMOUS_IDLE_LIMIT: chrono::Duration = chrono::Duration::days(30);

/// An anonymous user's last-activity time is written at most this often, so a busy
/// visitor does not cost a database write per request. Must stay far below
/// [`ANONYMOUS_IDLE_LIMIT`]. Safe range 1 minute to 1 day.
pub const ACTIVITY_THROTTLE: chrono::Duration = chrono::Duration::hours(1);

// ---- daily limits (each overridable by the environment variable named)

/// Captures a signed-in user may run per day before capture stops using Jev.
/// `RAMBL_JEV_DAILY_CAP`.
pub const DEFAULT_JEV_DAILY_CAP: u32 = 200;
/// The same for a user on the paid plan. `RAMBL_JEV_DAILY_CAP_PAID`.
pub const DEFAULT_PAID_JEV_DAILY_CAP: u32 = 1000;
/// The same for an anonymous user. `RAMBL_JEV_DAILY_CAP_ANON`.
pub const DEFAULT_ANON_JEV_DAILY_CAP: u32 = 30;
/// Anonymous ids one client address may be issued per day. Raise it for a shared
/// address such as an office network. `RAMBL_ANON_IDS_PER_IP`.
pub const DEFAULT_ANON_IDS_PER_IP: u32 = 20;
/// Sign-in attempts one client address may make per day. `RAMBL_SIGNINS_PER_IP`.
pub const DEFAULT_SIGNINS_PER_IP: u32 = 60;

// ---- request size limits

/// The longest text one capture may carry, in characters. Each clause of it is a Jev
/// question, and the whole capture counts as one use of the daily cap.
pub const CAPTURE_MAX_CHARS: usize = 20_000;
/// The most clauses one capture may split into, for the same reason.
pub const CAPTURE_MAX_CLAUSES: usize = 60;
/// The largest JSON import body, in bytes (2 MiB).
pub const IMPORT_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// The most tasks one JSON import may hold.
pub const IMPORT_MAX_TASKS: usize = 500;
/// The longest title a task or series may have, in characters.
pub const MAX_TITLE_CHARS: usize = 500;
/// The most tasks one person may hold. Every call loads and plans the whole list under
/// one lock, so an unbounded list lets one account slow everyone down.
pub const MAX_TASKS_PER_USER: usize = 5000;

// ---- the snapshot window

/// Days of series occurrences a snapshot carries when the caller names no window:
/// today plus this many.
pub const DEFAULT_WINDOW_DAYS: u32 = 7;
/// The widest window a caller may ask for, in days: a year, the horizon the scheduler
/// itself fills.
pub const MAX_WINDOW_DAYS: u32 = 366;

// ---- calendar import

/// The most occurrences a counted rule (`RRULE` with `COUNT`) is followed for, about
/// five years of a daily event. A larger count is clamped to this.
pub const IMPORT_MAX_RULE_COUNT: u32 = 1820;
/// The longest `DURATION` an imported event may have; anything longer is treated as
/// unreadable. Ten years.
pub const IMPORT_MAX_EVENT_LENGTH: chrono::Duration = chrono::Duration::days(3650);

// ---- storage and backups

/// How long a database call waits for another handle's write lock (the admin console
/// opens one) before failing. Safe range 1 to 30 seconds.
pub const DB_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Days of per-day usage counts kept for the admin page; older rows are dropped on
/// write.
pub const USAGE_RETENTION_DAYS: i64 = 30;
/// How often the backup runs.
pub const BACKUP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Days a daily backup is kept before it is deleted.
pub const BACKUP_RETENTION_DAYS: i64 = 14;

// ---- the admin console

/// Rows `usage` and the admin page list by default.
pub const ADMIN_TOP_ROWS: usize = 10;

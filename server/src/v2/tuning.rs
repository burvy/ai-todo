//! Every number the server can be tuned by, in one place. See `docs/TUNING.md`.
//!
//! Each value says what it does, its unit and a safe range. Values a deployment
//! overrides with an environment variable are the defaults for that variable, which
//! is named beside them. Task lengths are shared with the crates and live in
//! `todo_core::tuning`.

use std::time::Duration;

/// The port the server listens on. `RAMBL_PORT`, then `PORT`, override it. Any free
/// port from 1024 to 65535 works locally; the firewall and the Cloudflare tunnel on
/// the server machine expect this one, so change them with it.
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
/// `RAMBL_JEV_DAILY_CAP`. Safe range 20 to 2000: below it ordinary use hits the cap,
/// above it one account can run up the Jev bill.
pub const DEFAULT_JEV_DAILY_CAP: u32 = 200;
/// The same for a user on the paid plan. `RAMBL_JEV_DAILY_CAP_PAID`. Safe range
/// [`DEFAULT_JEV_DAILY_CAP`] to 10,000.
pub const DEFAULT_PAID_JEV_DAILY_CAP: u32 = 1000;
/// The same for an anonymous user. `RAMBL_JEV_DAILY_CAP_ANON`. Safe range 5 to
/// [`DEFAULT_JEV_DAILY_CAP`]: anyone can mint an anonymous id, so keep it low.
pub const DEFAULT_ANON_JEV_DAILY_CAP: u32 = 30;
/// Anonymous ids one client address may be issued per day. Raise it for a shared
/// address such as an office network. `RAMBL_ANON_IDS_PER_IP`. Safe range 5 to
/// 200; higher lets one address mint ids to dodge the anonymous cap.
pub const DEFAULT_ANON_IDS_PER_IP: u32 = 20;
/// Sign-in attempts one client address may make per day. `RAMBL_SIGNINS_PER_IP`.
/// Safe range 10 to 500; each attempt is a call to Google.
pub const DEFAULT_SIGNINS_PER_IP: u32 = 60;

// ---- request size limits

/// The longest text one capture may carry, in characters. Each clause of it is a Jev
/// question, and the whole capture counts as one use of the daily cap. Safe range
/// 2,000 to 50,000.
pub const CAPTURE_MAX_CHARS: usize = 20_000;
/// The most clauses one capture may split into, for the same reason. Safe range 10
/// to 200; each clause is one Jev request sent at the same time.
pub const CAPTURE_MAX_CLAUSES: usize = 60;
/// The largest JSON import body, in bytes (2 MiB). Safe range 256 KiB to 16 MiB; the
/// whole body is held in memory while it is read.
pub const IMPORT_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// The most tasks one JSON import may hold. Safe range 50 to
/// [`MAX_TASKS_PER_USER`]; more than that could never be stored.
pub const IMPORT_MAX_TASKS: usize = 500;
/// The longest title a task or series may have, in characters. Safe range 100 to
/// 2,000; the page is laid out for titles of a line or two.
pub const MAX_TITLE_CHARS: usize = 500;
/// The most tasks one person may hold. Every call loads and plans the whole list under
/// one lock, so an unbounded list lets one account slow everyone down. Safe range
/// 500 to 20,000.
pub const MAX_TASKS_PER_USER: usize = 5000;

// ---- the snapshot window

/// The widest window a caller may ask for, in days: a year, the horizon the scheduler
/// itself fills. Safe range 31 to 366; a wider window makes a bigger snapshot to
/// expand and send.
pub const MAX_WINDOW_DAYS: u32 = 366;

// ---- calendar import

/// The most occurrences a counted rule (`RRULE` with `COUNT`) is followed for, about
/// five years of a daily event. A larger count is clamped to this. Safe range 366
/// to 3650; each occurrence is expanded in memory on import.
pub const IMPORT_MAX_RULE_COUNT: u32 = 1820;
/// The largest `INTERVAL` an imported rule may have (weeks for a weekly rule, days for a
/// daily one); a rule with a larger one is imported as one task and reported. Safe range
/// 52 to 10000.
pub const IMPORT_MAX_INTERVAL: u32 = 1000;
/// The longest `DURATION` an imported event may have; anything longer is treated as
/// unreadable. Ten years. Safe range 1 to 10 years.
pub const IMPORT_MAX_EVENT_LENGTH: chrono::Duration = chrono::Duration::days(3650);

// ---- storage and backups

/// How long a database call waits for another handle's write lock (the admin console
/// opens one) before failing. Safe range 1 to 30 seconds.
pub const DB_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Days of per-day usage counts kept for the admin page; older rows are dropped on
/// write. Safe range 7 to 365; the admin page shows no more days than are kept.
pub const USAGE_RETENTION_DAYS: i64 = 30;
/// How often the backup runs. Safe range 6 to 48 hours: each run writes a full copy
/// of the database, and the anonymous cleanup runs right after it.
pub const BACKUP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Days a daily backup is kept before it is deleted. Safe range 2 to 90; each day
/// kept is a full copy of the database on disk.
pub const BACKUP_RETENTION_DAYS: i64 = 14;

// ---- the admin console

/// Rows `usage` and the admin page list by default. Safe range 5 to 100.
pub const ADMIN_TOP_ROWS: usize = 10;

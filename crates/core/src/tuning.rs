//! Values shared by more than one crate. See `docs/TUNING.md`.
//!
//! A value lives here only when two crates must agree on it; anything one crate uses
//! alone belongs in that crate's own `tuning.rs`.

use chrono::NaiveTime;

// ---- task length

/// The shortest a task may be, in minutes. Capture, calendar import and revisions
/// clamp to it, and the API refuses an edit below it. Safe range 1 to 15. The
/// scheduler plans every task for at least this long.
pub const TASK_MIN_DURATION_MIN: u16 = 5;

/// The longest a task may be, in minutes (8 hours), with the same clamping and
/// refusal as the minimum. Safe range 60 to 959: the scheduler places a task in one
/// unbroken stretch of the working day ([`WORKDAY_START`] to [`WORKDAY_END`]), so a
/// longer task can never be placed.
pub const TASK_MAX_DURATION_MIN: u16 = 480;

/// A task's length when nothing says how long it takes, in minutes: a capture with
/// no duration phrase, or a calendar event with no end. Must lie between
/// [`TASK_MIN_DURATION_MIN`] and [`TASK_MAX_DURATION_MIN`].
pub const TASK_DEFAULT_DURATION_MIN: u16 = 30;

const _: () = assert!(TASK_MIN_DURATION_MIN <= TASK_DEFAULT_DURATION_MIN);
const _: () = assert!(TASK_DEFAULT_DURATION_MIN <= TASK_MAX_DURATION_MIN);

// ---- the working day

/// When the planner may start placing flexible tasks each day, local time. Pinned
/// tasks are kept wherever they are; only tasks the planner places wait for this.
pub const WORKDAY_START: NaiveTime = NaiveTime::from_hms_opt(8, 0, 0).expect("08:00 is valid");

/// When the planner stops placing flexible tasks each day, local time: the last
/// second of the day, so evenings count as free time.
pub const WORKDAY_END: NaiveTime = NaiveTime::from_hms_opt(23, 59, 59).expect("23:59:59 is valid");

// ---- day-only tasks ("call mom Friday"): defaults for the per-user settings

/// When a task given a day but no time is due on that day, until the user changes it
/// in their settings. Never 00:00: that is the start of the day it should end.
pub const DEFAULT_DAY_DEADLINE: NaiveTime =
    NaiveTime::from_hms_opt(23, 59, 0).expect("23:59 is valid");

/// Where on its day such a task is placed when the user has no timed tasks of their
/// own to learn a usual time from, until the user changes it in their settings.
/// Should lie inside the working day.
pub const DEFAULT_DAY_PLACEMENT: NaiveTime =
    NaiveTime::from_hms_opt(12, 0, 0).expect("12:00 is valid");

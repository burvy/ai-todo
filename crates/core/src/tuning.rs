//! Values shared by more than one crate. See `docs/TUNING.md`.
//!
//! A value lives here only when two crates must agree on it; anything one crate uses
//! alone belongs in that crate's own `tuning.rs`.

/// The shortest a task may be, in minutes. Capture, calendar import and revisions
/// clamp to it, and the API refuses an edit below it. Safe range 1 to 15. The scheduler still
/// gives any task at least its own minimum block (5 minutes), so going lower only
/// changes what the task says, not the time it is planned for.
pub const TASK_MIN_DURATION_MIN: u16 = 5;

/// The longest a task may be, in minutes (8 hours), with the same clamping and
/// refusal as the minimum. Safe range 60 to 959: the scheduler places a task in one
/// unbroken stretch of the working day (08:00 to 23:59:59), so a longer task can
/// never be placed.
pub const TASK_MAX_DURATION_MIN: u16 = 480;

/// A task's length when nothing says how long it takes, in minutes: a capture with
/// no duration phrase, or a calendar event with no end. Must lie between
/// [`TASK_MIN_DURATION_MIN`] and [`TASK_MAX_DURATION_MIN`].
pub const TASK_DEFAULT_DURATION_MIN: u16 = 30;

const _: () = assert!(TASK_MIN_DURATION_MIN <= TASK_DEFAULT_DURATION_MIN);
const _: () = assert!(TASK_DEFAULT_DURATION_MIN <= TASK_MAX_DURATION_MIN);

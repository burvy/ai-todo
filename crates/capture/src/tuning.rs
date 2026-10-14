//! The thresholds capture decides with. See `docs/TUNING.md`. Task lengths are in
//! `todo_core::tuning`.

/// How sure Jev must be that a clause is a task before it becomes one, from 0.0 to
/// 1.0. Lower keeps more borderline clauses ("I should probably…"), higher drops more
/// real tasks. Safe range 0.3 to 0.7.
pub const IS_TASK_THRESHOLD: f64 = 0.5;

/// The most words a clause may have and still count as a time fragment ("by Friday at
/// 3") to be joined onto the clause before it rather than judged on its own. Safe
/// range 3 to 8; higher joins real tasks that happen to start with "by".
pub const TIME_FRAGMENT_MAX_WORDS: usize = 5;

/// In an email, a `From:` line starts a quoted earlier message only when its value is
/// longer than this many characters. "From: Jane" alone is usually a signature, while
/// a quoted header carries a full name and address. Safe range 2 to 10.
pub const QUOTE_HEADER_MIN_CHARS: usize = 3;

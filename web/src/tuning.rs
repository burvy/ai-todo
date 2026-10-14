//! Numbers the page's own widgets use. Values two crates must agree on live in
//! `todo_core::tuning` instead; see `docs/TUNING.md`.

/// The gap between the minutes the time picker offers. Safe range 1 to 30 and a
/// divisor of 60; the picker also keeps a task's own off-step minute on offer.
pub const MINUTE_STEP: u16 = 5;

/// Where the time picker's hands rest before any time is picked, in minutes into the
/// day (9:00): the first tap on any hour then names a whole time.
pub const TIME_PAGE_START: u16 = 9 * 60;

const _: () = assert!(60 % MINUTE_STEP == 0);

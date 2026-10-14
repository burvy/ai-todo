//! Numbers the page's own widgets use. Values two crates must agree on live in
//! `todo_core::tuning` instead; see `docs/TUNING.md`.

/// The gap between the minutes the time picker offers. Safe range 1 to 30 and a
/// divisor of 60; the picker also keeps a task's own off-step minute on offer.
pub const MINUTE_STEP: u16 = 5;

/// Where the time picker's hands rest before any time is picked, in minutes into the
/// day (9:00): the first tap on any hour then names a whole time.
pub const TIME_PAGE_START: u16 = 9 * 60;

/// The least room, in CSS pixels, between Rambo's ask bubble and either side of the
/// screen: near an edge the bubble slides in rather than run off it. Safe range 4 to
/// 24; it matches the page's `--space-2` gutter.
pub const BUBBLE_EDGE_PX: f64 = 8.0;

const _: () = assert!(60 % MINUTE_STEP == 0);

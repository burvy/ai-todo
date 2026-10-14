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

// ---- the scribble-out delete (DESIGN-DIRECTION §9, "Gestures, final")

/// Share of a row's width a swipe must cover for letting go to delete it; short of
/// it the ink lifts away and the row stays. Safe range 0.25 to 0.6: lower and a
/// sideways nudge deletes, higher and a phone's row needs a full-width drag.
pub const SCRIBBLE_COMMIT_SHARE: f64 = 0.4;
/// A flick at least this fast, in px per ms, deletes from `row::SWIPE_PX` of travel
/// on. Safe range 0.08 to 0.3.
pub const SCRIBBLE_FLICK_PX_PER_MS: f64 = 0.11;
/// The width of one loop of the scribble, in px. Safe range 8 to 24.
pub const SCRIBBLE_LOOP_PX: f64 = 14.0;
/// The band of a line of text the scribble's loops stay inside, as shares of the
/// line's height from its top: about the x-height, so it crosses the letters and not
/// the gap between rows. Safe range 0.2 to 0.9, the first below the second.
pub const SCRIBBLE_BAND: (f64, f64) = (0.35, 0.8);
/// How much each loop's width and height may wander, as a share of a loop, seeded by
/// the row's id: a hand's scribble, never a machine's sawtooth, and the same row
/// always scribbles the same. Safe range 0 to 0.45.
pub const SCRIBBLE_JITTER: f64 = 0.3;
/// How long a committed scribble takes to finish across the row, in ms. Safe range
/// 120 to 300: the system answering, so quick.
pub const SCRIBBLE_FINISH_MS: u32 = 180;
/// How long ink that was not enough takes to lift away, in ms. Safe range 150 to 300.
pub const SCRIBBLE_LIFT_MS: u32 = 200;

// ---- the scrap undo

/// How long a deleted row's scrap lies in the corner to be taped back, in ms. Safe
/// range 3000 to 10000: long enough to reach, short enough not to linger.
pub const SCRAP_UNDO_MS: u32 = 5000;
/// How long the scrap takes to slide in or away, in ms. Safe range 150 to 400.
pub const SCRAP_SLIDE_MS: u32 = 240;
/// How often the scrap's undo clock is read, in ms: it only runs while the page is
/// in view. Safe range 50 to 500; a hidden tab may tick slower, which only pauses.
pub const SCRAP_TICK_MS: u32 = 100;

const _: () = assert!(SCRIBBLE_BAND.0 < SCRIBBLE_BAND.1);

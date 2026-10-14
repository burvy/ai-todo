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

// ---- the page turn (web/src/bend.rs)

/// The phone layout's media query, the one `web/theme.css` switches the two panes into
/// a row on: the page bends only where the panes are pages. Keep it equal to the CSS.
pub const PHONE_MEDIA: &str = "(max-width: 700px)";
/// How far a press must travel before it is a swipe and not a tap, in px. Safe range 6
/// to 16: the same slop a task row uses (`row::SLOP_PX`), a little wider.
pub const BEND_SLOP_PX: f64 = 10.0;
/// Share of the page's width the finger must carry the corner for letting go to turn the
/// page; short of it the page springs back. Safe range 0.2 to 0.5.
pub const BEND_COMMIT_SHARE: f64 = 0.35;
/// A flick at least this fast, in px per ms, turns the page from `BEND_FLICK_MIN_PX` of
/// travel on. Safe range 0.2 to 0.6.
pub const BEND_FLICK_PX_PER_MS: f64 = 0.3;
/// The least travel, in px, that a flick can turn the page from. Safe range 16 to 48.
pub const BEND_FLICK_MIN_PX: f64 = 24.0;
/// How far the corner can be dragged, in page widths: just over two, the drag that
/// carries the fold clean across the page. Safe range 2.0 to 2.5.
pub const BEND_FULL_TURN: f64 = 2.05;
/// How long a released bend takes to turn the page the rest of the way, in ms. Safe range
/// 250 to 500: the system finishing what the finger started, so quick.
pub const BEND_COMPLETE_MS: u32 = 360;
/// How long a released bend takes to spring back flat, in ms. Safe range 200 to 400.
pub const BEND_SPRING_MS: u32 = 300;
/// How long a button's page turn takes, in ms: nobody is holding the page, so it is
/// unhurried. Safe range 400 to 700.
pub const BEND_BUTTON_MS: u32 = 520;
/// How high a button's turn lifts the corner on its way, as a share of the page's
/// height, so the fold crosses the page on a slant. Safe range 0 to 0.3.
pub const BEND_BUTTON_ARC: f64 = 0.12;
/// With reduced motion the page does not bend: it slides, and this is how long the slide
/// takes to finish, in ms. Safe range 100 to 200.
pub const BEND_SLIDE_MS: u32 = 150;

const _: () =
    assert!(BEND_FLICK_MIN_PX < 100.0 && BEND_COMMIT_SHARE < 1.0 && BEND_FULL_TURN >= 2.0);

// ---- Rambo writing a new row (UI-BUGS #11, v1's writing)

/// How often the ink and the nib move while a title is written, in ms: one frame,
/// so the ink never jumps a letter and the pen never looks stalled. Safe range 8
/// to 33.
pub const INK_TICK_MS: u32 = 16;
/// His flight between two spots (caret, row, home), in ms. Safe range 300 to 700.
pub const FLY_MS: u32 = 450;
/// Quiet time at the capture box after typing stops before he flies home, in ms.
/// Safe range 800 to 4000.
pub const LINGER_MS: u32 = 2000;
/// Extra width, in px of glyph advance, that the pen lift over a space costs: the
/// nib hops the gap and pauses before the next word. Safe range 0 to 24.
pub const WORD_GAP_W: f64 = 10.0;
/// Stroke time per glyph: the pace of his hand. Safe range 70 to 160 ms; v1 wrote at
/// about this pace, slower than typing so it reads as considered writing.
pub const WRITE_PER_GLYPH_MS: u32 = 110;
/// The shortest a title takes to write, so a short one still reads as writing.
/// Safe range 400 to 1200 ms.
pub const WRITE_MIN_MS: u32 = 700;
/// The longest a title takes to write, so a long one still finishes. Safe range
/// 1800 to 5000 ms.
pub const WRITE_MAX_MS: u32 = 3200;
/// The beat the nib rests on the line after landing, before the first stroke.
/// Safe range 0 to 300 ms.
pub const WRITE_SETTLE_MS: u32 = 140;
/// The hold after the last letter closes, before he flies home. Safe range 0 to
/// 500 ms.
pub const WRITE_FINISH_MS: u32 = 250;
/// How long a half-written row waits for him (held, typing elsewhere, or the row
/// scrolled away) before its rest of the title simply appears. Safe range 5000 to
/// 60000 ms.
pub const WRITE_HOLD_MAX_MS: u32 = 15_000;
/// How far past its edges the ink may show, in px, so a letter's flourish above
/// the line or past the last stroke is not cut. Safe range 0 to 4.
pub const INK_BLEED_PX: f64 = 2.0;

const _: () = assert!(WRITE_MIN_MS < WRITE_MAX_MS);
// A title that is merely slow to write is never cut short as "waited too long".
const _: () = assert!(WRITE_MAX_MS < WRITE_HOLD_MAX_MS);

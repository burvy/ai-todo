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

// ---- Rambo's ink (web/src/ink.rs, inklayer.rs, interview 4 item 2). Speeds in px
// per ms, times in ms.

/// How far apart the dabs of a stroke lie, in px: closer than the narrowest dab is
/// wide (a ballpoint's quick, tapered end is about 1.1 px), so the line never shows
/// dots. Safe range 0.5 to 1.0.
pub const INK_STEP_PX: f64 = 1.0;
/// Up to this speed a stroke is the nib's full width. Safe range 0.1 to 0.6.
pub const INK_SLOW_SPEED: f64 = 0.3;
/// From this speed on a stroke is as thin as it gets. Safe range 1 to 4, above
/// `INK_SLOW_SPEED`.
pub const INK_FAST_SPEED: f64 = 2.0;
/// The width of the quickest stroke, as a share of the nib's. Safe range 0.35 to 0.9.
pub const INK_FAST_SHARE: f64 = 0.55;
/// How quickly the width follows a change of speed, per dab, 0 to 1. Safe range
/// 0.05 to 0.4: higher and the line steps where the hand speeds up.
pub const INK_WIDTH_EASE: f64 = 0.15;
/// How far along a long stroke the nib lands and lifts (the taper), in px. Safe
/// range 4 to 24.
pub const INK_TAPER_PX: f64 = 10.0;
/// The end of a stroke in Rambo's own tint, in px. Safe range 6 to 30.
pub const INK_TAIL_PX: f64 = 20.0;
/// How long fresh ink takes to dry, in ms, and how much of it shows once dry.
/// Safe ranges 300 to 2000 ms and 0.6 to 0.95.
pub const INK_DRY_MS: f64 = 900.0;
pub const INK_DRY_ALPHA: f64 = 0.75;
/// The thinnest a stroke gets, as a dab's radius in px: a nib's hairline. It keeps
/// the quickest cross-stroke wider than the dab spacing, so it never breaks into
/// dots. Safe range 0.5 to 1.0, above half of `INK_STEP_PX`.
pub const INK_HAIRLINE_PX: f64 = 0.6;
/// The slant of a broad nib, in degrees from the page's horizontal: the hand's
/// natural angle. A broad nib draws thick across it and thin along it. Safe range
/// 25 to 55.
pub const INK_NIB_ANGLE_DEG: f64 = 40.0;
/// How long ink stays on the page before it starts to fade, and how long the fade
/// takes, in ms. Safe ranges 3000 to 20000 and 1000 to 5000.
pub const INK_HOLD_MS: f64 = 6000.0;
pub const INK_FADE_OUT_MS: f64 = 2400.0;
/// How much earlier the thinnest ink starts to fade than ink at the nib's full
/// width, in ms. Safe range 0 to 1500.
pub const INK_FADE_THIN_LEAD_MS: f64 = 800.0;

const _: () = assert!(
    INK_SLOW_SPEED < INK_FAST_SPEED
        && INK_DRY_MS < INK_HOLD_MS - INK_FADE_THIN_LEAD_MS
        && INK_STEP_PX <= 1.0
        && INK_HAIRLINE_PX * 2.0 > INK_STEP_PX
);

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
/// The most scraps lying in the corner at once; a new delete beyond it sends the oldest
/// away. Safe range 1 to 5: two quick deletes keep two, but the pile never fills the page.
pub const SCRAP_MAX: usize = 3;
/// The most done tasks hanging at the pin at once, newest first; the older ones stay done
/// (and in the planner's history) but are not drawn, so the column never runs past the page.
/// Safe range 3 to 10.
pub const PIN_SHOWN: usize = 6;
/// How often the scrap's undo clock is read, in ms: it only runs while the page is
/// in view. Safe range 50 to 500; a hidden tab may tick slower, which only pauses.
pub const SCRAP_TICK_MS: u32 = 100;

const _: () = assert!(SCRIBBLE_BAND.0 < SCRIBBLE_BAND.1 && SCRAP_MAX >= 1);

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
/// height, on top of `BEND_LIFT`. Safe range 0 to 0.3.
pub const BEND_BUTTON_ARC: f64 = 0.1;
/// How far the corner rises for each px it travels sideways, so the fold leans (about
/// 27 degrees at 0.5) and the flap grows out of the corner like a page turning, not a
/// sheet folded in half. Safe range 0.3 to 0.8.
pub const BEND_LIFT: f64 = 0.5;
/// The most the corner rises, as a share of the page's height. Safe range 0.3 to 0.5.
pub const BEND_LIFT_CAP: f64 = 0.4;
/// How far the page rebounds after it falls flat on a spring-back, as a share of the
/// distance it was held. Safe range 0.03 to 0.1.
pub const BEND_BOUNCE: f64 = 0.06;
/// The share of a spring-back at which the page first lies flat, before it rebounds.
/// Safe range 0.6 to 0.8.
pub const BEND_BOUNCE_AT: f64 = 0.7;
/// The lift for a button's turn, which has no finger to set the path: far lower than
/// `BEND_LIFT`, so the fold stays steep (never more than about 40 degrees off vertical)
/// and the page leaves to the side instead of rolling up from the bottom.
pub const BEND_BUTTON_LIFT: f64 = 0.15;
/// The most a button's turn lifts the corner, as a share of the page's height.
pub const BEND_BUTTON_LIFT_CAP: f64 = 0.15;
/// How wide the curl at the fold is, in px, for each px the corner has been dragged: a
/// tight roll at the start of a turn, a looser one past halfway. Safe range 0.08 to 0.2.
pub const BEND_ROLL_PER_PX: f64 = 0.12;
/// The tightest and the loosest the curl gets, in px. Safe range 8 to 16 and 30 to 48.
pub const BEND_ROLL_MIN_PX: f64 = 12.0;
pub const BEND_ROLL_MAX_PX: f64 = 40.0;
/// How far, in px, the flap's soft shadow is cast onto the page below: a near and a far
/// copy of the flap, drawn instead of a per-frame blur. Safe range 2 to 10 and 8 to 20.
pub const BEND_SHADOW_NEAR_PX: f64 = 4.0;
pub const BEND_SHADOW_FAR_PX: f64 = 11.0;
/// With reduced motion the page does not bend: it slides, and this is how long the slide
/// takes to finish, in ms. Safe range 100 to 200.
pub const BEND_SLIDE_MS: u32 = 150;

const _: () = assert!(
    BEND_FLICK_MIN_PX < 100.0
        && BEND_COMMIT_SHARE < 1.0
        && BEND_FULL_TURN >= 2.0
        && BEND_BOUNCE_AT < 1.0
        && BEND_SHADOW_NEAR_PX < BEND_SHADOW_FAR_PX
);

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
/// Stroke time per glyph: the pace of his hand. Safe range 90 to 180 ms: slower than
/// typing, so it reads as considered writing.
pub const WRITE_PER_GLYPH_MS: u32 = 130;
/// The shortest a title takes to write, so a short one still reads as writing.
/// Safe range 400 to 1200 ms.
pub const WRITE_MIN_MS: u32 = 700;
/// The longest a title takes to write, so a long one still finishes. Safe range
/// 3000 to 6000 ms: lower and a long title is rushed at half a short one's pace.
pub const WRITE_MAX_MS: u32 = 4500;
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
/// How long a glyph's ink takes to appear once the nib passes its middle, in ms. Ink
/// is the one thing that fades in. Safe range 40 to 160.
pub const INK_FADE_MS: u32 = 90;
/// How far the nib presses into the paper at the middle of each glyph's stroke, in
/// px. Safe range 0 to 3.
pub const WRITE_DIP_PX: f64 = 1.5;
/// How far the pen lifts off the paper over a word gap, in px. Safe range 0 to 4.
pub const WRITE_LIFT_PX: f64 = 2.0;
/// How far his lean wanders from glyph to glyph, either way, in degrees. Safe range
/// 0 to 4.
pub const WRITE_WOBBLE_DEG: f64 = 2.0;
/// The dip as he lands on the line, in px, and how long it takes to settle, in ms.
/// Safe ranges 0 to 4 px and 30 to 120 ms; the settle fits in `WRITE_SETTLE_MS`.
pub const WRITE_TOUCHDOWN_PX: f64 = 2.0;
pub const WRITE_TOUCHDOWN_MS: u32 = 60;
/// The flick as the last letter closes: how high it lifts in px, how far it turns
/// in degrees, and how long it takes to go up and settle in ms, before
/// `WRITE_FINISH_MS`. Safe ranges 0 to 4 px, 0 to 8 degrees and 60 to 200 ms.
pub const WRITE_FLICK_PX: f64 = 2.0;
pub const WRITE_FLICK_DEG: f64 = 4.0;
pub const WRITE_FLICK_MS: u32 = 120;

const _: () = assert!(WRITE_MIN_MS < WRITE_MAX_MS);
const _: () = assert!(WRITE_TOUCHDOWN_MS <= WRITE_SETTLE_MS);
// A title that is merely slow to write is never cut short as "waited too long".
const _: () = assert!(WRITE_MAX_MS < WRITE_HOLD_MAX_MS);

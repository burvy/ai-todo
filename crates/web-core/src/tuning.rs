//! Numbers the page's own widgets use. Values two crates must agree on live in
//! `todo_core::tuning` instead; see `docs/TUNING.md`.

/// How long a task made by hand on the + sheet lasts until you say otherwise, in
/// minutes. Safe range 5 to 120.
pub const NEW_TASK_MINUTES: u16 = 30;

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
/// How long Rambo keeps saying why a change was not saved, in ms, unless another
/// refusal takes its place. Safe range 4000 to 12000: shorter and a two-clause note is
/// gone before it is read, longer and it hides what he says next.
pub const REFUSAL_SAY_MS: f64 = 7000.0;
/// How long a slip taken off the planner takes to slide out of its day, in ms; the
/// week is drawn without it no sooner. Safe range 180 to 400: shorter and it pops,
/// longer and the planner waits on it.
pub const SLIP_OFF_MS: u32 = 260;
/// How long a phone finger must hold still on a slip before it lifts onto the finger,
/// in ms, so a plain drag still scrolls the paper (a mouse lifts at once). Safe range
/// 150 to 500: shorter and a scroll lifts slips, longer and a lift feels stuck.
pub const PRESS_HOLD_MS: u32 = 250;
/// How far, in px in all, a finger that has not lifted its slip may slide and still be
/// holding: past it the press is a scroll. Safe range 6 to 16: a resting finger on a
/// phone shakes a few pixels.
pub const PRESS_SLOP_PX: f64 = 10.0;
/// How long an empty planner hour must be held before the capture box is filled with
/// it, in ms. Safe range 450 to 900: shorter and a slow tap fills it, longer and the
/// hold seems ignored.
pub const LONG_PRESS_MS: u32 = 600;

/// How loud the background music plays, 0 to 1. Safe range 0.05 to 0.25: the music is
/// a bed under the cues (whose own volumes are 0.2 to 0.42), not a track to listen to.
pub const MUSIC_VOLUME: f64 = 0.12;
/// How quickly the sheet of the other paper comes down over the page, and goes back up,
/// when the desk lamp (or the gear's Light and Dark) switches the paper, in radians per
/// second: a critically damped spring (`engine/glide.rs`) lands in about 6 over this, so
/// 18 is about a third of a second each way. The paper is swapped when it lands. Safe
/// range 10 to 30: slower and the lamp feels sluggish, faster and the swap reads as a flash.
pub const LAMP_SHEET_OMEGA: f64 = 18.0;
/// How long the lamp's cord stays pulled down after a tap, in ms. Safe range 150 to 400.
pub const LAMP_PULL_MS: u32 = 240;
/// How far the lamp's cord drops when pulled, in px, and the spring that takes it there and
/// back: its quickness in radians per second and its damping (below 1 it swings past the
/// end and back, like a cord let go). Safe ranges 3 to 10 px, 25 to 60 and 0.3 to 0.8.
pub const LAMP_CORD_DROP_PX: f64 = 6.0;
pub const LAMP_CORD_OMEGA: f64 = 45.0;
pub const LAMP_CORD_ZETA: f64 = 0.45;
/// How quickly a strip flies from its row to its pin, in radians per second, and how
/// damped (below 1 it swings a little past the pin, like a paper dropped on a table).
/// Safe ranges 10 to 30 and 0.6 to 1.0: a lower damping swings long past the pin.
pub const SCRAP_FLY_OMEGA: f64 = 16.0;
pub const SCRAP_FLY_ZETA: f64 = 0.8;

// Rambo's dance to that music (`engine/dance.rs`): he hears each note's attack in how
// loud the music is, and every beat kicks his sway and bounces the nest.

/// How quickly he follows the music's loudness, and how slowly he remembers it, in ms: a
/// note's attack is the quick one rising above the slow one. Safe ranges 15 to 60 and
/// 250 to 800.
pub const DANCE_FAST_MS: f64 = 30.0;
pub const DANCE_SLOW_MS: f64 = 400.0;
/// How far above lately's loudness a sound must rise to be a beat. Safe range 1.2 to 2.5:
/// lower and he twitches at every swell, higher and only the loudest chords move him. At
/// 1.35 the Nocturne (the first track) moves him about every 1.6 s; at 1.5, half as often.
pub const DANCE_ONSET_RATIO: f64 = 1.35;
/// Softer than this is silence to him (loudness, 0 to 1). Safe range 0.001 to 0.01.
pub const DANCE_FLOOR: f64 = 0.003;
/// The shortest gap between two beats he moves to, in ms. Safe range 180 to 500.
pub const DANCE_MIN_GAP_MS: f64 = 250.0;
/// His sway: how stiff (rad/s) and how damped (a share of critical), how hard a full
/// beat kicks it (degrees/s), and the most it leans (degrees). Loose and slow, so the
/// swing carries from one note to the next and the notes swell it: a period of about
/// 2 s that takes about 3 s to die away. Safe ranges 2 to 5, 0.08 to 0.3, 8 to 30 and
/// 3 to 10.
pub const DANCE_OMEGA: f64 = 3.0;
pub const DANCE_DAMPING: f64 = 0.12;
pub const DANCE_LEAN_KICK: f64 = 15.0;
pub const DANCE_LEAN_MAX_DEG: f64 = 6.0;
/// How hard a full beat bounces the nest under him, in px/s of sag: he bobs on it, a
/// few px on a phone. Safe range 30 to 90.
pub const DANCE_BOUNCE_KICK: f64 = 60.0;
/// How often a sleeping Rambo checks whether the music has started, in ms. Safe range
/// 250 to 1000.
pub const DANCE_LISTEN_MS: f64 = 500.0;

const _: () = assert!(60 % MINUTE_STEP == 0);

// ---- What a finger means (web/src/gesture.rs, VISION-AUDIT item 3): one slop and
// one hold time for every surface. Times in ms, speeds in px per ms.

/// Travel that turns a press into a drag. Safe range 4 to 16: a resting finger on
/// a phone shakes a few pixels, and past 16 a short drag feels ignored.
pub const GESTURE_SLOP_PX: f64 = 8.0;
/// How long a still finger stays down before the press is a hold. Safe range 200
/// to 600: shorter and a slow tap holds, longer and a hold feels like a wait.
pub const GESTURE_HOLD_MS: f64 = 280.0;
/// How far back the release speed is read from the finger's last moves. Safe range
/// 40 to 120: shorter and one jittery move decides it, longer and a flick that
/// slowed at the end still throws.
pub const GESTURE_RELEASE_SAMPLE_MS: f64 = 80.0;
/// How long a finger may rest between its last move and letting go and still throw.
/// Safe range 24 to 50: shorter and a slow pointer's frame gap reads as a rest,
/// longer and a flick that stopped still throws.
pub const GESTURE_RELEASE_REST_MS: f64 = 32.0;
/// The fastest throw, however wild the flick. Safe range 2 to 6.
pub const GESTURE_RELEASE_MAX_SPEED: f64 = 4.0;
/// The least release speed that throws; slower, what was carried is put down.
/// Safe range 0.3 to 1.0: lower and a careful drop drifts off.
pub const GESTURE_FLING_MIN_SPEED: f64 = 0.6;

const _: () = assert!(GESTURE_FLING_MIN_SPEED < GESTURE_RELEASE_MAX_SPEED);
const _: () = assert!(GESTURE_RELEASE_REST_MS < GESTURE_RELEASE_SAMPLE_MS);

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
/// How long the bookmark ribbon's slide back to today takes (`ribbon.rs`), in ms. Safe
/// range 250 to 700 (shorter jumps, longer drags).
pub const RIBBON_GLIDE_MS: f64 = 450.0;
/// How long fresh ink takes to dry, in ms, and how much of it shows once dry.
/// Safe ranges 300 to 2000 ms and 0.6 to 0.95.
pub const INK_DRY_MS: f64 = 900.0;
pub const INK_DRY_ALPHA: f64 = 0.75;
/// How long a row Rambo has just written stays wet: glossy, and smudged by a tap
/// (`smudge.rs`), in ms. Safe range 2000 to 8000: shorter and nobody catches it wet,
/// longer and a tap meant to open the row smudges it instead.
pub const INK_WET_MS: f64 = 4000.0;
/// How far a fingertip drags wet ink, in px, and in how many blots. Safe ranges 8 to
/// 24 px (past that it reads as a scribble) and 4 to 10 blots.
pub const SMUDGE_REACH_PX: f64 = 14.0;
pub const SMUDGE_BLOBS: u32 = 7;
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
/// How quickly a row slides, in radians per second (a critically damped spring, so about 6
/// over this seconds): the rows below a removed one closing the gap, and a row coming back
/// from the pin. 22 is about a quarter of a second. Safe range 12 to 35.
pub const ROW_SLIDE_OMEGA: f64 = 22.0;
/// How quickly a dragged strip swings back to the pin when it is let go anywhere but the
/// bin, in radians per second (critically damped, so no bounce). 20 is about a third of a
/// second. Safe range 12 to 35.
pub const STRIP_RELEASE_OMEGA: f64 = 20.0;
/// How quickly a scrap whose time is up slides off the page's edge, in radians per second
/// (critically damped). 25 carries it most of the way in `SCRAP_SLIDE_MS`, the time it is
/// kept before it is removed. Safe range 15 to 40.
pub const SCRAP_LEAVE_OMEGA: f64 = 25.0;
/// How quickly a strip slides forward in the stack when the one above it is taped back or
/// gone, in radians per second (critically damped). 20 is about a third of a second. Safe
/// range 12 to 35.
pub const STRIP_STACK_OMEGA: f64 = 20.0;
/// How quickly paper slides, in radians per second (critically damped): the ribbon's
/// head hopping to another day, a stretched slip growing or shrinking, the placing note,
/// the edit sheet and Rambo's ask bubble coming and going. 24 is about a quarter of a
/// second. Safe range 14 to 40.
pub const PAPER_SLIDE_OMEGA: f64 = 24.0;
/// How near its end, in px, sliding paper (the ribbon's head, a stretched slip) must come
/// to count as there. Safe range 0.05 to 0.5: under a pixel, so the last step never shows.
pub const SLIDE_NEAR_PX: f64 = 0.1;
/// How near its end how far paper is in sight (0 out of sight, 1 in it: the placing note,
/// the edit sheet) must come to count as there. Safe range 0.0005 to 0.005: a sheet the
/// height of the screen is then under a few px from its end.
pub const SHOWN_NEAR: f64 = 0.001;
/// The done tasks hang at the pin as a stack: the newest on top, the rest under it. This many
/// edges of the older ones show below the top strip, a few px each, to say how deep the stack
/// is; the rest sit behind the last edge. Safe range 1 to 5.
pub const PIN_EDGES: usize = 3;
/// How tall a strip at the pin grows, in px. Safe range 80 to 140.
pub const STRIP_H_PX: f64 = 110.0;
/// The gap between hanging strips, in px. Safe range 2 to 12.
pub const STRIP_GAP_PX: f64 = 4.0;
/// How far, in px, each strip of the pin's stack sits below the one above it. Safe range 2 to 8.
pub const PIN_EDGE_PX: f64 = 4.0;
/// How far, in px, a strip must be dragged before it counts as a drag and not a tap.
/// Safe range 4 to 12.
pub const STRIP_DRAG_PX: f64 = 6.0;
/// The bottom-left corner that deletes a dragged strip, as a share of the screen's width
/// and of its height. Safe range 0.2 to 0.4.
pub const BIN_CORNER_FRACTION: f64 = 0.3;
/// How long a strip crumples in the bin corner before the task is deleted, in ms.
/// Safe range 200 to 600.
pub const BIN_CRUMPLE_MS: u64 = 360;
/// What lies above the first strip in the pin's column (the pin itself), in px.
/// Safe range 20 to 40.
pub const PIN_ROOM_PX: f64 = 28.0;
/// How often the scrap's undo clock is read, in ms: it only runs while the page is
/// in view. Safe range 50 to 500; a hidden tab may tick slower, which only pauses.
pub const SCRAP_TICK_MS: u32 = 100;

const _: () = assert!(SCRIBBLE_BAND.0 < SCRIBBLE_BAND.1 && SCRAP_MAX >= 1);

// ---- the nest

/// Where Rambo rests in the nest's box, as shares of its width and height from its top
/// left: the middle of the bowl, a little above the middle of the box. The box is the
/// `.nest` button (web/theme.css); the back layer, the front rim and his home are all read
/// from it, so there is one nest and nothing to drift apart.
pub const NEST_HOME: (f64, f64) = (0.5, 0.40);

// ---- the page turn (web/src/flip.rs)

/// The phone layout's media query, the one `web/theme.css` switches the two panes into
/// stacked pages on: the page turns only where the panes are pages. Keep it equal to the CSS.
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
/// How long a button's page turn takes, in ms: nobody is holding the page, so it is
/// unhurried. Safe range 400 to 700.
pub const BEND_BUTTON_MS: u32 = 520;
/// How far in front of the spine the viewer is for the book flip, in page widths: the
/// nearer the eye, the more the free edge grows as it swings toward it. Safe range 2 to 6.
pub const FLIP_PERSPECTIVE: f64 = 5.0;
/// How quickly a released flip finishes, in radians per second (the spring's natural
/// frequency): the system finishing what a finger started, so quick. Safe range 10 to 20.
pub const FLIP_FINISH_OMEGA: f64 = 14.0;
/// How quickly a page dropped short of the turn falls back, and how loosely (damping 1 is
/// no rebound; lower swings). Safe range 12 to 20, and 0.4 to 0.8.
pub const FLIP_FALL_OMEGA: f64 = 16.0;
pub const FLIP_FALL_DAMPING: f64 = 0.55;
/// How hard a Rambo caught under the leaf pushes its corner up, in `open` per second
/// squared: it must beat the leaf's own pull (`FLIP_FALL_OMEGA` squared, about 256 per
/// unit of `open`) to lift it at all. Safe range 100 to 300: lower and he cannot lift it,
/// higher and the leaf pops up like a lid.
pub const COVERED_PUSH_ACCEL: f64 = 160.0;
/// How far his push can open the leaf, in `open`: a corner lifted a little (about 28
/// degrees), never a turn. A leaf closing from beyond it is what catches him; one falling
/// back from a push is not. Safe range 0.05 to 0.3.
pub const COVERED_LIFT_REACH: f64 = 0.15;
/// How much of its speed the leaf keeps when it meets the table or the spine. Safe range
/// 0.4 to 0.8.
pub const FLIP_WALL_BOUNCE: f64 = 0.65;

const _: () = assert!(BEND_FLICK_MIN_PX < 100.0 && BEND_COMMIT_SHARE < 1.0);

// ---- How Rambo moves (Interview 4: he hops, never glides; held, he rides above
// the finger). The hops are `hop`'s, drawn by `rambo.rs` until the engine drives
// them.

/// One hop: crouch, spring, land and settle, in ms. Brisk: a longer way is more
/// hops, never slower ones. Safe range 110 to 200.
pub const HOP_MS: u32 = 150;
/// How far one hop carries him, in px; the way is split into this many-px hops,
/// rounded up. Safe range 100 to 300.
pub const HOP_SPAN_PX: f64 = 180.0;
/// The most hops one move takes, however far: the notice beat and the whole run
/// fit in `FLY_MS`. Safe range 1 to 3.
pub const HOP_MAX: u32 = 2;
/// Moves shorter than this, in px, are a step, not a hop: the caret moving a
/// letter, a nudge. Safe range 4 to 30.
pub const HOP_MIN_PX: f64 = 12.0;
/// How high a short hop rises, as a share of its length: a small hop is a small
/// arc. Safe range 0.15 to 0.6.
pub const HOP_RISE: f64 = 0.35;
/// The height, in px, where a hop stops rising at `HOP_RISE`: past it a hop's
/// height still grows, by `HOP_LONG_RISE` of each extra px, so a longer hop is
/// always higher, never stuck at one height. Safe range 8 to 40.
pub const HOP_HEIGHT_PX: f64 = 22.0;
/// How much taller a long hop gets per extra px of its length: lower than a short
/// hop's round arc, but never nothing, so a long trip is a higher arc, never the
/// glide Interview 4 removed. Safe range 0.05 to 0.12.
pub const HOP_LONG_RISE: f64 = 0.072;
/// The highest any hop rises above the straight way, in px, however long: two hops
/// across a desk stay a pen's hops, not a ball's. Safe range 32 to 80.
pub const HOP_HEIGHT_MAX_PX: f64 = 48.0;
/// How much he squashes crouching and landing, and stretches in the air, as a
/// share of his size. Safe range 0 to 0.25.
pub const HOP_SQUASH: f64 = 0.14;

/// The beat before he sets off anywhere (DESIGN-DIRECTION §9, "Rambo reacts before
/// he moves"): he notices, his face changes and he perks up, then he hops
/// (`mascot::notice`), in ms. Safe range 120 to 250: shorter and the face never
/// registers; longer leaves too little of `FLY_MS` for the hops, and a capture
/// waits out `FLY_MS`, so it must never grow to fit the beat.
pub const NOTICE_BEAT_MS: u32 = 150;
/// How much he straightens up in that beat, as a share of his size: a stretch, the
/// opposite of a hop's squash. Safe range 0 to 0.12.
pub const NOTICE_PERK: f64 = 0.06;

/// How hard the ride spring pulls him after the finger that holds him, in rad/s: it is
/// critically damped, so he trails the finger by about 1/RIDE_OMEGA s and never swings
/// past it. Safe range 25 to 70: lower and he drags behind like a balloon, higher and
/// he is glued to the finger with no weight at all.
pub const RIDE_OMEGA: f64 = 40.0;

/// How long he takes to ease from one pose into the next when his state changes, in
/// ms (`engine::look`), eased in and out so no frame jumps. Safe range 100 to 240:
/// shorter and the change snaps, longer and he still leans the old way after he has
/// set off.
pub const POSE_BLEND_MS: f64 = 160.0;
/// The same into or out of the writing pose, which turns him by the theme's whole
/// `--write-lean` (34 degrees): longer, so no frame turns him by more than 2 or 3
/// degrees. Safe range 300 to 600.
pub const WRITE_BLEND_MS: f64 = 400.0;
// The longest blend is the writing pose's: the loop sleeps once it has passed.
const _: () = assert!(POSE_BLEND_MS < WRITE_BLEND_MS);
/// How far he leans into where he is going, in degrees per px/s of sideways speed: a
/// hop runs at up to about 6000 px/s. Safe range 0.0005 to 0.002: lower and a fast hop
/// stays upright, higher and every hop pins him at `LEAN_MAX_DEG`.
pub const LEAN_PER_PX_S: f64 = 0.001;
/// How long his lean takes to follow his speed, in ms: a hop's speed rises and falls
/// within it, and the lean eases after it instead of snapping. Safe range 50 to 200.
pub const LEAN_EASE_MS: f64 = 100.0;
/// The most he leans into a move, in degrees. Safe range 4 to 12: past that he reads
/// as falling, not running.
pub const LEAN_MAX_DEG: f64 = 8.0;
// Rambo thrown (`engine/fling.rs`, SPEC-ANSWERS item 31). Speeds in px per ms, as the
// finger's release speed is read (`gesture::release_velocity`), accelerations in px
// per ms².

/// How hard the paper slows him: a steady loss of speed against his motion. Safe range
/// 0.002 to 0.008: a 2 px/ms throw slides about 500 px at 0.004.
pub const FLING_FRICTION: f64 = 0.004;
/// The pull toward the bottom of the screen while he slides, so a throw curves down.
/// Safe range 0 to 0.003; it must stay below `FLING_FRICTION`, or he would creep down
/// while lying still.
pub const FLING_GRAVITY: f64 = 0.0015;
/// The share of his speed an edge gives back. Safe range 0.2 to 0.7.
pub const FLING_BOUNCE: f64 = 0.45;
/// Below this speed he has stopped sliding. Safe range 0.01 to 0.1.
pub const FLING_REST_SPEED: f64 = 0.03;
/// How fast he must hit an edge to splat, and only then is he dazed. Safe range 0.8 to
/// 2.5: lower and every bounce dazes him, higher and only a wild flick does.
pub const FLING_SPLAT_SPEED: f64 = 1.2;
/// How fast the paper he lies on may move, in px per ms, before he loses his grip on it:
/// slower, he rides along on the scrolling page; faster, he is flung with it, as a throw
/// at the paper's speed (Interview 1 item 2). Safe range 0.4 to 1.5: lower, an ordinary
/// scroll sweeps him off; higher, only the wildest flick of the page ever does.
pub const SWEEP_GRIP_PX_MS: f64 = 0.8;
/// The strokes in a splat, and how far the longest reaches at the least splatting
/// speed, in px (a harder hit reaches up to twice as far). Safe ranges 4 to 12 and 12
/// to 48.
pub const FLING_SPLAT_STROKES: u8 = 7;
pub const FLING_SPLAT_REACH_PX: f64 = 26.0;

// The nest (`engine/nest.rs`, SPEC-ANSWERS items 15-17): a soft dish on two damped
// springs, its sag in px and its rock in degrees.

/// How stiff the dish is, in rad/s. Safe range 7 to 16: softer and it wallows,
/// stiffer and it only twitches (16 read as no give at all: QA round 3).
pub const NEST_OMEGA: f64 = 10.0;
/// How quickly its swing dies, as a share of critical damping. Safe range 0.2 to 0.6:
/// lower and it wobbles on, higher and it never bounces.
pub const NEST_DAMPING: f64 = 0.25;
/// The deepest it sags, in px, and the most it rocks, in degrees, however hard he
/// lands. Safe ranges 3 to 10 and 2 to 6.
pub const NEST_SAG_MAX_PX: f64 = 10.0;
pub const NEST_ROCK_MAX_DEG: f64 = 4.0;
/// How much the dish gives under him: px/s of sag for each px/s he is still falling (a
/// hop's last drop, a throw's speed) as he comes down in it, so a hard landing sags it
/// deep and a soft one a little. Safe range 0.06 to 0.2.
pub const NEST_IMPACT_SAG: f64 = 0.12;
/// The least a landing sags it, in px/s, however softly he settles. Safe range 20 to 60.
pub const NEST_LAND_KICK: f64 = 40.0;
/// How far a landing off its middle tips it, in degrees per px off. Safe range 0.02 to
/// 0.1.
pub const NEST_ROCK_PER_PX: f64 = 0.05;
/// How hard it rocks as he leaves it or a throw comes in sideways, in degrees/s. Safe
/// range 15 to 50.
pub const NEST_LEAVE_KICK: f64 = 30.0;
/// How far up and left of the nest's middle a throw is still caught by it, in px: the
/// nest fills the bottom-right corner, and this is the reach of its catch there. Safe
/// range 40 to 120.
pub const NEST_CATCH_PX: f64 = 70.0;

/// After this much writing, in ms of writing rows and at the caret, his nib has run dry:
/// home again, he taps it on the nest's rim (Interview 3, idea 2). Safe range 6000 to
/// 30000: lower and he taps after every few tasks, higher and hardly ever.
pub const DRY_NIB_AFTER_MS: f64 = 12_000.0;
/// How long he sits in the nest before the first tap, in ms. Safe range 150 to 600.
pub const DRY_NIB_PAUSE_MS: f64 = 300.0;
/// How many times he taps, and how far apart, in ms. Safe ranges 2 to 4 and 120 to 260.
pub const DRY_NIB_TAPS: u32 = 3;
pub const DRY_NIB_TAP_MS: f64 = 180.0;
/// How far he tips toward the rim while he taps, in degrees (negative: his top toward the
/// page). Safe range -20 to -6.
pub const DRY_NIB_LEAN_DEG: f64 = -12.0;
/// How hard each tap knocks the nest, in px/s of sag. Safe range 15 to 50.
pub const DRY_NIB_TAP_KICK: f64 = 35.0;
/// How hard each tap rocks the nest toward the side he taps, in degrees/s. Safe range 4
/// to 15: a tap is a knock, smaller than his leaving it.
pub const DRY_NIB_TAP_ROCK: f64 = 9.0;

const _: () = assert!(
    FLING_GRAVITY < FLING_FRICTION
        && FLING_REST_SPEED < GESTURE_FLING_MIN_SPEED
        && GESTURE_FLING_MIN_SPEED < FLING_SPLAT_SPEED
        && FLING_SPLAT_SPEED < GESTURE_RELEASE_MAX_SPEED
);

/// How far above a finger he rides while held, in px between the finger and his
/// bottom edge: a fingertip covers more than its point. Safe range 8 to 48.
pub const HELD_GAP_TOUCH_PX: f64 = 20.0;
/// The same above a mouse or pen cursor: the grabbing hand covers a little around
/// its point. Safe range 0 to 24.
pub const HELD_GAP_POINTER_PX: f64 = 12.0;

// The beat and the longest run of hops fit in one flight, which the writing waits out.
const _: () = assert!(NOTICE_BEAT_MS + HOP_MS * HOP_MAX <= FLY_MS);
const _: () = assert!(HOP_MIN_PX < HOP_SPAN_PX);
const _: () = assert!(HOP_HEIGHT_PX <= HOP_HEIGHT_MAX_PX);

// ---- Rambo writing a new row (UI-BUGS #11, v1's writing)

/// How often the ink and the nib move while a title is written, in ms: one frame,
/// so the ink never jumps a letter and the pen never looks stalled. Safe range 8
/// to 33.
pub const INK_TICK_MS: u32 = 16;
/// The most one writing step may advance the pen, in ms. A step is the time really passed
/// since the last, so the pen keeps its speed at any frame rate; this stops a tab that
/// slept from throwing the pen far ahead. Safe range 2 to 8 times `INK_TICK_MS`.
pub const INK_STEP_MAX_MS: u32 = 4 * INK_TICK_MS;
/// His flight between two spots (caret, row, home), in ms: the notice beat, then
/// his hops. Safe range 300 to 700.
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

/// One hour of the planner grid, in CSS pixels. `--hour-h` in `theme.css` has the
/// same height, so the current-time line and the slips can be placed from the clock.
/// Change both together; safe range 40 to 100 px.
pub const HOUR_PX: f32 = 64.0;
/// A slip shorter than this writes its words a size smaller (`.placement.short` in
/// `theme.css`); every slip holds them inside its edges.
pub const SHORT_SLIP_PX: f32 = HOUR_PX;
/// The band near a scroll-box edge where a held slip makes the paper slide along so slots
/// beyond the edge can be reached without letting go. Safe range 24 to 96 px.
pub const EDGE_ZONE_PX: f32 = 48.0;
/// The most the paper slides per auto-scroll tick, right at an edge. Safe range 2 to 12 px.
pub const EDGE_MAX_PX: f32 = 6.0;

#[cfg(test)]
mod tests {
    use super::*;

    const CSS: &str = include_str!("../../../web/theme.css");

    /// The px of every `edge` (`max-width` or `min-width`) in the `@media` lines, however
    /// the query is combined or spaced, and how many widths of any kind those lines name.
    fn media_edges(edge: &str) -> (Vec<u32>, usize) {
        let medias: Vec<&str> = CSS
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("@media"))
            .collect();
        let edges = medias
            .iter()
            .flat_map(|line| line.split(edge).skip(1))
            .map(|rest| {
                let digits = rest.trim_start().trim_start_matches(':').trim_start();
                let px = digits.split("px").next().expect("a width in px");
                px.trim().parse().expect("a whole number of px")
            })
            .collect();
        let named = medias
            .iter()
            .map(|line| line.matches("width").count())
            .sum();
        (edges, named)
    }

    /// A media query cannot read a variable, so the one phone width is typed in the style
    /// sheet; this holds every query to `PHONE_MEDIA`, and the wide side to the px after it.
    #[test]
    fn every_phone_breakpoint_in_the_style_sheet_is_the_one_in_tuning() {
        let phone: u32 = PHONE_MEDIA
            .trim_start_matches("(max-width: ")
            .trim_end_matches("px)")
            .parse()
            .expect("PHONE_MEDIA is a max-width in px");
        let ((narrow, named), (wide, _)) = (media_edges("max-width"), media_edges("min-width"));
        assert!(!narrow.is_empty() && !wide.is_empty());
        assert_eq!(
            named,
            narrow.len() + wide.len(),
            "a media query names a width other than max-width or min-width"
        );
        assert!(
            narrow.iter().all(|&px| px == phone),
            "{narrow:?} vs {phone}"
        );
        assert!(
            wide.iter().all(|&px| px == phone + 1),
            "{wide:?} vs {phone} + 1"
        );
    }

    /// A duration the page writes to a variable has a fallback in the style sheet for the
    /// frame before it does; the fallback is the same number.
    #[test]
    fn the_slip_off_fallback_in_the_style_sheet_is_the_one_in_tuning() {
        let fallback = format!("var(--slip-off-ms, {SLIP_OFF_MS}ms)");
        assert!(CSS.contains(&fallback), "theme.css should say {fallback}");
    }
}

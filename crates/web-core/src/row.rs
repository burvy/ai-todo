//! What a press on a task row has become.
//!
//! A row answers three gestures from the same finger: a tap, a press-and-hold
//! that picks the row up to reorder it, and a swipe to the right that scribbles
//! it out, the ink following the finger. The browser cannot tell them apart, and guessing wrong is expensive —
//! a tap that becomes a drag throws the user's list around, a hold that stays
//! a tap never reorders anything — so every decision is a pure function of the
//! pointer's travel and how long it has been down, and all of it is unit
//! tested. Only the DOM side that feeds these numbers in lives in `tasks/mod.rs`.

/// How long a finger stays down before the row is picked up. Long enough that
/// nobody picks a row up by accident while scrolling or by resting a thumb on
/// the phone, short enough that it does not feel like a wait.
pub const HOLD_MS: u32 = 280;

/// Travel that ends the hold without picking the row up: past this the finger
/// is scrolling the page, which is what a finger that slides is almost always
/// doing.
pub const SLOP_PX: f64 = 8.0;

/// The least rightward travel that deletes when the pointer lets go: a flick
/// needs this much, a slow swipe a share of the row (see [`scribble_commits`]).
pub const SWIPE_PX: f64 = 64.0;

use crate::tuning::{
    SCRIBBLE_BAND, SCRIBBLE_COMMIT_SHARE, SCRIBBLE_FLICK_PX_PER_MS, SCRIBBLE_JITTER,
    SCRIBBLE_LOOP_PX,
};

/// What a press on a row has turned into so far. Every state is decided, and
/// only `Pressed` is still undecided.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gesture {
    /// Down, but not yet a tap, a lift or a swipe.
    Pressed,
    /// The finger slid without settling: this is a scroll, and the row's
    /// business ends here.
    Scrolling,
    /// Held long enough: the row is off the page and moving with the finger.
    Lifted,
    /// Travelling right: the row is on its way out.
    Swiping,
}

/// One step of the gesture. `dx` and `dy` are how far the pointer has travelled
/// from where it went down; `held_ms` how long it has been down.
///
/// Decided states are final, so a finger that lifts and then travels right
/// reorders rather than deletes - the hold was the more deliberate of the two.
pub fn advance(g: Gesture, dx: f64, dy: f64, held_ms: u32) -> Gesture {
    match g {
        Gesture::Lifted | Gesture::Swiping | Gesture::Scrolling => g,
        Gesture::Pressed => {
            if held_ms >= HOLD_MS {
                Gesture::Lifted
            } else if (dx * dx + dy * dy).sqrt() < SLOP_PX {
                Gesture::Pressed
            } else if dx > dy.abs() {
                // Decided as soon as the pointer leaves the slop: a pointer
                // reports every few pixels, so waiting for a full swipe's
                // travel would call every real swipe a scroll first.
                Gesture::Swiping
            } else {
                Gesture::Scrolling
            }
        }
    }
}

/// What letting go means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Release {
    /// A tap. The row's own controls - the box, the title, the x - take it.
    Tap,
    /// Dropped somewhere in the list: reorder onto whatever row is under it.
    Reorder,
    /// Finish the scribble across the row: the task is done, and tears off.
    Done,
    /// Not far enough to mean it: the ink lifts away and the row stays.
    SnapBack,
    /// A scroll, not ours. Nothing to undo.
    Scroll,
}

/// What releasing a pointer does, given what the gesture had become. A swipe is
/// judged by [`scribble_commits`] on how far right it finished (`dx`) across a row
/// `width` wide, and how fast it was going (`velocity`, px per ms).
pub fn release(g: Gesture, dx: f64, width: f64, velocity: f64) -> Release {
    match g {
        Gesture::Pressed => Release::Tap,
        Gesture::Lifted => Release::Reorder,
        Gesture::Scrolling => Release::Scroll,
        Gesture::Swiping if scribble_commits(dx, width, velocity) => Release::Done,
        Gesture::Swiping => Release::SnapBack,
    }
}

/// Whether letting go of a swipe deletes the row: past a share of its width, or a
/// flick past [`SWIPE_PX`]. Slow and short, the user is still deciding.
pub fn scribble_commits(dx: f64, width: f64, velocity: f64) -> bool {
    let reach = (width * SCRIBBLE_COMMIT_SHARE).max(SWIPE_PX);
    dx >= reach || (dx >= SWIPE_PX && velocity >= SCRIBBLE_FLICK_PX_PER_MS)
}

/// How much of the scribble the finger has drawn, 0 to 1: the ink is under the
/// finger, never ahead of it.
pub fn ink_drawn(dx: f64, width: f64) -> f64 {
    if width <= 0.0 {
        return 0.0;
    }
    (dx / width).clamp(0.0, 1.0)
}

/// Whether the timer of the stroke `lifted`, lifting away after a short swipe, may
/// take the ink now on its row (`on_row`): only its own. A new swipe that starts
/// before the lift ends draws a stroke of its own, and keeps it.
pub fn lift_clears(lifted: u32, on_row: Option<u32>) -> bool {
    on_row == Some(lifted)
}

/// Deterministic small value for a strip, so a given task always hangs from
/// the same offset on the pin and lies at the same jaunty angle. FNV-1a.
pub fn hash_unit(id: &str, salt: u8) -> f64 {
    // The top 24 bits mapped to -1..=1.
    (f64::from(hash32(id, salt) >> 8) / 16_777_215.0 - 0.5) * 2.0
}

fn hash32(id: &str, salt: u8) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in id.bytes().chain([salt]) {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

/// Where a row's words sit inside the row, in px from its top-left corner: the
/// first line's height, and from where the words start to where they end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Words {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

/// The scribble across a row's `words`, as an SVG path: loops from where the words
/// start to where they end, inside their [`SCRIBBLE_BAND`]. Each loop's width and
/// height wander by [`SCRIBBLE_JITTER`], seeded by `seed` (the row's id). It is
/// drawn whole and revealed as far as the swipe has gone.
pub fn scribble_path(words: Words, seed: &str) -> String {
    let (low, high) = SCRIBBLE_BAND;
    let height = words.bottom - words.top;
    let (top, bottom) = (words.top + height * low, words.top + height * high);
    let (middle, reach) = ((top + bottom) / 2.0, (bottom - top) / 2.0);
    let jitter = |i: usize, salt: u8| SCRIBBLE_JITTER * hash_unit(&format!("{seed}:{i}"), salt);
    let mut x = words.left;
    let loops: String = (1_usize..)
        .map_while(|i| {
            (x < words.right).then(|| {
                x = (x + SCRIBBLE_LOOP_PX / 2.0 * (1.0 + jitter(i, 0))).min(words.right);
                let side = if i % 2 == 1 { -1.0 } else { 1.0 };
                let y = middle + side * reach * (1.0 - jitter(i, 1).abs());
                format!(" L{x:.1} {y:.1}")
            })
        })
        .collect();
    format!("M{:.1} {middle:.1}{loops}", words.left)
}

/// Where a row lands relative to the row it was dropped on, which the drop
/// decides from which half of that row the pointer was over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Before,
    After,
}

/// Whether a drop at `y` on a row spanning `top..=bottom` belongs above it.
pub fn lands_above(y: f64, top: f64, bottom: f64) -> bool {
    y < (top + bottom) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A desktop list row, where a share of the width is less than SWIPE_PX, and a
    /// pointer moving too slowly to count as a flick.
    const ROW: f64 = 120.0;
    const SLOW: f64 = 0.01;

    fn held(g: Gesture, dx: f64, dy: f64, ms: u32) -> Gesture {
        advance(g, dx, dy, ms)
    }

    #[test]
    fn a_touch_that_does_not_move_or_wait_is_still_a_tap() {
        assert_eq!(held(Gesture::Pressed, 0.0, 0.0, 0), Gesture::Pressed);
        assert_eq!(held(Gesture::Pressed, 1.0, 0.0, 100), Gesture::Pressed);
        assert_eq!(release(Gesture::Pressed, 2.0, ROW, SLOW), Release::Tap);
    }

    #[test]
    fn resting_a_thumb_picks_the_row_up() {
        assert_eq!(held(Gesture::Pressed, 1.0, 0.0, HOLD_MS), Gesture::Lifted);
        assert_eq!(
            held(Gesture::Pressed, 0.0, 0.0, HOLD_MS + 500),
            Gesture::Lifted
        );
    }

    #[test]
    fn a_sliding_finger_scrolls_and_never_lifts_the_row() {
        // Down the list, past the slop but never held: this is a scroll.
        assert_eq!(held(Gesture::Pressed, 0.0, 40.0, 10), Gesture::Scrolling);
        // And once scrolling, staying down for a long time does not pick it up:
        // a finger that has already started scrolling is scrolling.
        assert_eq!(
            held(Gesture::Scrolling, 0.0, 90.0, HOLD_MS + 900),
            Gesture::Scrolling
        );
        assert_eq!(release(Gesture::Scrolling, 0.0, ROW, SLOW), Release::Scroll);
    }

    #[test]
    fn travelling_right_crosses_the_row_out() {
        assert_eq!(held(Gesture::Pressed, SWIPE_PX, 0.0, 50), Gesture::Swiping);
        assert_eq!(
            release(Gesture::Swiping, SWIPE_PX, ROW, SLOW),
            Release::Done
        );
        // A diagonal that happens to go far right is still a scroll unless the
        // rightward travel clearly wins.
        assert_eq!(
            held(Gesture::Pressed, SWIPE_PX, SWIPE_PX, 50),
            Gesture::Scrolling
        );
        assert_eq!(
            held(Gesture::Pressed, SWIPE_PX, SWIPE_PX - 1.0, 50),
            Gesture::Swiping
        );
    }

    #[test]
    fn a_swipe_that_is_backed_out_snaps_home() {
        assert_eq!(held(Gesture::Pressed, 90.0, 0.0, 50), Gesture::Swiping);
        assert_eq!(
            release(Gesture::Swiping, 20.0, ROW, SLOW),
            Release::SnapBack
        );
        // Right, but not far enough to mean it: the row goes back unharmed.
        assert_eq!(
            held(Gesture::Pressed, SWIPE_PX - 1.0, 0.0, 50),
            Gesture::Swiping
        );
        assert_eq!(
            release(Gesture::Swiping, SWIPE_PX - 1.0, ROW, SLOW),
            Release::SnapBack
        );
    }

    #[test]
    fn a_swipe_reported_a_few_pixels_at_a_time_still_deletes() {
        // A mouse or finger sends a move every few pixels, so the gesture
        // passes the slop long before it has travelled a whole swipe.
        let gesture = (1..=40).fold(Gesture::Pressed, |g, step| {
            held(g, f64::from(step) * 2.0, 1.0, step * 4)
        });
        assert_eq!(gesture, Gesture::Swiping);
        assert_eq!(release(gesture, 80.0, ROW, SLOW), Release::Done);
    }

    #[test]
    fn a_swipe_left_is_a_scroll_so_the_page_turns() {
        // Swiping left is the page flip, not ours: it must reach the panes.
        assert_eq!(held(Gesture::Pressed, -90.0, 0.0, 50), Gesture::Scrolling);
    }

    #[test]
    fn the_hold_wins_over_a_swipe_that_starts_after_it() {
        // Lifted, then dragged right: the row is being reordered, and releasing
        // it must not delete it.
        let lifted = held(Gesture::Pressed, 0.0, 0.0, HOLD_MS);
        assert_eq!(held(lifted, 200.0, 0.0, HOLD_MS + 200), Gesture::Lifted);
        assert_eq!(release(Gesture::Lifted, 200.0, ROW, SLOW), Release::Reorder);
    }

    #[test]
    fn a_decided_gesture_is_never_re_decided() {
        // Every decided state is a fixed point, which is what stops a finger
        // that drifts back under the slop from turning into a tap.
        for g in [Gesture::Lifted, Gesture::Swiping, Gesture::Scrolling] {
            assert_eq!(held(g, 0.0, 0.0, 0), g);
            assert_eq!(held(g, -200.0, 0.0, 9999), g);
        }
    }

    #[test]
    fn a_phone_row_needs_a_real_swipe_or_a_flick() {
        let phone = 350.0;
        let reach = phone * SCRIBBLE_COMMIT_SHARE;
        assert!(
            !scribble_commits(reach - 1.0, phone, SLOW),
            "still deciding"
        );
        assert!(scribble_commits(reach, phone, SLOW));
        // A quick flick deletes from SWIPE_PX on, but never from a nudge.
        assert!(scribble_commits(SWIPE_PX, phone, SCRIBBLE_FLICK_PX_PER_MS));
        assert!(!scribble_commits(SWIPE_PX - 1.0, phone, 1.0));
        assert_eq!(
            release(Gesture::Swiping, reach - 1.0, phone, SLOW),
            Release::SnapBack
        );
    }

    #[test]
    fn the_ink_is_under_the_finger_and_never_ahead() {
        assert_eq!(ink_drawn(0.0, 300.0), 0.0);
        assert_eq!(ink_drawn(150.0, 300.0), 0.5);
        assert_eq!(ink_drawn(900.0, 300.0), 1.0, "past the edge: drawn whole");
        assert_eq!(ink_drawn(-40.0, 300.0), 0.0, "back left of where it began");
        assert_eq!(ink_drawn(40.0, 0.0), 0.0, "a row with no width");
    }

    fn points(path: &str) -> Vec<(f64, f64)> {
        path.split(['M', 'L'])
            .filter_map(|point| {
                let mut xy = point.split_whitespace().map(|n| n.parse::<f64>().ok());
                Some((xy.next()??, xy.next()??))
            })
            .collect()
    }

    #[test]
    fn the_scribble_crosses_the_words_and_nothing_else() {
        // A title from 40px (past the checkbox) to 220px, its line 10 to 30px down.
        let words = Words {
            left: 40.0,
            right: 220.0,
            top: 10.0,
            bottom: 30.0,
        };
        let points = points(&scribble_path(words, "t1"));
        assert_eq!(
            points.first().map(|p| p.0),
            Some(40.0),
            "starts at the words"
        );
        assert_eq!(points.last().map(|p| p.0), Some(220.0), "ends with them");
        assert!(
            points.windows(2).all(|w| w[1].0 > w[0].0),
            "always left to right"
        );
        let (low, high) = SCRIBBLE_BAND;
        let band = (10.0 + 20.0 * low - 0.05)..=(10.0 + 20.0 * high + 0.05);
        assert!(
            points.iter().all(|(_, y)| band.contains(y)),
            "inside the letters' height: {points:?}"
        );
        assert!(points.len() > 15, "loops, not a line: {}", points.len());
    }

    #[test]
    fn a_lift_only_takes_its_own_stroke() {
        assert!(lift_clears(3, Some(3)), "its own ink lifts away");
        assert!(
            !lift_clears(3, Some(4)),
            "a new swipe began within the lift: its ink stays"
        );
        assert!(!lift_clears(3, None), "already gone");
    }

    #[test]
    fn each_row_scribbles_its_own_way() {
        let words = Words {
            left: 0.0,
            right: 300.0,
            top: 0.0,
            bottom: 24.0,
        };
        assert_eq!(
            scribble_path(words, "a"),
            scribble_path(words, "a"),
            "seeded"
        );
        assert_ne!(scribble_path(words, "a"), scribble_path(words, "b"));
        // Not a sawtooth: the loops are not all one width or one height.
        let points = points(&scribble_path(words, "a"));
        let widths: Vec<i64> = points
            .windows(2)
            .map(|w| ((w[1].0 - w[0].0) * 10.0).round() as i64)
            .collect();
        let heights: Vec<i64> = points.iter().map(|p| (p.1 * 10.0).round() as i64).collect();
        assert!(widths[..widths.len() - 1].windows(2).any(|w| w[0] != w[1]));
        assert!(
            heights
                .iter()
                .skip(1)
                .step_by(2)
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 1
        );
    }

    #[test]
    fn the_top_half_of_a_row_is_above_it() {
        assert!(lands_above(110.0, 100.0, 140.0));
        assert!(!lands_above(130.0, 100.0, 140.0));
        // Exactly the middle counts as below, so a drop down the list that
        // lands on the seam does not jump a row upwards.
        assert!(!lands_above(120.0, 100.0, 140.0));
    }
}

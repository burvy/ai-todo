//! The ink trail Rambo leaves when you pull him across the page.
//!
//! Pulling Rambo is a finger moving fast, slowly, or not at all, so the raw
//! pointer path is the wrong thing to draw: it bunches up where the finger was
//! slow, gaps where it was quick, and jitters around the line. Everything here
//! is pure and takes a path of points, so the drawing can be reasoned about and
//! tested without a pointer.
//!
//! The trail is a run of round dots rather than a stroked path: a stroked path
//! has one width for the whole stroke, and a pen stroke is thin where it starts,
//! full in the middle and thin where it lifts.

/// One dab of ink, in the page's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
}

/// A finger's path, in the order it was walked.
pub type Path = Vec<(f64, f64)>;

/// The default look: a dab every few points, so slow and fast hands draw the
/// same line, with the ends thinner than the middle.
pub const SPACING: f64 = 4.0;
pub const BASE_RADIUS: f64 = 3.0;
/// How much of the radius the first and last dabs keep. A nib touches down
/// lighter than it presses.
pub const TAPER: f64 = 0.45;

/// The dabs for one stroke.
///
/// Segments of no length — a finger that paused, or a pointer event with no
/// movement — are skipped, so a finger that never moves draws nothing at all and
/// tapping Rambo does not blot the page. Every other segment is walked from
/// where the last dab was left, placing a dab every `spacing` and one where the
/// finger stopped, so a fast hop draws the same line as a slow one. The radius
/// follows a taper into the first and last dabs.
///
/// A stroke too short for even one dab still inked: one light dot where it
/// stopped.
pub fn trail(path: &[(f64, f64)], spacing: f64, base_radius: f64, taper: f64) -> Vec<Dot> {
    let spacing = spacing.max(f64::EPSILON);
    let mut dots: Vec<Dot> = Vec::new();
    // Distance walked since the last dab: it carries across segments, so a
    // hand that crawls and a hand that leaps draw the same line.
    let mut pending = 0.0;
    for pair in path.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let step = (dx * dx + dy * dy).sqrt();
        if step <= MOVE_EPSILON {
            continue;
        }
        let mut walked = 0.0;
        while pending + (step - walked) >= spacing {
            walked += spacing - pending;
            dots.push(Dot {
                x: x0 + dx * walked / step,
                y: y0 + dy * walked / step,
                radius: base_radius,
            });
            pending = 0.0;
        }
        pending += step - walked;
    }
    if dots.is_empty()
        && let Some((x, y)) = path
            .windows(2)
            .find(|pair| {
                let (dx, dy) = (pair[1].0 - pair[0].0, pair[1].1 - pair[0].1);
                (dx * dx + dy * dy).sqrt() > MOVE_EPSILON
            })
            .map(|pair| pair[1])
    {
        dots.push(Dot {
            x,
            y,
            radius: base_radius * taper,
        });
    }
    taper_ends(&mut dots, base_radius, taper);
    dots
}

/// A finger that has not moved: sub-pixel tremor and repeated pointer events
/// are the same point, and neither is ink.
const MOVE_EPSILON: f64 = 0.01;

/// Thin the first and last dabs, the way a nib lands and lifts.
fn taper_ends(dots: &mut [Dot], base_radius: f64, taper: f64) {
    let last = dots.len().saturating_sub(1);
    if last < 2 {
        return;
    }
    // A short stroke thins over a quarter of its length, a long one over a
    // fixed few dabs, so a scribble does not fade away to nothing.
    let reach = (dots.len() / 4).max(2);
    for (index, dot) in dots.iter_mut().enumerate() {
        let from_start = index.min(last - index);
        let weight = taper + (1.0 - taper) * (from_start.min(reach) as f64 / reach as f64);
        dot.radius = base_radius * weight;
    }
}

/// The whole trail in one go: [`trail`] with the look this app draws.
pub fn rambo_trail(path: &[(f64, f64)]) -> Vec<Dot> {
    trail(path, SPACING, BASE_RADIUS, TAPER)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(from: (f64, f64), to: (f64, f64), steps: usize) -> Path {
        (0..=steps)
            .map(|i| {
                let t = i as f64 / steps as f64;
                (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
            })
            .collect()
    }

    #[test]
    fn a_still_finger_draws_nothing() {
        // A press with no movement is not a dot on the page.
        assert!(rambo_trail(&[]).is_empty());
        assert!(rambo_trail(&[(10.0, 10.0)]).is_empty());
        assert!(rambo_trail(&[(10.0, 10.0); 5]).is_empty());
    }

    #[test]
    fn a_held_still_finger_does_not_blob() {
        // Pausing mid-stroke repeats the same point; the dab count must not
        // grow with the pause, or holding still makes a blot.
        let mut path = line((0.0, 0.0), (100.0, 0.0), 10);
        let held = *path.last().unwrap();
        path.extend(std::iter::repeat_n(held, 40));
        let paused = rambo_trail(&path);
        let drawn = rambo_trail(&line((0.0, 0.0), (100.0, 0.0), 10));
        assert_eq!(paused.len(), drawn.len());
    }

    #[test]
    fn a_slow_hand_and_a_fast_hand_draw_the_same_line() {
        // Same path, wildly different sample density: the dab count follows the
        // distance walked, not the number of events.
        let expected = 200.0 / SPACING;
        let dense = rambo_trail(&line((0.0, 0.0), (200.0, 0.0), 200));
        let sparse = rambo_trail(&line((0.0, 0.0), (200.0, 0.0), 4));
        for (label, dots) in [("dense", &dense), ("sparse", &sparse)] {
            assert!(
                (dots.len() as f64 - expected).abs() <= 2.0,
                "{label}: {} dabs for a 200px line, expected about {expected}",
                dots.len()
            );
        }
    }

    #[test]
    fn a_fast_hop_is_filled_in_not_gapped() {
        // Two pointer events 60px apart still draw a continuous line.
        let dots = rambo_trail(&[(0.0, 0.0), (60.0, 0.0)]);
        assert!(dots.len() > 10, "{}", dots.len());
        for pair in dots.windows(2) {
            let gap = (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
            assert!(gap <= SPACING * 1.5, "gap {gap}");
        }
    }

    #[test]
    fn a_trail_lands_where_the_finger_stopped() {
        let dots = rambo_trail(&line((0.0, 0.0), (100.0, 40.0), 25));
        let last = dots.last().expect("a trail");
        assert!((last.x - 100.0).abs() < SPACING, "{last:?}");
        assert!((last.y - 40.0).abs() < SPACING, "{last:?}");
    }

    #[test]
    fn a_trail_thins_at_both_ends_and_is_fullest_in_the_middle() {
        let dots = rambo_trail(&line((0.0, 0.0), (300.0, 0.0), 60));
        let first = dots.first().expect("a trail");
        let last = dots.last().expect("a trail");
        let middle = dots[dots.len() / 2];
        assert!(first.radius < middle.radius, "{:?} vs {:?}", first, middle);
        assert!(last.radius < middle.radius, "{:?} vs {:?}", last, middle);
        assert_eq!(middle.radius, BASE_RADIUS, "the body keeps its width");
        // And the ends are never fainter than the taper asks for.
        assert!(first.radius >= BASE_RADIUS * TAPER);
        assert!(last.radius >= BASE_RADIUS * TAPER);
    }

    #[test]
    fn a_stroke_shorter_than_one_dab_still_inks_one_dot() {
        let dots = rambo_trail(&[(0.0, 0.0), (2.0, 0.0)]);
        assert_eq!(dots.len(), 1);
        assert!((dots[0].x - 2.0).abs() < 0.01, "{dots:?}");
        assert!(dots[0].radius > 0.0);
    }

    #[test]
    fn dabs_never_exceed_the_look_asks_for() {
        let dots = rambo_trail(&line((0.0, 0.0), (500.0, 500.0), 500));
        assert!(dots.iter().all(|dot| dot.radius <= BASE_RADIUS));
        assert!(dots.iter().all(|dot| dot.radius > 0.0));
    }
}

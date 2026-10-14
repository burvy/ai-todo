//! The ink trail Rambo leaves when you pull him across the page.
//!
//! Pulling Rambo is a finger moving fast, slowly, or not at all, so the raw
//! pointer path is the wrong thing to draw: it bunches up where the finger was
//! slow, gaps where it was quick, and jitters around the line. Everything here
//! is pure and takes a path of points, so the drawing can be reasoned about and
//! tested without a pointer.
//!
//! The trail is a run of round dabs rather than a stroked path: a stroked path
//! has one width for the whole stroke, and a pen stroke is thin where it starts,
//! full in the middle and thin where it lifts.
//!
//! The numbers come from `docs/DESIGN-DIRECTION.md` §6: each tool writes in its
//! own ink and its own weight, the pencil is light and grainy and the marker is
//! blunt. Colour is not decided here — the page draws the dabs in the tool's
//! ink and the tail of the stroke in `--rambo-tint`.

use crate::settings::Pen;

/// One dab of ink, in the page's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    /// How much ink the nib lays down, 0..=1. Graphite is light; a felt tip is
    /// not, and a quill's tail dries out.
    pub alpha: f64,
    /// The last few dabs of a stroke take Rambo's own tint, so the trail is
    /// his rather than the tool's (DESIGN-DIRECTION §1: `--rambo-tint` is the
    /// ink tail of a stroke).
    pub tail: bool,
}

/// A finger's path, in the order it was walked.
pub type Path = Vec<(f64, f64)>;

/// How one tool's nib puts ink down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nib {
    /// How far apart the dabs are, so a slow hand and a fast hand draw the same
    /// line.
    pub spacing: f64,
    /// The width of the body of the stroke.
    pub radius: f64,
    /// What the first and last dabs keep: `1.0` is blunt (a marker lands flat),
    /// lower is a nib touching down.
    pub taper: f64,
    /// How much the **deposit** varies from dab to dab, 0..=1. Graphite lays
    /// down an even width with uneven ink; a wandering outline reads as a shaky
    /// hand and, at two or three pixels, as aliasing.
    pub grain: f64,
    /// How much ink the nib lays down in the body of the stroke, 0..=1.
    pub alpha: f64,
    /// How far the ink dries out through the run-out, 0..=1. A quill is the one
    /// tool that runs dry; a pen does not.
    pub dry_out: f64,
}

/// The fountain pen: the plain line every other tool is measured against.
const FOUNTAIN: Nib = Nib {
    spacing: 4.0,
    radius: 3.0,
    taper: 0.45,
    grain: 0.0,
    alpha: 1.0,
    dry_out: 0.0,
};

impl Nib {
    /// The nib of the chosen tool. DESIGN-DIRECTION §6: the fountain pen and the
    /// marker write in `--ink`, the pencil in `--ink` at 70% with the grain
    /// texture, the ballpoint in `--ink-ballpoint`, the quill in `--ink-quill`.
    pub fn for_pen(pen: Pen) -> Self {
        match pen {
            Pen::FountainPen => FOUNTAIN,
            // A ballpoint lays down a fine, even line.
            Pen::Ballpoint => Nib {
                radius: 2.2,
                spacing: 3.4,
                ..FOUNTAIN
            },
            // Graphite: an even line with an uneven deposit, light ink.
            Pen::Pencil => Nib {
                radius: 2.6,
                spacing: 3.2,
                taper: 0.55,
                grain: 0.28,
                alpha: 0.625,
                ..FOUNTAIN
            },
            // A felt tip lands and lifts flat, and thick.
            Pen::Marker => Nib {
                radius: 5.0,
                spacing: 4.6,
                taper: 0.95,
                ..FOUNTAIN
            },
            // A feather is broad in the middle, runs out at the tail and dries
            // as it goes: width and ink both fall away.
            Pen::Quill => Nib {
                radius: 3.6,
                spacing: 4.2,
                taper: 0.25,
                grain: 0.12,
                dry_out: 0.85,
                ..FOUNTAIN
            },
        }
    }
}

/// The dabs for one stroke, in that tool's nib.
///
/// Segments of no length — a finger that paused, or a pointer event with no
/// movement — are skipped, so a finger that never moves draws nothing at all and
/// tapping Rambo does not blot the page. Every other segment is walked from
/// where the last dab was left, placing a dab every `spacing` along the path and
/// one where the finger stopped, so a fast hop draws the same line as a slow
/// one. The radius follows a taper into the first and last dabs.
///
/// A stroke too short for even one dab still inked: one light dot where it
/// stopped.
pub fn trail(path: &[(f64, f64)], nib: Nib) -> Vec<Dot> {
    let spacing = nib.spacing.max(f64::EPSILON);
    let mut dots: Vec<Dot> = Vec::new();
    // Distance walked since the last dab: it carries across segments, so a hand
    // that crawls and a hand that leaps draw the same line.
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
                radius: nib.radius,
                alpha: nib.alpha,
                tail: false,
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
            radius: nib.radius,
            alpha: nib.alpha,
            tail: false,
        });
    }
    taper_ends(&mut dots, nib);
    vary_deposit(&mut dots, nib);
    mark_the_tail(&mut dots);
    dots
}

/// A finger that has not moved: sub-pixel tremor and repeated pointer events are
/// the same point, and neither is ink.
const MOVE_EPSILON: f64 = 0.01;

/// Thin the first and last dabs, the way a nib lands and lifts.
fn taper_ends(dots: &mut [Dot], nib: Nib) {
    let last = dots.len().saturating_sub(1);
    if last < 2 {
        return;
    }
    // A short stroke thins over a quarter of its length, a long one over a fixed
    // few dabs, so a scribble does not fade away to nothing.
    let reach = (dots.len() / 4).max(2);
    for (index, dot) in dots.iter_mut().enumerate() {
        let from_start = index.min(last - index);
        let weight = nib.taper + (1.0 - nib.taper) * (from_start.min(reach) as f64 / reach as f64);
        dot.radius = nib.radius * weight;
    }
}

/// Vary the **deposit**, not the outline: graphite and quill ink are uneven in
/// how much they leave down, while the line they draw is an even width. The
/// variation is a function of the dab's place along the stroke rather than a
/// random number, so the same stroke always draws the same way and a redraw
/// under the finger does not shimmer.
fn vary_deposit(dots: &mut [Dot], nib: Nib) {
    let count = dots.len();
    let run_out = ((count as f64) * nib.dry_out) as usize;
    for (index, dot) in dots.iter_mut().enumerate() {
        if nib.grain > 0.0 {
            let wave = ((index * 37) % 11) as f64 / 5.0 - 1.0;
            dot.alpha *= 1.0 + nib.grain * wave;
        }
        if run_out > 0 && index >= count - run_out {
            // The last stretch of a quill's run-out dries as it goes.
            let through = (index - (count - run_out)) as f64 / run_out as f64;
            dot.alpha *= 1.0 - nib.dry_out * through;
        }
        dot.alpha = dot.alpha.clamp(0.0, 1.0);
    }
}

/// Mark the last few dabs as Rambo's: the tail of the stroke takes his tint, so
/// the trail is his rather than the tool's. Kept short — the last sixth of the
/// stroke or six dabs, whichever is fewer — so a line never looks blue-green.
fn mark_the_tail(dots: &mut [Dot]) {
    let total = dots.len();
    let tail = (total / 6).min(6);
    for dot in dots.iter_mut().skip(total.saturating_sub(tail)) {
        dot.tail = true;
    }
}

/// The whole trail in one go: the chosen tool's nib over a path.
pub fn rambo_trail(path: &[(f64, f64)], pen: Pen) -> Vec<Dot> {
    trail(path, Nib::for_pen(pen))
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

    fn fountain(path: &[(f64, f64)]) -> Vec<Dot> {
        trail(path, Nib::for_pen(Pen::FountainPen))
    }

    #[test]
    fn a_still_finger_draws_nothing() {
        // A press with no movement is not a dot on the page.
        assert!(fountain(&[]).is_empty());
        assert!(fountain(&[(10.0, 10.0)]).is_empty());
        assert!(fountain(&[(10.0, 10.0); 5]).is_empty());
    }

    #[test]
    fn a_held_still_finger_does_not_blob() {
        // Pausing mid-stroke repeats the same point; the dab count must not grow
        // with the pause, or holding still makes a blot.
        let mut path = line((0.0, 0.0), (100.0, 0.0), 10);
        let held = *path.last().unwrap();
        path.extend(std::iter::repeat_n(held, 40));
        let paused = fountain(&path);
        let drawn = fountain(&line((0.0, 0.0), (100.0, 0.0), 10));
        assert_eq!(paused.len(), drawn.len());
    }

    #[test]
    fn a_slow_hand_and_a_fast_hand_draw_the_same_line() {
        // Same path, wildly different sample density: the dab count follows the
        // distance walked, not the number of events.
        let nib = Nib::for_pen(Pen::FountainPen);
        let expected = 200.0 / nib.spacing;
        let dense = fountain(&line((0.0, 0.0), (200.0, 0.0), 200));
        let sparse = fountain(&line((0.0, 0.0), (200.0, 0.0), 4));
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
        let spacing = Nib::for_pen(Pen::FountainPen).spacing;
        let dots = fountain(&[(0.0, 0.0), (60.0, 0.0)]);
        assert!(dots.len() > 10, "{}", dots.len());
        for pair in dots.windows(2) {
            let gap = (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
            assert!(gap <= spacing * 1.5, "gap {gap}");
        }
    }

    #[test]
    fn a_trail_lands_where_the_finger_stopped() {
        let spacing = Nib::for_pen(Pen::FountainPen).spacing;
        let dots = fountain(&line((0.0, 0.0), (100.0, 40.0), 25));
        let last = dots.last().expect("a trail");
        assert!((last.x - 100.0).abs() < spacing, "{last:?}");
        assert!((last.y - 40.0).abs() < spacing, "{last:?}");
    }

    #[test]
    fn a_trail_thins_at_both_ends_and_is_fullest_in_the_middle() {
        let nib = Nib::for_pen(Pen::FountainPen);
        let dots = fountain(&line((0.0, 0.0), (300.0, 0.0), 60));
        let first = dots.first().expect("a trail");
        let last = dots.last().expect("a trail");
        let middle = dots[dots.len() / 2];
        assert!(first.radius < middle.radius, "{:?} vs {:?}", first, middle);
        assert!(last.radius < middle.radius, "{:?} vs {:?}", last, middle);
        assert_eq!(middle.radius, nib.radius, "the body keeps its width");
        assert!(first.radius >= nib.radius * nib.taper);
        assert!(last.radius >= nib.radius * nib.taper);
    }

    #[test]
    fn a_stroke_shorter_than_one_dab_still_inks_one_dot() {
        let dots = fountain(&[(0.0, 0.0), (2.0, 0.0)]);
        assert_eq!(dots.len(), 1);
        assert!((dots[0].x - 2.0).abs() < 0.01, "{dots:?}");
        assert!(dots[0].radius > 0.0);
    }

    #[test]
    fn every_tool_keeps_a_positive_width_and_its_own_look() {
        let path = line((0.0, 0.0), (500.0, 500.0), 500);
        for pen in [
            Pen::FountainPen,
            Pen::Ballpoint,
            Pen::Pencil,
            Pen::Marker,
            Pen::Quill,
        ] {
            let nib = Nib::for_pen(pen);
            assert!(nib.spacing > 0.0 && nib.radius > 0.0, "{pen:?}");
            assert!((0.0..=1.0).contains(&nib.taper), "{pen:?}");
            assert!((0.0..=1.0).contains(&nib.alpha), "{pen:?}");
            assert!((0.0..=1.0).contains(&nib.dry_out), "{pen:?}");
            let dots = rambo_trail(&path, pen);
            assert!(dots.iter().all(|dot| dot.radius > 0.0), "{pen:?}");
            assert!(
                dots.iter().all(|dot| dot.radius <= nib.radius),
                "{pen:?}: a dab is never wider than the nib"
            );
            assert!(
                dots.iter().all(|dot| (0.0..=1.0).contains(&dot.alpha)),
                "{pen:?}"
            );
        }
        // The marker is the broad one and the ballpoint the fine one.
        assert!(Nib::for_pen(Pen::Marker).radius > Nib::for_pen(Pen::FountainPen).radius);
        assert!(Nib::for_pen(Pen::Ballpoint).radius < Nib::for_pen(Pen::FountainPen).radius);
        // A marker lands and lifts flat; a quill runs out at the tail.
        assert!(Nib::for_pen(Pen::Marker).taper > Nib::for_pen(Pen::Quill).taper);
    }

    #[test]
    fn graphite_varies_its_ink_not_its_width() {
        let path = line((0.0, 0.0), (300.0, 0.0), 120);
        let nib = Nib::for_pen(Pen::Pencil);
        let pencil = rambo_trail(&path, Pen::Pencil);
        let body = &pencil[pencil.len() / 4..pencil.len() * 3 / 4];
        assert!(
            body.iter().all(|dot| dot.radius == nib.radius),
            "a pencil draws an even width: {:?}",
            body.iter().map(|d| d.radius).collect::<Vec<_>>()
        );
        let inks: Vec<f64> = body.iter().map(|dot| dot.alpha).collect();
        let min = inks.iter().cloned().fold(f64::MAX, f64::min);
        let max = inks.iter().cloned().fold(f64::MIN, f64::max);
        // Roughly 0.45 to 0.8 by design; the band is stated against the nib's own
        // alpha so the test cannot fail on float slack.
        assert!(
            min >= nib.alpha * 0.70 && max <= nib.alpha * 1.30,
            "pencil ink {min}..{max}"
        );
        assert!(max > min + 0.05, "the deposit wanders: {inks:?}");

        // Ink is steady in both width and deposit.
        let fountain = rambo_trail(&path, Pen::FountainPen);
        let body = &fountain[fountain.len() / 4..fountain.len() * 3 / 4];
        assert!(body.iter().all(|dot| dot.alpha == 1.0));
        assert!(
            body.iter().all(|dot| dot.radius == FOUNTAIN.radius),
            "{:?}",
            body.iter().map(|d| d.radius).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_quills_tail_dries_out() {
        let path = line((0.0, 0.0), (300.0, 0.0), 120);
        let quill = rambo_trail(&path, Pen::Quill);
        let head = quill[quill.len() / 4];
        let tip = quill.last().expect("a trail");
        assert!(
            tip.alpha < head.alpha * 0.5,
            "tail {tip:?} vs head {head:?}"
        );
        // And a pen does not dry.
        let pen = rambo_trail(&path, Pen::FountainPen);
        assert_eq!(pen.last().expect("a trail").alpha, 1.0);
    }

    #[test]
    fn only_the_short_tail_is_rambo_tinted() {
        let path = line((0.0, 0.0), (300.0, 0.0), 120);
        let dots = rambo_trail(&path, Pen::FountainPen);
        let tinted = dots.iter().filter(|dot| dot.tail).count();
        assert!(tinted > 0, "the tail is his");
        assert!(tinted <= 6, "and short: {tinted}");
        assert!(tinted * 6 <= dots.len(), "no more than a sixth: {tinted}");
        // The tint is the end of the stroke, never the start.
        assert_eq!(
            dots.iter().position(|dot| dot.tail),
            Some(dots.len() - tinted)
        );
    }

    #[test]
    fn the_same_stroke_always_draws_the_same_way() {
        // Grain must not be random: the same path twice is the same trail, or a
        // redraw under the finger would shimmer.
        let path = line((0.0, 0.0), (200.0, 120.0), 40);
        assert_eq!(
            rambo_trail(&path, Pen::Pencil),
            rambo_trail(&path, Pen::Pencil)
        );
        assert_eq!(
            rambo_trail(&path, Pen::Quill),
            rambo_trail(&path, Pen::Quill)
        );
    }
}

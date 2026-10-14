//! The ink Rambo leaves on the page: the trail when you pull him across it, and
//! the splat when he hits a side.
//!
//! A finger moves fast, slowly, or not at all, so the raw pointer path is the
//! wrong thing to draw: it bunches up where the finger was slow, has corners where
//! the events are far apart, and jitters around the line. The trail here is drawn
//! through the pointer's samples as a smooth curve, then laid down as close round
//! dabs whose width follows the hand: a quick stroke is thinner than a slow one,
//! as it is with a real nib. Everything here is pure and takes samples, so the
//! drawing can be reasoned about and tested without a pointer.
//!
//! Ink lives on the page for a while: it lies wet, dries a shade lighter, and then
//! fades, thin strokes before thick ones ([`age_alpha`]).
//!
//! The numbers come from `docs/DESIGN-DIRECTION.md` §6: each tool writes in its
//! own ink and its own weight, the pencil is light and grainy and the marker is
//! blunt. Colour is not decided here — the page draws the dabs in the tool's
//! ink and the tail of the stroke in `--rambo-tint`. The shared numbers are in
//! [`crate::tuning`].

use crate::settings::Pen;
use crate::tuning::{
    INK_DRY_ALPHA, INK_DRY_MS, INK_FADE_OUT_MS, INK_FADE_THIN_LEAD_MS, INK_FAST_SHARE,
    INK_FAST_SPEED, INK_HOLD_MS, INK_SLOW_SPEED, INK_STEP_PX, INK_TAIL_PX, INK_TAPER_PX,
    INK_WIDTH_EASE,
};

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

/// Where the nib was, and when: `t` in ms on the pointer events' clock
/// (`event.timeStamp`), the same clock the page's frames run on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub x: f64,
    pub y: f64,
    pub t: f64,
}

/// One stroke of ink: the samples the nib passed through, in order, and the tool
/// that drew it, which it keeps if the tool is changed later.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub samples: Vec<Sample>,
    pub pen: Pen,
}

impl Stroke {
    pub fn new(pen: Pen, first: Sample) -> Self {
        Stroke {
            samples: vec![first],
            pen,
        }
    }

    /// When the nib last touched this stroke: its ink ages from then.
    pub fn laid_at(&self) -> f64 {
        self.samples.last().map_or(0.0, |sample| sample.t)
    }

    /// The stroke's dabs, in its own tool's nib.
    pub fn dots(&self) -> Vec<Dot> {
        trail(&self.samples, Nib::for_pen(self.pen))
    }
}

/// How one tool's nib puts ink down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nib {
    /// The width of the body of a slow stroke.
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
                ..FOUNTAIN
            },
            // Graphite: an even line with an uneven deposit, light ink.
            Pen::Pencil => Nib {
                radius: 2.6,
                taper: 0.55,
                grain: 0.28,
                alpha: 0.625,
                ..FOUNTAIN
            },
            // A felt tip lands and lifts flat, and thick.
            Pen::Marker => Nib {
                radius: 5.0,
                taper: 0.95,
                ..FOUNTAIN
            },
            // A feather is broad in the middle, runs out at the tail and dries
            // as it goes: width and ink both fall away.
            Pen::Quill => Nib {
                radius: 3.6,
                taper: 0.25,
                grain: 0.12,
                dry_out: 0.85,
                ..FOUNTAIN
            },
        }
    }
}

/// A finger that has not moved: sub-pixel tremor and repeated pointer events are
/// the same point, and neither is ink.
const MOVE_EPSILON: f64 = 0.01;

fn distance(a: &Sample, b: &Sample) -> f64 {
    (b.x - a.x).hypot(b.y - a.y)
}

/// The samples with the repeats of a finger that paused taken out.
fn moved(samples: &[Sample]) -> Vec<Sample> {
    let mut kept: Vec<Sample> = Vec::with_capacity(samples.len());
    for &sample in samples {
        match kept.last() {
            Some(last) if distance(last, &sample) <= MOVE_EPSILON => {}
            _ => kept.push(sample),
        }
    }
    kept
}

/// A smooth curve through every sample: a centripetal Catmull-Rom spline, which
/// passes through each point, never loops or overshoots on a sharp turn, and
/// turns the corners between far-apart pointer events into curves. Each span is
/// cut into pieces no longer than [`INK_STEP_PX`]; time runs evenly along it.
pub fn smooth(samples: &[Sample]) -> Vec<Sample> {
    let points = moved(samples);
    let Some(&first) = points.first() else {
        return Vec::new();
    };
    let mut curve = vec![first];
    for (index, pair) in points.windows(2).enumerate() {
        let (p1, p2) = (pair[0], pair[1]);
        // The ends are their own neighbours, so the curve starts and stops on them.
        let p0 = index.checked_sub(1).map_or(p1, |before| points[before]);
        let p3 = points.get(index + 2).copied().unwrap_or(p2);
        let pieces = (distance(&p1, &p2) / INK_STEP_PX).ceil().max(1.0) as usize;
        curve.extend((1..=pieces).map(|piece| {
            let u = piece as f64 / pieces as f64;
            let (x, y) = catmull_rom(&p0, &p1, &p2, &p3, u);
            Sample {
                x,
                y,
                t: p1.t + (p2.t - p1.t) * u,
            }
        }));
    }
    curve
}

/// The point `u` (0..=1) of the way from `p1` to `p2` on the centripetal
/// Catmull-Rom curve through `p0`..`p3` (Barry and Goldman's form).
fn catmull_rom(p0: &Sample, p1: &Sample, p2: &Sample, p3: &Sample, u: f64) -> (f64, f64) {
    // Knot spacing by the square root of the distance: the centripetal choice.
    let knot = |a: &Sample, b: &Sample| distance(a, b).sqrt().max(1e-6);
    let t1 = knot(p0, p1);
    let t2 = t1 + knot(p1, p2);
    let t3 = t2 + knot(p2, p3);
    let t = t1 + (t2 - t1) * u;
    let mix = |a: (f64, f64), b: (f64, f64), ta: f64, tb: f64| {
        let w = (t - ta) / (tb - ta);
        (a.0 + (b.0 - a.0) * w, a.1 + (b.1 - a.1) * w)
    };
    let (q0, q1, q2, q3) = ((p0.x, p0.y), (p1.x, p1.y), (p2.x, p2.y), (p3.x, p3.y));
    let a1 = mix(q0, q1, 0.0, t1);
    let a2 = mix(q1, q2, t1, t2);
    let a3 = mix(q2, q3, t2, t3);
    let b1 = mix(a1, a2, 0.0, t2);
    let b2 = mix(a2, a3, t1, t3);
    mix(b1, b2, t1, t2)
}

/// How wide the nib draws at `speed` px per ms, as a share of its full width: full
/// up to [`INK_SLOW_SPEED`], easing down to [`INK_FAST_SHARE`] at
/// [`INK_FAST_SPEED`] and beyond. A hurried line is a thinner one.
pub fn width_at(speed: f64) -> f64 {
    let through = ((speed - INK_SLOW_SPEED) / (INK_FAST_SPEED - INK_SLOW_SPEED)).clamp(0.0, 1.0);
    let eased = through * through * (3.0 - 2.0 * through);
    1.0 - (1.0 - INK_FAST_SHARE) * eased
}

/// The dabs for one stroke, in that tool's nib.
///
/// The samples are drawn through as a smooth curve ([`smooth`]) and a dab is laid
/// every [`INK_STEP_PX`] along it, closer than any nib's radius, so the line is
/// unbroken however fast the finger went. Each dab's width follows the hand's
/// speed there ([`width_at`]), eased along the stroke so it swells and thins and
/// never steps, and the ends taper the way a nib lands and lifts.
///
/// Samples where the finger did not move are skipped, so a finger that never
/// moves draws nothing and tapping Rambo does not blot the page. A stroke too
/// short for even one step still inks one light dot where it stopped.
pub fn trail(samples: &[Sample], nib: Nib) -> Vec<Dot> {
    let curve = smooth(samples);
    let mut dots: Vec<Dot> = Vec::new();
    // Distance walked since the last dab: it carries across pieces, so the dabs
    // are evenly spaced whatever the pieces' lengths.
    let mut pending = 0.0;
    let mut width: Option<f64> = None;
    for pair in curve.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let step = distance(&a, &b);
        if step <= MOVE_EPSILON {
            continue;
        }
        let speed = match b.t - a.t {
            dt if dt > 0.0 => step / dt,
            _ => INK_FAST_SPEED,
        };
        let mut walked = 0.0;
        while pending + (step - walked) >= INK_STEP_PX {
            walked += INK_STEP_PX - pending;
            let target = width_at(speed);
            let eased = width.map_or(target, |was| was + (target - was) * INK_WIDTH_EASE);
            width = Some(eased);
            dots.push(Dot {
                x: a.x + (b.x - a.x) * walked / step,
                y: a.y + (b.y - a.y) * walked / step,
                radius: nib.radius * eased,
                alpha: nib.alpha,
                tail: false,
            });
            pending = 0.0;
        }
        pending += step - walked;
    }
    if dots.is_empty()
        && let [.., before, last] = curve.as_slice()
        && distance(before, last) > MOVE_EPSILON
    {
        dots.push(Dot {
            x: last.x,
            y: last.y,
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

/// How many dabs cover `px` of a stroke.
fn dabs_in(px: f64) -> usize {
    (px / INK_STEP_PX).round().max(1.0) as usize
}

/// Thin the first and last dabs, the way a nib lands and lifts.
fn taper_ends(dots: &mut [Dot], nib: Nib) {
    let last = dots.len().saturating_sub(1);
    if last < 2 {
        return;
    }
    // A short stroke thins over a quarter of its length, a long one over
    // `INK_TAPER_PX`, so a scribble does not fade away to nothing.
    let reach = (dots.len() / 4).clamp(2, dabs_in(INK_TAPER_PX).max(2));
    for (index, dot) in dots.iter_mut().enumerate() {
        let from_start = index.min(last - index);
        let weight = nib.taper + (1.0 - nib.taper) * (from_start.min(reach) as f64 / reach as f64);
        dot.radius *= weight;
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

/// Mark the end of the stroke as Rambo's: its last [`INK_TAIL_PX`] take his tint,
/// so the trail is his rather than the tool's. Never more than a sixth of the
/// stroke, so a line never looks blue-green.
fn mark_the_tail(dots: &mut [Dot]) {
    let total = dots.len();
    let tail = (total / 6).min(dabs_in(INK_TAIL_PX));
    for dot in dots.iter_mut().skip(total.saturating_sub(tail)) {
        dot.tail = true;
    }
}

/// How much of a dab's ink shows `age_ms` after it was laid. Fresh ink lies wet
/// and full; over [`INK_DRY_MS`] it dries to [`INK_DRY_ALPHA`], a shade lighter;
/// after [`INK_HOLD_MS`] it fades away over [`INK_FADE_OUT_MS`], easing in and out, never
/// cut. `thinness` (0 for the nib's full width, 1 for nothing) brings the fade
/// forward by up to [`INK_FADE_THIN_LEAD_MS`]: thin strokes and tapered ends go
/// first, as faint ink does.
pub fn age_alpha(age_ms: f64, thinness: f64) -> f64 {
    let ease = |share: f64| {
        let s = share.clamp(0.0, 1.0);
        s * s * (3.0 - 2.0 * s)
    };
    let dried = 1.0 - (1.0 - INK_DRY_ALPHA) * ease(age_ms / INK_DRY_MS);
    let fade_from = INK_HOLD_MS - INK_FADE_THIN_LEAD_MS * thinness.clamp(0.0, 1.0);
    dried * (1.0 - ease((age_ms - fade_from) / INK_FADE_OUT_MS))
}

/// Whether ink laid `age_ms` ago has faded from the page entirely.
pub fn faded(age_ms: f64) -> bool {
    age_ms >= INK_HOLD_MS + INK_FADE_OUT_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight stroke from `from` to `to` in `steps` events `ms` apart.
    fn line(from: (f64, f64), to: (f64, f64), steps: usize, ms: f64) -> Vec<Sample> {
        (0..=steps)
            .map(|i| {
                let t = i as f64 / steps as f64;
                Sample {
                    x: from.0 + (to.0 - from.0) * t,
                    y: from.1 + (to.1 - from.1) * t,
                    t: i as f64 * ms,
                }
            })
            .collect()
    }

    /// A slow, careful stroke: well under `INK_SLOW_SPEED`.
    fn slow(from: (f64, f64), to: (f64, f64), steps: usize) -> Vec<Sample> {
        let length = (to.0 - from.0).hypot(to.1 - from.1);
        line(
            from,
            to,
            steps,
            length / steps as f64 / (INK_SLOW_SPEED * 0.5),
        )
    }

    fn at(x: f64, y: f64, t: f64) -> Sample {
        Sample { x, y, t }
    }

    fn fountain(samples: &[Sample]) -> Vec<Dot> {
        trail(samples, Nib::for_pen(Pen::FountainPen))
    }

    #[test]
    fn a_still_finger_draws_nothing() {
        // A press with no movement is not a dot on the page.
        assert!(fountain(&[]).is_empty());
        assert!(fountain(&[at(10.0, 10.0, 0.0)]).is_empty());
        let held: Vec<Sample> = (0..5)
            .map(|i| at(10.0, 10.0, f64::from(i) * 16.0))
            .collect();
        assert!(fountain(&held).is_empty());
    }

    #[test]
    fn a_held_still_finger_does_not_blob() {
        // Pausing mid-stroke repeats the same point; the dab count must not grow
        // with the pause, or holding still makes a blot.
        let drawn = slow((0.0, 0.0), (100.0, 0.0), 10);
        let mut paused = drawn.clone();
        let held = *paused.last().unwrap();
        paused.extend((1..=40).map(|i| Sample {
            t: held.t + f64::from(i) * 16.0,
            ..held
        }));
        assert_eq!(fountain(&paused).len(), fountain(&drawn).len());
    }

    #[test]
    fn the_dabs_follow_the_distance_not_the_events() {
        // Same path, wildly different event density: the dab count follows the
        // distance walked.
        let expected = 200.0 / INK_STEP_PX;
        let dense = fountain(&slow((0.0, 0.0), (200.0, 0.0), 200));
        let sparse = fountain(&slow((0.0, 0.0), (200.0, 0.0), 4));
        for (label, dots) in [("dense", &dense), ("sparse", &sparse)] {
            assert!(
                (dots.len() as f64 - expected).abs() <= 2.0,
                "{label}: {} dabs for a 200px line, expected about {expected}",
                dots.len()
            );
        }
    }

    #[test]
    fn no_dotted_gaps_however_fast_the_hand() {
        // Two pointer events 60px apart, a frame apart, still draw an unbroken line
        // in every tool: each dab overlaps the next, even where the quick stroke is
        // at its thinnest, in its tapered ends.
        for pen in [
            Pen::FountainPen,
            Pen::Ballpoint,
            Pen::Pencil,
            Pen::Marker,
            Pen::Quill,
        ] {
            let dots = trail(&[at(0.0, 0.0, 0.0), at(60.0, 0.0, 16.0)], Nib::for_pen(pen));
            assert!(dots.len() > 50, "{pen:?}: {}", dots.len());
            for pair in dots.windows(2) {
                let gap = (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
                let reach = pair[0].radius + pair[1].radius;
                assert!(
                    gap < reach,
                    "{pen:?}: {gap:.2}px between dabs {reach:.2}px across"
                );
            }
        }
    }

    #[test]
    fn the_curve_passes_through_every_sample_and_rounds_the_corners() {
        // A zigzag of far-apart events, the way a quick hand reports a curve.
        let samples = [
            at(0.0, 0.0, 0.0),
            at(40.0, 30.0, 16.0),
            at(80.0, 0.0, 32.0),
            at(120.0, 30.0, 48.0),
        ];
        let curve = smooth(&samples);
        for sample in &samples {
            assert!(
                curve.iter().any(|c| distance(c, sample) < 1e-6),
                "{sample:?} is on the curve"
            );
        }
        // Pieces are short (a curved span runs a little longer than its chord), and
        // the heading turns a little at a time, never in one corner: the largest
        // turn between two pieces is far below the zigzag's ~74 degree corners.
        assert!(
            curve
                .windows(2)
                .all(|p| distance(&p[0], &p[1]) <= 2.0 * INK_STEP_PX)
        );
        let heading = |p: &[Sample]| (p[1].y - p[0].y).atan2(p[1].x - p[0].x);
        let turns: Vec<f64> = curve
            .windows(3)
            .map(|w| {
                let d = heading(&w[1..]) - heading(&w[..2]);
                d.sin().atan2(d.cos()).abs().to_degrees()
            })
            .collect();
        let sharpest = turns.iter().cloned().fold(0.0, f64::max);
        assert!(sharpest < 20.0, "a corner of {sharpest:.1} degrees");
        // Time runs forward along it.
        assert!(curve.windows(2).all(|p| p[1].t >= p[0].t));
    }

    #[test]
    fn a_quick_stroke_is_thinner_than_a_slow_one_and_never_steps() {
        assert_eq!(width_at(0.0), 1.0);
        assert_eq!(width_at(INK_SLOW_SPEED), 1.0);
        assert!((width_at(INK_FAST_SPEED * 2.0) - INK_FAST_SHARE).abs() < 1e-12);
        let widths: Vec<f64> = (0..100).map(|i| width_at(f64::from(i) * 0.05)).collect();
        assert!(
            widths.windows(2).all(|w| w[1] <= w[0]),
            "faster is never wider"
        );

        // Slow for 100 px, then the hand speeds up for 100 px.
        let mut samples = slow((0.0, 0.0), (100.0, 0.0), 20);
        let start = samples.last().unwrap().t;
        samples.extend(
            (1..=20).map(|i| at(100.0 + f64::from(i) * 5.0, 0.0, start + f64::from(i) * 2.0)),
        );
        let dots = fountain(&samples);
        let radius_near = |x: f64| {
            dots.iter()
                .min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs()))
                .unwrap()
                .radius
        };
        assert!(
            radius_near(160.0) < radius_near(50.0) * 0.8,
            "fast is thinner"
        );
        let biggest_step = dots
            .windows(2)
            .map(|w| (w[1].radius - w[0].radius).abs())
            .fold(0.0, f64::max);
        assert!(
            biggest_step < 0.25,
            "the width eases: {biggest_step:.2}px in a dab"
        );
    }

    #[test]
    fn a_trail_lands_where_the_finger_stopped() {
        let dots = fountain(&slow((0.0, 0.0), (100.0, 40.0), 25));
        let last = dots.last().expect("a trail");
        assert!((last.x - 100.0).abs() < INK_STEP_PX * 2.0, "{last:?}");
        assert!((last.y - 40.0).abs() < INK_STEP_PX * 2.0, "{last:?}");
    }

    #[test]
    fn a_trail_thins_at_both_ends_and_is_fullest_in_the_middle() {
        let nib = Nib::for_pen(Pen::FountainPen);
        let dots = fountain(&slow((0.0, 0.0), (300.0, 0.0), 60));
        let first = dots.first().expect("a trail");
        let last = dots.last().expect("a trail");
        let middle = dots[dots.len() / 2];
        assert!(first.radius < middle.radius, "{:?} vs {:?}", first, middle);
        assert!(last.radius < middle.radius, "{:?} vs {:?}", last, middle);
        assert_eq!(
            middle.radius, nib.radius,
            "a slow body keeps the full width"
        );
        assert!(first.radius >= nib.radius * nib.taper);
        assert!(last.radius >= nib.radius * nib.taper);
    }

    #[test]
    fn a_stroke_shorter_than_one_dab_still_inks_one_dot() {
        let dots = fountain(&[at(0.0, 0.0, 0.0), at(1.0, 0.0, 50.0)]);
        assert_eq!(dots.len(), 1);
        assert!((dots[0].x - 1.0).abs() < 0.01, "{dots:?}");
        assert!(dots[0].radius > 0.0);
    }

    #[test]
    fn every_tool_keeps_a_positive_width_and_its_own_look() {
        let samples = slow((0.0, 0.0), (500.0, 500.0), 500);
        for pen in [
            Pen::FountainPen,
            Pen::Ballpoint,
            Pen::Pencil,
            Pen::Marker,
            Pen::Quill,
        ] {
            let nib = Nib::for_pen(pen);
            assert!(nib.radius > INK_STEP_PX, "{pen:?}: dabs overlap");
            assert!((0.0..=1.0).contains(&nib.taper), "{pen:?}");
            assert!((0.0..=1.0).contains(&nib.alpha), "{pen:?}");
            assert!((0.0..=1.0).contains(&nib.dry_out), "{pen:?}");
            let dots = trail(&samples, nib);
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
        let samples = slow((0.0, 0.0), (300.0, 0.0), 120);
        let nib = Nib::for_pen(Pen::Pencil);
        let pencil = trail(&samples, nib);
        let body = &pencil[pencil.len() / 4..pencil.len() * 3 / 4];
        assert!(
            body.iter().all(|dot| dot.radius == nib.radius),
            "a pencil draws an even width"
        );
        let inks: Vec<f64> = body.iter().map(|dot| dot.alpha).collect();
        let min = inks.iter().cloned().fold(f64::MAX, f64::min);
        let max = inks.iter().cloned().fold(f64::MIN, f64::max);
        assert!(
            min >= nib.alpha * 0.70 && max <= nib.alpha * 1.30,
            "pencil ink {min}..{max}"
        );
        assert!(max > min + 0.05, "the deposit wanders");

        // Ink is steady in both width and deposit.
        let ink = fountain(&samples);
        let body = &ink[ink.len() / 4..ink.len() * 3 / 4];
        assert!(body.iter().all(|dot| dot.alpha == 1.0));
        assert!(body.iter().all(|dot| dot.radius == FOUNTAIN.radius));
    }

    #[test]
    fn a_quills_tail_dries_out() {
        let samples = slow((0.0, 0.0), (300.0, 0.0), 120);
        let quill = trail(&samples, Nib::for_pen(Pen::Quill));
        let head = quill[quill.len() / 4];
        let tip = quill.last().expect("a trail");
        assert!(
            tip.alpha < head.alpha * 0.5,
            "tail {tip:?} vs head {head:?}"
        );
        // And a pen does not dry.
        assert_eq!(fountain(&samples).last().expect("a trail").alpha, 1.0);
    }

    #[test]
    fn only_the_short_tail_is_rambo_tinted() {
        let dots = fountain(&slow((0.0, 0.0), (300.0, 0.0), 120));
        let tinted = dots.iter().filter(|dot| dot.tail).count();
        assert!(tinted > 0, "the tail is his");
        assert!(tinted <= dabs_in(INK_TAIL_PX), "and short: {tinted}");
        assert!(tinted * 6 <= dots.len(), "no more than a sixth: {tinted}");
        // The tint is the end of the stroke, never the start.
        assert_eq!(
            dots.iter().position(|dot| dot.tail),
            Some(dots.len() - tinted)
        );
    }

    #[test]
    fn the_same_stroke_always_draws_the_same_way() {
        // Grain must not be random: the same samples twice are the same trail, or
        // a redraw under the finger would shimmer.
        let samples = line((0.0, 0.0), (200.0, 120.0), 40, 16.0);
        for pen in [Pen::Pencil, Pen::Quill] {
            let stroke = Stroke {
                samples: samples.clone(),
                pen,
            };
            assert_eq!(stroke.dots(), stroke.dots(), "{pen:?}");
        }
    }

    #[test]
    fn a_stroke_keeps_its_tool_and_ages_from_its_last_touch() {
        let mut stroke = Stroke::new(Pen::Marker, at(0.0, 0.0, 100.0));
        stroke.samples.push(at(20.0, 0.0, 180.0));
        assert_eq!(stroke.laid_at(), 180.0);
        let radius = Nib::for_pen(Pen::Marker).radius;
        assert!(stroke.dots().iter().any(|dot| dot.radius > radius * 0.9));
    }

    #[test]
    fn ink_dries_a_shade_lighter_then_fades_away_smoothly() {
        assert_eq!(age_alpha(0.0, 0.0), 1.0, "wet and full");
        let dry = age_alpha(INK_DRY_MS, 0.0);
        assert!((dry - INK_DRY_ALPHA).abs() < 1e-12, "dried: {dry}");
        assert!(
            (age_alpha(INK_HOLD_MS - 1.0, 0.0) - INK_DRY_ALPHA).abs() < 1e-9,
            "it holds"
        );
        assert_eq!(age_alpha(INK_HOLD_MS + INK_FADE_OUT_MS, 0.0), 0.0, "gone");
        assert!(faded(INK_HOLD_MS + INK_FADE_OUT_MS) && !faded(INK_HOLD_MS));
        // Never brighter as it ages, and never a cut: no frame loses more than a
        // sliver of ink.
        let frames: Vec<f64> = (0..=((INK_HOLD_MS + INK_FADE_OUT_MS) / 16.0) as usize)
            .map(|frame| age_alpha(frame as f64 * 16.0, 0.0))
            .collect();
        assert!(frames.windows(2).all(|w| w[1] <= w[0]));
        let biggest = frames.windows(2).map(|w| w[0] - w[1]).fold(0.0, f64::max);
        assert!(biggest < 0.02, "a cut of {biggest:.3} in one frame");
    }

    #[test]
    fn thin_ink_fades_before_thick_ink() {
        let mid_fade = INK_HOLD_MS + INK_FADE_OUT_MS * 0.25;
        assert!(age_alpha(mid_fade, 1.0) < age_alpha(mid_fade, 0.0));
        assert_eq!(age_alpha(INK_HOLD_MS + INK_FADE_OUT_MS, 1.0), 0.0);
        // Before any fade, thin and thick ink look the same.
        assert_eq!(age_alpha(INK_DRY_MS, 1.0), age_alpha(INK_DRY_MS, 0.0));
    }
}

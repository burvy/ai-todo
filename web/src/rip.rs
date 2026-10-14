//! The page rips (DESIGN-DIRECTION §9, "Gestures, final"): a left-to-right swipe on the
//! list page that does not start on a task tears the page a little. The tear is a real
//! hole: the planner page, which lies under the list on a phone, shows through it.
//! Rambo hops over and tapes it; the tape and the tear then fade away (the one thing
//! besides ink that is allowed to fade).
//!
//! Everything here is plain geometry and timing, tested without a browser: whether a
//! swipe is a rip, where the tear may lie so it never crosses a row, its ragged outline,
//! the hole it cuts in the page, where the tapes go, and when each stage of the repair
//! ends. `Ripper` (browser only) reads the pointer and draws it.

use crate::tuning::{
    RIP_EDGE_PX, RIP_GAP_PX, RIP_JAG_PX, RIP_JAG_STEP_PX, RIP_MAX_SLOPE, RIP_MIN_PX, TAPE_AT_MS,
    TAPE_EVERY_PX, TAPE_LAY_MS, TAPE_LEN_PX, TAPE_SKEW_DEG, TEAR_FADE_MS, TEAR_HOLD_MS,
};

pub type Pt = (f64, f64);

/// A tear in the list page, in the page's own coordinates: x from the page's left edge
/// and y from the top of its content, so it scrolls with the rows. It runs from where
/// the finger went down to where the finger is, level, since the paper tears along the
/// swipe.
#[derive(Clone, Debug, PartialEq)]
pub struct Tear {
    pub from: Pt,
    pub to: Pt,
    /// Makes this tear's ragged edge its own, and the same every time it is drawn.
    pub seed: u32,
}

/// Whether a press that has travelled `dx`, `dy` is a rip: far enough to the right and
/// flat enough that it is not a scroll (the list scrolls up and down). A swipe to the
/// left turns the page, and the planner's own swipes are not the list's to claim.
pub fn is_rip(dx: f64, dy: f64) -> bool {
    dx >= RIP_MIN_PX && dy.abs() <= dx * RIP_MAX_SLOPE
}

/// How far from a tear's line the paper may be torn, either way: the hole's widest
/// half plus the ragged edge.
pub fn reach_px() -> f64 {
    RIP_GAP_PX / 2.0 + RIP_JAG_PX
}

/// The level a tear starting at `y` may run along, or `None` when it would touch the
/// words of a row: `rows` are the rows' `(top, bottom)` in the page's coordinates. A
/// tear never covers a row's words, so a press in the narrow space between two rows,
/// or beside one, tears nothing.
pub fn lane(y: f64, rows: &[(f64, f64)]) -> Option<f64> {
    let reach = reach_px();
    rows.iter()
        .all(|&(top, bottom)| y + reach < top || y - reach > bottom)
        .then_some(y)
}

/// How far to the right the tear may reach on a page `width` wide whose pin's column
/// takes `pin` px of its right edge: it stops short of the pin and of the screen.
pub fn right_limit(width: f64, pin: f64) -> f64 {
    width - pin - RIP_EDGE_PX
}

/// A small repeatable number in `-1.0..=1.0`, so a tear's edge is ragged but the same
/// on every frame.
fn unit(seed: u32, i: u32) -> f64 {
    let mut x = seed ^ i.wrapping_mul(0x9E37_79B1);
    x ^= x >> 15;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    f64::from(x) / f64::from(u32::MAX) * 2.0 - 1.0
}

impl Tear {
    /// How far the tear runs, in px.
    pub fn span(&self) -> f64 {
        (self.to.0 - self.from.0).max(0.0)
    }

    /// The tear's outline, a closed ragged slit: the upper lip left to right, then the
    /// lower lip back. It is pinched to a point at each end and opens widest in the
    /// middle, up to `RIP_GAP_PX`; each lip wanders by up to `RIP_JAG_PX`.
    pub fn outline(&self) -> Vec<Pt> {
        let span = self.span();
        if span <= 0.0 {
            return Vec::new();
        }
        let steps = ((span / RIP_JAG_STEP_PX).ceil() as u32).max(2);
        let lips = |side: f64, salt: u32| {
            (0..=steps).map(move |i| {
                let t = f64::from(i) / f64::from(steps);
                let open = RIP_GAP_PX / 2.0 * (std::f64::consts::PI * t).sin().powf(0.6);
                // The ends stay pinched shut however the edge wanders.
                let wander =
                    RIP_JAG_PX * unit(self.seed, i + salt) * (std::f64::consts::PI * t).sin();
                (self.from.0 + span * t, self.from.1 + side * open + wander)
            })
        };
        let upper: Vec<Pt> = lips(-1.0, 0).collect();
        let lower: Vec<Pt> = lips(1.0, 1_000).collect();
        upper.into_iter().chain(lower.into_iter().rev()).collect()
    }

    /// Where the tapes go: one across the tear for every `TAPE_EVERY_PX` of it, evenly
    /// spaced, each a little off square as a hand lays it.
    pub fn tapes(&self) -> Vec<Tape> {
        let span = self.span();
        let count = ((span / TAPE_EVERY_PX).round() as u32).max(1);
        (0..count)
            .map(|i| {
                let share = (f64::from(i) + 0.5) / f64::from(count);
                Tape {
                    at: (self.from.0 + span * share, self.from.1),
                    len: TAPE_LEN_PX,
                    angle_deg: 90.0 + TAPE_SKEW_DEG * unit(self.seed, 2_000 + i),
                }
            })
            .collect()
    }

    /// The middle of the tear: where Rambo stands to mend it.
    pub fn middle(&self) -> Pt {
        (self.from.0 + self.span() / 2.0, self.from.1)
    }
}

/// A strip of tape laid across a tear: its middle, its length and its angle (90 is
/// straight across the tear).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tape {
    pub at: Pt,
    pub len: f64,
    pub angle_deg: f64,
}

/// Whether `p` is inside the closed polygon `outline` (even-odd rule).
pub fn contains(outline: &[Pt], p: Pt) -> bool {
    let mut inside = false;
    let mut j = outline.len().wrapping_sub(1);
    for (i, &(xi, yi)) in outline.iter().enumerate() {
        let (xj, yj) = outline[j];
        if (yi > p.1) != (yj > p.1) && p.0 < (xj - xi) * (p.1 - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// An SVG path for a closed polygon, shifted down by `dy` (the page's scroll, so the
/// hole stays on the paper it was torn in).
pub fn ring_path(outline: &[Pt]) -> String {
    ring(outline, 0.0)
}

fn ring(outline: &[Pt], dy: f64) -> String {
    let mut path = String::new();
    for (i, (x, y)) in outline.iter().enumerate() {
        path.push_str(if i == 0 { "M" } else { "L" });
        path.push_str(&format!("{x:.1} {:.1}", y + dy));
    }
    path.push('Z');
    path
}

/// The CSS `clip-path` that cuts the tear out of a page `width` by `height`: the whole
/// page with the slit left out (even-odd), the page scrolled `scroll` px.
pub fn sheet_path(tear: &Tear, width: f64, height: f64, scroll: f64) -> String {
    format!(
        "path(evenodd, \"M0 0H{width:.1}V{height:.1}H0Z{}\")",
        ring(&tear.outline(), -scroll)
    )
}

/// The repair's clock, in ms since the finger let go. The tape goes on when Rambo has
/// had time to get there, lies for a while, then tape and tear fade together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Torn and open; Rambo is on his way.
    Open,
    /// The tape is going on.
    Taping,
    /// Taped and left alone.
    Taped,
    /// Tape and tear fading out.
    Fading,
    /// The page is whole again.
    Gone,
}

/// The stage `elapsed` ms after the finger let go.
pub fn stage(elapsed: u32) -> Stage {
    let laid = TAPE_AT_MS + TAPE_LAY_MS;
    let faded = laid + TEAR_HOLD_MS;
    match elapsed {
        e if e < TAPE_AT_MS => Stage::Open,
        e if e < laid => Stage::Taping,
        e if e < faded => Stage::Taped,
        e if e < faded + TEAR_FADE_MS => Stage::Fading,
        _ => Stage::Gone,
    }
}

/// When the page is whole again, in ms after the finger let go.
pub const fn total_ms() -> u32 {
    TAPE_AT_MS + TAPE_LAY_MS + TEAR_HOLD_MS + TEAR_FADE_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tear(span: f64) -> Tear {
        Tear {
            from: (40.0, 300.0),
            to: (40.0 + span, 300.0),
            seed: 7,
        }
    }

    #[test]
    fn a_rip_is_a_flat_swipe_to_the_right() {
        assert!(is_rip(RIP_MIN_PX, 0.0));
        assert!(!is_rip(RIP_MIN_PX - 1.0, 0.0), "too short");
        assert!(!is_rip(-80.0, 0.0), "left turns the page");
        assert!(!is_rip(60.0, 60.0), "a diagonal is a scroll");
        assert!(is_rip(100.0, 100.0 * RIP_MAX_SLOPE));
    }

    #[test]
    fn a_tear_never_covers_a_row() {
        let rows = [(100.0, 140.0), (156.0, 196.0)];
        // Between two rows there is no room for the whole hole.
        assert_eq!(lane(148.0, &rows), None);
        // On a row, beside it, or too near its words.
        assert_eq!(lane(120.0, &rows), None);
        assert_eq!(lane(100.0 - reach_px() + 1.0, &rows), None);
        assert_eq!(lane(196.0 + reach_px() - 1.0, &rows), None);
        // Clear paper below the last.
        assert_eq!(
            lane(196.0 + reach_px() + 1.0, &rows),
            Some(196.0 + reach_px() + 1.0)
        );
        assert_eq!(lane(300.0, &[]), Some(300.0));
    }

    #[test]
    fn however_it_is_torn_the_hole_stays_off_every_row() {
        let rows = [(100.0, 140.0)];
        for seed in 0..40 {
            for span in [30.0, 90.0, 220.0] {
                let y = 140.0 + reach_px() + 0.5;
                assert!(lane(y, &rows).is_some());
                let torn = Tear {
                    from: (20.0, y),
                    to: (20.0 + span, y),
                    seed,
                };
                for (_, py) in torn.outline() {
                    assert!(py > 140.0, "seed {seed}, span {span}: edge at {py}");
                }
            }
        }
    }

    #[test]
    fn the_tear_is_a_ragged_slit_pinched_at_both_ends_and_open_in_the_middle() {
        let torn = tear(160.0);
        let outline = torn.outline();
        assert!(outline.len() >= 6);
        let (first, last) = (outline[0], outline[outline.len() / 2 - 1]);
        assert_eq!(
            first, torn.from,
            "the upper lip starts at the finger's start"
        );
        assert!(
            (last.0 - torn.to.0).abs() < 1e-9,
            "and runs to where the finger is"
        );
        // The middle is a real hole: a point on the tear's line is inside it.
        assert!(contains(&outline, torn.middle()));
        // Well beyond the ends or the lips is paper.
        assert!(!contains(&outline, (torn.from.0 - 5.0, 300.0)));
        assert!(!contains(&outline, (torn.to.0 + 5.0, 300.0)));
        let (mx, my) = torn.middle();
        assert!(!contains(&outline, (mx, my + RIP_GAP_PX)));
        // It is ragged, and the same every time.
        let ys: Vec<f64> = outline.iter().map(|p| p.1).collect();
        assert!(ys.windows(2).any(|w| (w[0] - w[1]).abs() > 0.5));
        assert_eq!(outline, torn.outline());
        assert_ne!(outline, Tear { seed: 8, ..torn }.outline());
    }

    #[test]
    fn the_slit_stays_within_the_reach_of_its_line() {
        for seed in 0..40 {
            let torn = Tear {
                seed,
                ..tear(200.0)
            };
            for (x, y) in torn.outline() {
                assert!((y - 300.0).abs() <= reach_px() + 1e-9);
                assert!((torn.from.0..=torn.to.0).contains(&x));
            }
        }
    }

    #[test]
    fn it_opens_as_the_finger_goes() {
        let area = |outline: &[Pt]| {
            outline
                .iter()
                .zip(outline.iter().cycle().skip(1))
                .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
                .sum::<f64>()
                .abs()
                / 2.0
        };
        assert!(area(&tear(120.0).outline()) > area(&tear(60.0).outline()));
        assert!(tear(0.0).outline().is_empty());
    }

    #[test]
    fn a_hole_in_the_page_scrolls_with_the_page() {
        let torn = tear(100.0);
        assert_ne!(
            sheet_path(&torn, 390.0, 800.0, 0.0),
            sheet_path(&torn, 390.0, 800.0, 50.0)
        );
        let path = sheet_path(&torn, 390.0, 800.0, 0.0);
        assert!(path.starts_with("path(evenodd, \"M0 0H390.0V800.0H0Z"));
        assert!(path.ends_with("Z\")"));
        assert_eq!(path.matches('Z').count(), 2, "the page, and the one hole");
    }

    #[test]
    fn a_long_tear_gets_more_tapes_each_across_it() {
        assert_eq!(tear(40.0).tapes().len(), 1);
        let long = tear(300.0);
        let tapes = long.tapes();
        assert!(tapes.len() >= 3);
        for tape in &tapes {
            assert!((long.from.0..=long.to.0).contains(&tape.at.0));
            assert!((tape.angle_deg - 90.0).abs() <= TAPE_SKEW_DEG);
        }
        assert!(tapes.windows(2).all(|w| w[0].at.0 < w[1].at.0));
    }

    #[test]
    fn the_repair_runs_open_taping_taped_fading_gone() {
        let stages: Vec<Stage> = (0..=total_ms()).step_by(50).map(stage).collect();
        assert_eq!(stages.first(), Some(&Stage::Open));
        assert_eq!(stages.last(), Some(&Stage::Gone));
        let mut order = stages.clone();
        order.dedup();
        assert_eq!(
            order,
            [
                Stage::Open,
                Stage::Taping,
                Stage::Taped,
                Stage::Fading,
                Stage::Gone
            ]
        );
        assert_eq!(stage(total_ms() - 1), Stage::Fading);
    }
}

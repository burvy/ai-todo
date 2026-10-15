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
    TAPE_EVERY_PX, TAPE_LAY_MS, TAPE_LEN_PX, TAPE_SKEW_DEG, TEAR_EDGE_IN_PX, TEAR_FADE_MS,
    TEAR_HOLD_MS,
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
    /// How wide it opens, in px: at most `RIP_GAP_PX` in the middle of a slit torn by hand,
    /// all along a strip, where it is a little less than the row's height.
    pub gap: f64,
    /// How it opens along its length.
    pub profile: Profile,
}

/// The shape of a tear's opening along its length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// Torn by hand: pinched shut at both ends, widest in the middle.
    Slit,
    /// A row torn out of the page: full height from end to end, with ragged short ends.
    Strip,
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
    /// A slit torn by hand along the swipe, `RIP_GAP_PX` wide at most.
    pub fn slit(from: Pt, to: Pt, seed: u32) -> Self {
        Tear {
            from,
            to,
            seed,
            gap: RIP_GAP_PX,
            profile: Profile::Slit,
        }
    }

    /// The hole a row leaves when it tears out of the page: a strip along its middle line
    /// from `left` to `right`, as tall as the row (`height`) less its ragged edge and a
    /// px, so the rules of the rows above and below stay whole.
    pub fn out_of_row(left: f64, right: f64, middle: f64, height: f64, seed: u32) -> Self {
        Tear {
            gap: (height - 2.0 * RIP_JAG_PX - 2.0).max(0.0),
            profile: Profile::Strip,
            ..Tear::slit((left, middle), (right, middle), seed)
        }
    }

    /// How far open the tear is, `0.0` to `1.0` of its gap, at `t` of the way along it
    /// (`span` px long).
    fn opening(&self, t: f64, span: f64) -> f64 {
        match self.profile {
            Profile::Slit => (std::f64::consts::PI * t).sin().powf(0.6),
            Profile::Strip => self.ends(t, span).powf(0.5),
        }
    }

    /// How much of the way from a pinched end a point `t` along the tear has come: `0.0` at
    /// the very end of a pinched slit, `1.0` once past the ragged short end of a strip.
    fn ends(&self, t: f64, span: f64) -> f64 {
        match self.profile {
            Profile::Slit => (std::f64::consts::PI * t).sin(),
            Profile::Strip => (t.min(1.0 - t) * span / (2.0 * RIP_JAG_STEP_PX)).min(1.0),
        }
    }

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
                let open = self.gap / 2.0 * self.opening(t, span);
                // The ends stay closed however the edge wanders.
                let wander = RIP_JAG_PX * unit(self.seed, i + salt) * self.ends(t, span);
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

/// A day column of the planner, `left` and `width` measured from the planner page's own
/// left edge, and whether it holds a slip.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub left: f64,
    pub width: f64,
    pub has_slips: bool,
    /// What the hole lines up on, as heights from the planner page's top: the column's hour
    /// lines.
    pub marks: Vec<f64>,
    /// The middle of each of its slips, as heights from the planner page's top.
    pub slips: Vec<f64>,
}

/// The column the hole should show: the first holding a slip, or the first.
fn pick(columns: &[Column]) -> Option<&Column> {
    columns
        .iter()
        .find(|column| column.has_slips)
        .or(columns.first())
}

/// How far left the planner slides while a tear lasts (all in the page's coordinates,
/// `planner_left` being the planner's own left now, `tear_x` where the tear starts). What the
/// hole must show is a day column (the planner's left edge is only the hour labels): the
/// first column holding a slip, or the first column. On a desk, where the pages are side by
/// side, its middle is brought under `tear_x`; with no column to show the planner slides
/// under the list's left edge, `list_left`; it never slides right. On a phone the planner is
/// under the list already, and the column's left edge is brought `TEAR_EDGE_IN_PX` inside
/// the tear's start, either way, so the day's edge, and a slip's edge and first word, cross
/// the slit.
pub fn slide_px(
    torn: bool,
    phone: bool,
    planner_left: f64,
    list_left: f64,
    tear_x: f64,
    columns: &[Column],
) -> f64 {
    if !torn {
        return 0.0;
    }
    if phone {
        return pick(columns).map_or(0.0, |column| {
            planner_left - (tear_x + TEAR_EDGE_IN_PX - column.left)
        });
    }
    let slid_left = pick(columns).map_or(list_left, |column| {
        tear_x - (column.left + column.width / 2.0)
    });
    (planner_left - slid_left).max(0.0)
}

/// How far left the planner slides so that its day grid lies under the whole of a wide hole
/// (a row torn out, from `hole_left` to `hole_right` in the page's coordinates), with
/// `planner_left` its own left now. The grid, from the first column's left edge to the last's
/// right, is kept covering the hole: the planner moves the least that takes, either way, and
/// a grid narrower than the hole is centred on it. No columns measured: it stays.
pub fn slide_over(planner_left: f64, hole_left: f64, hole_right: f64, columns: &[Column]) -> f64 {
    let (Some(first), Some(last)) = (columns.first(), columns.last()) else {
        return 0.0;
    };
    let (grid_left, grid_right) = (first.left, last.left + last.width);
    let (lowest, highest) = (hole_right - grid_right, hole_left - grid_left);
    let stands = if lowest <= highest {
        planner_left.clamp(lowest, highest)
    } else {
        (hole_left + hole_right - grid_left - grid_right) / 2.0
    };
    planner_left - stands
}

/// How far down the planner moves, so that an hour line of the column the hole shows lies
/// across the tear's level `tear_y` instead of between two lines: the nearest mark is
/// brought to the tear. Negative is up; `planner_top` is the planner page's
/// top in the page's coordinates. On a desk it is no more than half the gap between marks.
/// On a phone the planner lies wholly under the list, so it may move as far as it takes to
/// bring the column's nearest slip across the tear, when it has one.
pub fn lift_px(torn: bool, phone: bool, planner_top: f64, tear_y: f64, columns: &[Column]) -> f64 {
    if !torn {
        return 0.0;
    }
    pick(columns)
        .and_then(|column| {
            let marks = match (phone, column.slips.is_empty()) {
                (true, false) => &column.slips,
                _ => &column.marks,
            };
            marks
                .iter()
                .map(|mark| tear_y - (planner_top + mark))
                .min_by(|a, b| a.abs().total_cmp(&b.abs()))
        })
        .unwrap_or(0.0)
}

/// When the page is whole again, in ms after the finger let go.
pub const fn total_ms() -> u32 {
    TAPE_AT_MS + TAPE_LAY_MS + TEAR_HOLD_MS + TEAR_FADE_MS
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn tear(span: f64) -> Tear {
        Tear::slit((40.0, 300.0), (40.0 + span, 300.0), 7)
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
                let torn = Tear::slit((20.0, y), (20.0 + span, y), seed);
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
    fn a_row_torn_out_leaves_a_strip_as_tall_as_the_row_from_end_to_end() {
        // A row 48 px tall from x 20 to 330, its middle at 300.
        for seed in 0..40 {
            let hole = Tear::out_of_row(20.0, 330.0, 300.0, 48.0, seed);
            let outline = hole.outline();
            assert!(!outline.is_empty());
            // It stays inside the row's own height, so the rules above and below stay whole.
            for &(x, y) in &outline {
                assert!((20.0..=330.0).contains(&x));
                assert!(
                    (y - 300.0).abs() <= 24.0 - 1.0 + 1e-9,
                    "seed {seed}: edge at {y}"
                );
            }
            // The upper lip runs left to right, the lower back: over the middle 80 % of the
            // width the strip is open nearly the whole height, not just at its centre.
            let lip = outline.len() / 2;
            let (upper, lower) = (&outline[..lip], &outline[lip..]);
            for i in lip / 10..=lip - 1 - lip / 10 {
                let open = lower[lip - 1 - i].1 - upper[i].1;
                assert!(
                    open >= 39.0 - 2.0 * RIP_JAG_PX,
                    "seed {seed}, point {i}: open {open}"
                );
            }
            // Its ends are short and ragged, not a point: it has opened a short way in.
            let near_end = upper
                .iter()
                .position(|p| p.0 > 20.0 + 3.0 * RIP_JAG_STEP_PX)
                .expect("a point beyond the ragged end");
            assert!(
                lower[lip - 1 - near_end].1 - upper[near_end].1 >= 25.0,
                "seed {seed}"
            );
            // And it is taped like any tear: more than one tape across so wide a hole.
            assert!(hole.tapes().len() >= 2);
        }
    }

    #[test]
    fn a_wide_hole_has_the_whole_day_grid_under_it() {
        let day = |left| Column {
            left,
            width: 128.0,
            has_slips: false,
            marks: Vec::new(),
            slips: Vec::new(),
        };
        // A phone: the planner at 0, hour labels in the first 68 px, three columns to 452.
        let columns = [day(68.0), day(196.0), day(324.0)];
        // A row's hole from 13 to 351: the grid covers it only if the planner moves left.
        let slid = slide_over(0.0, 13.0, 351.0, &columns);
        assert_eq!(
            slid, 55.0,
            "the first column's edge is brought to the hole's left"
        );
        let p = 0.0 - slid;
        assert!(p + 68.0 <= 13.0 && p + 452.0 >= 351.0);
        // Already covering it: it stays.
        assert_eq!(slide_over(-60.0, 13.0, 351.0, &columns), 0.0);
        // A desk, the planner standing far to the right of the list: it slides under all of it.
        let slid = slide_over(473.0, 107.0, 418.0, &columns);
        let p = 473.0 - slid;
        assert!(p + 68.0 <= 107.0 && p + 452.0 >= 418.0, "{p}");
        // A grid narrower than the hole is centred on it; no columns: it stays.
        let narrow = [Column {
            width: 64.0,
            ..day(0.0)
        }];
        let p = 0.0 - slide_over(0.0, 100.0, 400.0, &narrow);
        assert_eq!(p + 32.0, 250.0);
        assert_eq!(slide_over(0.0, 13.0, 351.0, &[]), 0.0);
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
    fn on_a_desk_a_day_column_slides_under_where_the_tear_starts() {
        let day = |left, has_slips| Column {
            left,
            width: 90.0,
            has_slips,
            marks: Vec::new(),
            slips: Vec::new(),
        };
        let columns = [day(60.0, false), day(150.0, true), day(240.0, true)];
        // The spread: planner at 420, list at 20, the tear starting at 150.
        let slid = slide_px(true, false, 420.0, 20.0, 150.0, &columns);
        // The first column with a slip (centre 195 into the planner) lands under x 150.
        assert_eq!(420.0 - slid + 195.0, 150.0);
        // Where the tear is decides it, not a fixed offset.
        let further = slide_px(true, false, 420.0, 20.0, 250.0, &columns);
        assert_eq!(further, slid - 100.0);
        // No slips anywhere: the first column.
        let empty = [day(60.0, false), day(150.0, false)];
        assert_eq!(
            420.0 - slide_px(true, false, 420.0, 20.0, 150.0, &empty) + 105.0,
            150.0
        );
    }

    #[test]
    fn the_planner_only_slides_on_a_desk_while_torn_and_never_to_the_right() {
        let columns = [Column {
            left: 60.0,
            width: 90.0,
            has_slips: false,
            marks: vec![30.0],
            slips: Vec::new(),
        }];
        assert_eq!(
            slide_px(false, false, 420.0, 20.0, 150.0, &columns),
            0.0,
            "whole page"
        );
        // A tear far to the left of where the planner stands would push it right: no.
        assert_eq!(slide_px(true, false, 100.0, 20.0, 400.0, &columns), 0.0);
        // No columns measured: under the list's left edge.
        assert_eq!(slide_px(true, false, 420.0, 20.0, 150.0, &[]), 400.0);
    }

    #[test]
    fn on_a_phone_the_first_column_with_a_slip_has_its_edge_just_inside_the_tear() {
        let day = |left, has_slips| Column {
            left,
            width: 128.0,
            has_slips,
            marks: Vec::new(),
            slips: Vec::new(),
        };
        let columns = [day(68.0, false), day(200.0, true), day(332.0, true)];
        // The planner stands at 0, the tear starts at 200.
        let slid = slide_px(true, true, 0.0, 0.0, 200.0, &columns);
        // The slip column's left edge (200 into the planner) lands TEAR_EDGE_IN_PX inside it.
        assert_eq!(0.0 - slid + 200.0, 200.0 + TEAR_EDGE_IN_PX);
        // Either way: a tear starting left of the column pulls the planner right.
        let left = slide_px(true, true, 0.0, 0.0, 20.0, &columns);
        assert_eq!(0.0 - left + 200.0, 20.0 + TEAR_EDGE_IN_PX);
        // No slips: the first column, today's.
        let empty = [day(68.0, false), day(200.0, false)];
        let slid = slide_px(true, true, 0.0, 0.0, 200.0, &empty);
        assert_eq!(0.0 - slid + 68.0, 200.0 + TEAR_EDGE_IN_PX);
        // Whole page, or no column measured: it stays.
        assert_eq!(slide_px(false, true, 0.0, 0.0, 200.0, &columns), 0.0);
        assert_eq!(slide_px(true, true, 0.0, 0.0, 200.0, &[]), 0.0);
    }

    #[test]
    fn on_a_phone_the_nearest_slip_is_brought_across_the_tear_however_far() {
        let column = |slips: &[f64]| {
            [Column {
                left: 0.0,
                width: 90.0,
                has_slips: !slips.is_empty(),
                marks: (0..8).map(|i| 40.0 + 64.0 * f64::from(i)).collect(),
                slips: slips.to_vec(),
            }]
        };
        // A slip at 300 under a tear at 450 with the planner's top at 100: 50 px down.
        assert_eq!(
            lift_px(true, true, 100.0, 450.0, &column(&[300.0, 900.0])),
            50.0
        );
        // On a desk the same planner only lines an hour up (within half a gap).
        assert!(lift_px(true, false, 100.0, 450.0, &column(&[300.0, 900.0])).abs() <= 32.0);
        // A phone with no slip lines an hour line up.
        assert!(lift_px(true, true, 100.0, 450.0, &column(&[])).abs() <= 32.0);
    }

    #[test]
    fn the_hole_lies_across_an_hour_line_not_between_them() {
        let column = |has_slips, marks: &[f64]| {
            [Column {
                left: 0.0,
                width: 90.0,
                has_slips,
                marks: marks.to_vec(),
                slips: Vec::new(),
            }]
        };
        // Hour lines every 64 px from 40, the planner's top at 100, the tear at 450: the
        // lines lie at 140, 204, ..., 460, and the nearest is 460, 10 px below the tear.
        let lines: Vec<f64> = (0..8).map(|i| 40.0 + 64.0 * f64::from(i)).collect();
        let lift = lift_px(true, false, 100.0, 450.0, &column(false, &lines));
        assert_eq!(lift, -10.0);
        // A slip is chosen over the lines.
        assert_eq!(
            lift_px(true, false, 100.0, 450.0, &column(true, &[300.0])),
            50.0
        );
        // Never more than half the gap.
        for tear_y in 300..400 {
            let lift = lift_px(
                true,
                false,
                100.0,
                f64::from(tear_y),
                &column(false, &lines),
            );
            assert!(lift.abs() <= 32.0, "tear {tear_y}: {lift}");
        }
        // Whole page, or nothing to show: it does not move.
        assert_eq!(
            lift_px(false, false, 100.0, 450.0, &column(false, &lines)),
            0.0
        );
        assert_eq!(lift_px(true, false, 100.0, 450.0, &[]), 0.0);
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
}

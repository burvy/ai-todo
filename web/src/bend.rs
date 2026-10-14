//! The page turn on a phone: the page bends from the finger's corner and follows it.
//!
//! The model is a flat fold. The finger drags a corner `C` of the page to `P`; the page
//! folds along the line halfway between them. What stays flat is the part of the page on
//! `P`'s side of that line; the part on `C`'s side is lifted, laid back over the flat
//! part as the flap (showing the paper's underside), and the next page shows where it
//! used to be. Letting go past a share of the page's width, or in a flick, turns the
//! page; short of that it springs back.
//!
//! This file keeps the geometry and the decisions pure, so they are tested without a
//! browser; `Turner` (browser only) reads the pointer, draws the shapes and animates.

use crate::tuning::{BEND_COMMIT_SHARE, BEND_FLICK_MIN_PX, BEND_FLICK_PX_PER_MS, BEND_SLOP_PX};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub(crate) const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    fn minus(self, other: Pt) -> Pt {
        Pt::new(self.x - other.x, self.y - other.y)
    }
    fn plus(self, other: Pt) -> Pt {
        Pt::new(self.x + other.x, self.y + other.y)
    }
    fn times(self, k: f64) -> Pt {
        Pt::new(self.x * k, self.y * k)
    }
    fn dot(self, other: Pt) -> f64 {
        self.x * other.x + self.y * other.y
    }
    fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
}

/// Which way the page is turning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Turn {
    /// The list turns away to the planner: the corner is on the right edge and the
    /// finger travels left.
    Forward,
    /// The planner turns back to the list: the corner is on the left edge and the
    /// finger travels right.
    Back,
}

impl Turn {
    /// The pane that turns, 0 for the list and 1 for the planner.
    pub(crate) fn turning_pane(self) -> u8 {
        match self {
            Turn::Forward => 0,
            Turn::Back => 1,
        }
    }
    pub(crate) fn revealed_pane(self) -> u8 {
        1 - self.turning_pane()
    }
    /// +1 when a finger moving right helps the turn, -1 when moving left does.
    pub(crate) fn inward(self) -> f64 {
        match self {
            Turn::Forward => -1.0,
            Turn::Back => 1.0,
        }
    }
}

/// Which turn a press that has travelled `dx`, `dy` is asking for, once it has left
/// the slop and is more sideways than up or down: a left swipe on the list, a right
/// swipe on the planner. Any other swipe is not a page turn.
pub(crate) fn turn_for(pane: u8, dx: f64, dy: f64) -> Option<Turn> {
    if dx.hypot(dy) < BEND_SLOP_PX || dx.abs() <= dy.abs() {
        return None;
    }
    match (pane, dx < 0.0) {
        (0, true) => Some(Turn::Forward),
        (1, false) => Some(Turn::Back),
        _ => None,
    }
}

/// What letting go does: `reach` is how far the finger went the way the turn goes
/// (negative if it came back), `velocity` the same way in px per ms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Settle {
    Complete,
    SpringBack,
}

pub(crate) fn settle(reach: f64, velocity: f64, page_width: f64) -> Settle {
    let far_enough = reach >= page_width * BEND_COMMIT_SHARE;
    let flicked = reach >= BEND_FLICK_MIN_PX && velocity >= BEND_FLICK_PX_PER_MS;
    if far_enough || flicked {
        Settle::Complete
    } else {
        Settle::SpringBack
    }
}

/// The page's shapes for one frame, in the page's own pixels.
#[derive(Clone, Debug)]
pub(crate) struct Bend {
    /// What stays flat of the turning page.
    pub kept: Vec<Pt>,
    /// Where the turning page has left, so the next page shows (and takes the shadow).
    pub away: Vec<Pt>,
    /// The lifted part laid back over the flat part, underside up.
    pub flap: Vec<Pt>,
    /// A point on the fold line.
    pub fold: Pt,
    /// The direction from the corner to the finger, in radians.
    pub angle: f64,
}

/// Keeps the part of `poly` on the side of the line through `at` that `normal` points to.
fn clip(poly: &[Pt], at: Pt, normal: Pt) -> Vec<Pt> {
    let side = |p: Pt| p.minus(at).dot(normal);
    let mut out = Vec::new();
    for (i, &a) in poly.iter().enumerate() {
        let b = poly[(i + 1) % poly.len()];
        let (sa, sb) = (side(a), side(b));
        if sa >= 0.0 {
            out.push(a);
        }
        if (sa >= 0.0) != (sb >= 0.0) {
            out.push(a.plus(b.minus(a).times(sa / (sa - sb))));
        }
    }
    out
}

fn page_rect(w: f64, h: f64) -> Vec<Pt> {
    vec![
        Pt::new(0.0, 0.0),
        Pt::new(w, 0.0),
        Pt::new(w, h),
        Pt::new(0.0, h),
    ]
}

/// The shapes when the finger has dragged `corner` to `finger` on a `w` by `h` page, or
/// `None` while they are too close to fold at all.
pub(crate) fn bend(w: f64, h: f64, corner: Pt, finger: Pt) -> Option<Bend> {
    let drag = finger.minus(corner);
    let length = drag.len();
    if length < 0.5 {
        return None;
    }
    let normal = drag.times(1.0 / length);
    let fold = corner.plus(drag.times(0.5));
    let page = page_rect(w, h);
    let kept = clip(&page, fold, normal);
    let away = clip(&page, fold, normal.times(-1.0));
    // Lifted over the fold line, the away part lands on the flat side; the page's own
    // edges cut off what would overhang.
    let mut flap: Vec<Pt> = away
        .iter()
        .map(|&p| p.minus(normal.times(2.0 * p.minus(fold).dot(normal))))
        .collect();
    for (edge_at, inward) in [
        (Pt::new(0.0, 0.0), Pt::new(1.0, 0.0)),
        (Pt::new(w, 0.0), Pt::new(-1.0, 0.0)),
        (Pt::new(0.0, 0.0), Pt::new(0.0, 1.0)),
        (Pt::new(0.0, h), Pt::new(0.0, -1.0)),
    ] {
        if flap.is_empty() {
            break;
        }
        flap = clip(&flap, edge_at, inward);
    }
    Some(Bend {
        kept,
        away,
        flap,
        fold,
        angle: normal.y.atan2(normal.x),
    })
}

/// The finger's drag, limited to what turns the page over completely, so a settle never
/// has to chase a finger that is off the screen.
pub(crate) fn limited(drag: Pt, page_width: f64, full_turn: f64) -> Pt {
    let most = page_width * full_turn;
    let length = drag.len();
    if length > most {
        drag.times(most / length)
    } else {
        drag
    }
}

/// Where a drag of `from` has got to `t` (0 to 1) of the way to `to`, eased out: quick
/// off the mark, gentle in, the way paper settles.
pub(crate) fn eased(from: Pt, to: Pt, t: f64) -> Pt {
    let t = t.clamp(0.0, 1.0);
    let ease = 1.0 - (1.0 - t).powi(4);
    from.plus(to.minus(from).times(ease))
}

/// Polygon points as a CSS `polygon(...)`, or an empty clip when there is none.
pub(crate) fn polygon_css(points: &[Pt]) -> String {
    if points.len() < 3 {
        return "inset(100%)".to_owned();
    }
    let list: Vec<String> = points
        .iter()
        .map(|p| format!("{:.1}px {:.1}px", p.x, p.y))
        .collect();
    format!("polygon({})", list.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(poly: &[Pt]) -> f64 {
        let twice: f64 = poly
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let b = poly[(i + 1) % poly.len()];
                a.x * b.y - b.x * a.y
            })
            .sum();
        (twice / 2.0).abs()
    }

    const W: f64 = 400.0;
    const H: f64 = 700.0;

    #[test]
    fn a_fold_splits_the_page_with_nothing_lost() {
        let b = bend(W, H, Pt::new(W, H), Pt::new(W - 120.0, H - 40.0)).expect("a fold");
        assert!((area(&b.kept) + area(&b.away) - W * H).abs() < 1e-6);
    }

    #[test]
    fn a_straight_drag_folds_halfway_between_the_corner_and_the_finger() {
        let b = bend(W, H, Pt::new(W, 0.0), Pt::new(W - 160.0, 0.0)).expect("a fold");
        assert!(
            (area(&b.kept) - (W - 80.0) * H).abs() < 1e-6,
            "flat part ends at x = 320"
        );
        // The flap is the 80px strip folded back over the flat part.
        assert!((area(&b.flap) - 80.0 * H).abs() < 1e-6);
        assert!(
            b.flap
                .iter()
                .all(|p| p.x >= 240.0 - 1e-9 && p.x <= 320.0 + 1e-9)
        );
    }

    #[test]
    fn the_flap_lies_on_the_flat_side_and_inside_the_page() {
        let b = bend(W, H, Pt::new(W, H), Pt::new(40.0, H - 200.0)).expect("a fold");
        for p in &b.flap {
            assert!(p.x >= -1e-6 && p.x <= W + 1e-6 && p.y >= -1e-6 && p.y <= H + 1e-6);
        }
        assert!(
            area(&b.flap) <= area(&b.away) + 1e-6,
            "an overhang is cut, never grown"
        );
    }

    #[test]
    fn dragged_far_enough_the_page_is_gone() {
        let b = bend(W, H, Pt::new(W, 0.0), Pt::new(W - 2.05 * W, 0.0)).expect("a fold");
        assert!(b.kept.len() < 3 || area(&b.kept) < 1e-6);
        assert!(
            b.flap.len() < 3 || area(&b.flap) < 1e-6,
            "the flap has left the page too"
        );
    }

    #[test]
    fn the_left_corner_turns_the_other_way() {
        let b = bend(W, H, Pt::new(0.0, H), Pt::new(100.0, H)).expect("a fold");
        assert!(
            (area(&b.kept) - (W - 50.0) * H).abs() < 1e-6,
            "flat part starts at x = 50"
        );
    }

    #[test]
    fn a_finger_that_has_not_moved_folds_nothing() {
        assert!(bend(W, H, Pt::new(W, H), Pt::new(W, H)).is_none());
        assert!(bend(W, H, Pt::new(W, H), Pt::new(W - 0.2, H)).is_none());
    }

    #[test]
    fn only_a_sideways_swipe_the_right_way_turns_the_page() {
        assert_eq!(turn_for(0, -30.0, 4.0), Some(Turn::Forward));
        assert_eq!(turn_for(1, 30.0, -4.0), Some(Turn::Back));
        assert_eq!(
            turn_for(0, 30.0, 4.0),
            None,
            "right on the list is not a turn"
        );
        assert_eq!(
            turn_for(1, -30.0, 4.0),
            None,
            "left on the last page goes nowhere"
        );
        assert_eq!(turn_for(0, -4.0, 30.0), None, "up and down is a scroll");
        assert_eq!(
            turn_for(0, -2.0, 1.0),
            None,
            "inside the slop it is still a press"
        );
    }

    #[test]
    fn letting_go_past_the_share_or_in_a_flick_turns_the_page() {
        assert_eq!(
            settle(W * BEND_COMMIT_SHARE + 1.0, 0.0, W),
            Settle::Complete
        );
        assert_eq!(
            settle(W * BEND_COMMIT_SHARE - 1.0, 0.0, W),
            Settle::SpringBack
        );
        assert_eq!(
            settle(BEND_FLICK_MIN_PX, BEND_FLICK_PX_PER_MS, W),
            Settle::Complete
        );
        assert_eq!(
            settle(BEND_FLICK_MIN_PX - 1.0, 5.0, W),
            Settle::SpringBack,
            "too short to be a flick"
        );
        assert_eq!(
            settle(-50.0, 5.0, W),
            Settle::SpringBack,
            "a finger that came back"
        );
    }

    #[test]
    fn the_drag_is_limited_and_the_settle_is_eased_out() {
        let far = limited(Pt::new(-5000.0, 0.0), W, 2.0);
        assert!((far.x + 2.0 * W).abs() < 1e-9);
        let near = limited(Pt::new(-30.0, 4.0), W, 2.0);
        assert_eq!(near, Pt::new(-30.0, 4.0));
        let (from, to) = (Pt::new(0.0, 0.0), Pt::new(100.0, 0.0));
        assert_eq!(eased(from, to, 0.0), from);
        assert_eq!(eased(from, to, 1.0), to);
        assert!(
            eased(from, to, 0.5).x > 50.0,
            "eased out: more than half done at half time"
        );
    }

    #[test]
    fn each_turn_names_the_pane_that_turns_and_the_one_it_shows() {
        assert_eq!(
            (Turn::Forward.turning_pane(), Turn::Forward.revealed_pane()),
            (0, 1)
        );
        assert_eq!(
            (Turn::Back.turning_pane(), Turn::Back.revealed_pane()),
            (1, 0)
        );
        assert_eq!((Turn::Forward.inward(), Turn::Back.inward()), (-1.0, 1.0));
    }

    #[test]
    fn the_fold_line_runs_through_the_midpoint_across_the_drag() {
        let b = bend(W, H, Pt::new(W, 0.0), Pt::new(W - 100.0, 0.0)).expect("a fold");
        assert_eq!(b.fold, Pt::new(W - 50.0, 0.0));
        assert!(
            (b.angle.abs() - std::f64::consts::PI).abs() < 1e-9,
            "the drag points left"
        );
    }

    #[test]
    fn a_polygon_is_written_for_css_and_nothing_clips_everything() {
        let css = polygon_css(&[Pt::new(0.0, 0.0), Pt::new(10.0, 0.0), Pt::new(10.0, 5.5)]);
        assert_eq!(css, "polygon(0.0px 0.0px, 10.0px 0.0px, 10.0px 5.5px)");
        assert_eq!(polygon_css(&[]), "inset(100%)");
    }
}

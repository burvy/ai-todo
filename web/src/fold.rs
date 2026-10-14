//! The fast page turn on a phone (the gear's `Page turn: Fast`): the page bends from the
//! finger's corner and follows it. The book flip is `flip.rs`.
//!
//! The model is a flat fold. The finger drags a corner `C` of the page to `P`; the page
//! folds along the line halfway between them. What stays flat is the part of the page on
//! `P`'s side of that line; the part on `C`'s side is lifted, laid back over the flat
//! part as the flap (showing the paper's underside), and the next page shows where it
//! used to be. Letting go past a share of the page's width, or in a flick, turns the
//! page; short of that it springs back.
//!
//! This file keeps the geometry and the decisions pure, so they are tested without a
//! browser; `fold_turner` (browser only) reads the pointer, draws the shapes and animates.

use crate::tuning::{
    BEND_BOUNCE, BEND_BOUNCE_AT, BEND_COMMIT_SHARE, BEND_FLICK_MIN_PX, BEND_FLICK_PX_PER_MS,
    BEND_ROLL_MAX_PX, BEND_ROLL_MIN_PX, BEND_ROLL_PER_PX, BEND_SLOP_PX,
};

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
    pub(crate) fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
}

/// Which way the page is turning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Turn {
    /// The list turns away to the planner: the corner is on the right edge and the
    /// finger travels left.
    Forward,
    /// The list turns back from the planner: the forward turn in reverse. The same leaf
    /// and the same corner, so the finger travels right and the fold unwinds.
    Back,
}

impl Turn {
    /// The pane that turns: always the list, 0, forward or back.
    pub(crate) fn turning_pane(self) -> u8 {
        0
    }
    pub(crate) fn revealed_pane(self) -> u8 {
        1
    }
    /// Where the corner's drag starts and where it ends when the turn completes, along
    /// the page's width `w` (the corner's drag is 0 with the page flat and `-full * w`
    /// with it turned right over). Back runs from the one to the other.
    pub(crate) fn start_drag(self, w: f64, full: f64) -> f64 {
        match self {
            Turn::Forward => 0.0,
            Turn::Back => -full * w,
        }
    }
    pub(crate) fn end_drag(self, w: f64, full: f64) -> f64 {
        match self {
            Turn::Forward => -full * w,
            Turn::Back => 0.0,
        }
    }
    /// The pane showing once the turn has ended.
    pub(crate) fn lands_on(self, outcome: Settle) -> u8 {
        match (self, outcome) {
            (Turn::Forward, Settle::Complete) | (Turn::Back, Settle::SpringBack) => 1,
            (Turn::Forward, Settle::SpringBack) | (Turn::Back, Settle::Complete) => 0,
        }
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
/// swipe on the planner (which brings the list back). Any other swipe is not a page turn.
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

/// How a flight of the corner is paced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ease {
    /// The system finishing what a finger started: quick off the mark, gentle in.
    Out,
    /// Nobody holds the page, so it is lifted and laid down: slow, quick, slow.
    InOut,
    /// A page let go of short of the turn: it falls flat, then rebounds a little.
    Fall,
}

impl Ease {
    /// How much of the flight is done at `t` (0 to 1) of its time: 0 to 1 for the
    /// first two, and for `Fall` the share of the way back to flat, which is exactly 1
    /// when the page first lies flat, drops back by `BEND_BOUNCE` and is 1 again at the end.
    pub(crate) fn at(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Out => 1.0 - (1.0 - t).powi(4),
            Ease::InOut if t < 0.5 => 4.0 * t.powi(3),
            Ease::InOut => 1.0 - (2.0 - 2.0 * t).powi(3) / 2.0,
            Ease::Fall if t < BEND_BOUNCE_AT => 1.0 - (1.0 - t / BEND_BOUNCE_AT).powi(2),
            Ease::Fall => {
                let u = (t - BEND_BOUNCE_AT) / (1.0 - BEND_BOUNCE_AT);
                1.0 - BEND_BOUNCE * 4.0 * u * (1.0 - u)
            }
        }
    }
}

/// The point `e` (0 to 1) of the way from `from` to `to`.
pub(crate) fn along(from: Pt, to: Pt, e: f64) -> Pt {
    from.plus(to.minus(from).times(e))
}

/// A button's turn lifts the corner on its way and sets it down at the end: `arc` of the
/// page's height `h` at most, in step with how far (`e`) the corner has travelled, not
/// with the clock, so the fold keeps the same lean however slowly the turn starts.
pub(crate) fn arced(at: Pt, e: f64, arc: f64, h: f64) -> Pt {
    Pt::new(at.x, at.y - arc * h * 4.0 * e * (1.0 - e))
}

/// How wide the curl at the fold is for a corner dragged `len` px: tight at the start of a
/// turn, looser later.
pub(crate) fn roll_px(len: f64) -> f64 {
    (len * BEND_ROLL_PER_PX).clamp(BEND_ROLL_MIN_PX, BEND_ROLL_MAX_PX)
}

/// The drag with the corner lifted: it rises `lift` px for each px it travels sideways
/// (never more than `cap` of the page's height `h`), toward the page's middle from a
/// bottom corner (`low`) and from a top one alike. The fold stands square to this, so it
/// leans and the flap grows out of the corner instead of across the whole page.
pub(crate) fn lifted(drag: Pt, low: bool, h: f64, lift: f64, cap: f64) -> Pt {
    let rise = (lift * drag.x.abs()).min(cap * h);
    Pt::new(drag.x, drag.y + if low { -rise } else { rise })
}

/// `poly` moved `by` px along the direction `angle` (radians).
fn shifted(poly: &[Pt], angle: f64, by: f64) -> Vec<Pt> {
    let step = Pt::new(angle.cos() * by, angle.sin() * by);
    poly.iter().map(|&p| p.plus(step)).collect()
}

impl Bend {
    /// The flap's shadow on the page below: the flap pushed `by` px away from the fold and
    /// cut to the kept side, so it never falls on the part the page has left, which already
    /// has the cast shadow.
    pub(crate) fn flap_shadow(&self, by: f64) -> Vec<Pt> {
        let normal = Pt::new(self.angle.cos(), self.angle.sin());
        clip(&shifted(&self.flap, self.angle, by), self.fold, normal)
    }
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
        for ease in [Ease::Out, Ease::InOut, Ease::Fall] {
            assert_eq!(along(from, to, ease.at(0.0)), from);
            assert_eq!(along(from, to, ease.at(1.0)), to);
        }
        assert!(
            along(from, to, Ease::Out.at(0.5)).x > 50.0,
            "eased out: more than half done at half time"
        );
    }

    #[test]
    fn a_button_turn_is_slow_at_both_ends() {
        assert!(Ease::InOut.at(0.2) < 0.1, "slow to lift");
        assert!(Ease::InOut.at(0.8) > 0.9, "slow to lay down");
        assert!((Ease::InOut.at(0.5) - 0.5).abs() < 1e-9);
        let steps: Vec<f64> = (0..=20)
            .map(|i| Ease::InOut.at(f64::from(i) / 20.0))
            .collect();
        assert!(
            steps.windows(2).all(|pair| pair[0] <= pair[1]),
            "never backwards"
        );
    }

    #[test]
    fn a_released_page_falls_flat_and_rebounds_a_little() {
        assert!(
            (Ease::Fall.at(BEND_BOUNCE_AT) - 1.0).abs() < 1e-9,
            "flat first"
        );
        let rebound = Ease::Fall.at((1.0 + BEND_BOUNCE_AT) / 2.0);
        assert!(
            (1.0 - rebound - BEND_BOUNCE).abs() < 1e-9,
            "lifts again by the bounce, then settles"
        );
        assert!(Ease::Fall.at(0.4) < Ease::Fall.at(0.6), "falls in order");
    }

    #[test]
    fn a_sideways_swipe_leans_the_fold_off_vertical() {
        let (w, h) = (390.0, 800.0);
        let corner = Pt::new(w, h);
        let drag = lifted(
            Pt::new(-100.0, 0.0),
            true,
            h,
            crate::tuning::BEND_LIFT,
            crate::tuning::BEND_LIFT_CAP,
        );
        let shapes = bend(w, h, corner, corner.plus(drag)).expect("a fold");
        // The angle is the fold's normal, so its lean from horizontal is the fold's
        // lean from vertical.
        let lean = shapes
            .angle
            .sin()
            .abs()
            .atan2(shapes.angle.cos().abs())
            .to_degrees();
        assert!(lean > 25.0, "fold leans {lean} degrees");
    }

    #[test]
    fn the_lift_is_capped_and_mirrors_for_a_top_corner() {
        let far = lifted(Pt::new(-2000.0, 0.0), true, 800.0, 0.5, 0.4);
        assert_eq!(far, Pt::new(-2000.0, -320.0));
        let top = lifted(Pt::new(100.0, 3.0), false, 800.0, 0.5, 0.4);
        assert_eq!(top, Pt::new(100.0, 53.0));
        assert_eq!(
            lifted(Pt::new(0.0, 0.0), true, 800.0, 0.5, 0.4),
            Pt::new(0.0, 0.0)
        );
    }

    #[test]
    fn a_shifted_polygon_moves_along_the_angle() {
        let moved = shifted(&[Pt::new(1.0, 1.0)], std::f64::consts::FRAC_PI_2, 5.0);
        assert!((moved[0].x - 1.0).abs() < 1e-9 && (moved[0].y - 6.0).abs() < 1e-9);
    }

    #[test]
    fn a_buttons_fold_stays_within_forty_degrees_of_vertical() {
        use crate::tuning::{
            BEND_BUTTON_ARC, BEND_BUTTON_LIFT, BEND_BUTTON_LIFT_CAP, BEND_FULL_TURN,
        };
        // A forward turn from the bottom corner, as `turn_to` runs it, at every share of
        // the way and at every pace the ease can give.
        let (w, h) = (390.0, 800.0);
        let corner = Pt::new(w, h);
        let end = Pt::new(-BEND_FULL_TURN * w, 0.0);
        for ease in [Ease::InOut, Ease::Out] {
            for step in 1..=100 {
                let e = ease.at(f64::from(step) / 100.0);
                let at = arced(along(Pt::new(0.0, 0.0), end, e), e, BEND_BUTTON_ARC, h);
                let drag = lifted(at, true, h, BEND_BUTTON_LIFT, BEND_BUTTON_LIFT_CAP);
                let Some(shapes) = bend(w, h, corner, corner.plus(drag)) else {
                    continue;
                };
                let lean = shapes
                    .angle
                    .sin()
                    .abs()
                    .atan2(shapes.angle.cos().abs())
                    .to_degrees();
                assert!(
                    lean <= 40.0,
                    "{ease:?} at {step}%: fold leans {lean} degrees"
                );
            }
        }
    }

    #[test]
    fn the_curl_is_tight_at_first_and_looser_later() {
        assert_eq!(roll_px(10.0), BEND_ROLL_MIN_PX);
        assert_eq!(roll_px(5000.0), BEND_ROLL_MAX_PX);
        assert!(roll_px(150.0) < roll_px(300.0));
    }

    #[test]
    fn the_flaps_shadow_never_falls_on_the_part_the_page_has_left() {
        let (w, h) = (390.0, 800.0);
        let corner = Pt::new(0.0, h);
        let shapes = bend(w, h, corner, Pt::new(80.0, h - 100.0)).expect("a fold");
        let normal = Pt::new(shapes.angle.cos(), shapes.angle.sin());
        let shadow = shapes.flap_shadow(11.0);
        assert!(!shadow.is_empty());
        assert!(
            shadow
                .iter()
                .all(|p| p.minus(shapes.fold).dot(normal) >= -1e-6)
        );
    }

    #[test]
    fn the_turn_back_runs_the_forward_turn_the_other_way() {
        let (w, full) = (W, 2.05);
        assert_eq!(
            Turn::Back.start_drag(w, full),
            Turn::Forward.end_drag(w, full)
        );
        assert_eq!(
            Turn::Back.end_drag(w, full),
            Turn::Forward.start_drag(w, full)
        );
        for turn in [Turn::Forward, Turn::Back] {
            assert_eq!((turn.turning_pane(), turn.revealed_pane()), (0, 1));
        }
        assert_eq!(Turn::Forward.lands_on(Settle::Complete), 1);
        assert_eq!(Turn::Forward.lands_on(Settle::SpringBack), 0);
        assert_eq!(Turn::Back.lands_on(Settle::Complete), 0);
        assert_eq!(Turn::Back.lands_on(Settle::SpringBack), 1);
    }

    #[test]
    fn each_turn_names_the_pane_that_turns_and_the_one_it_shows() {
        assert_eq!(
            (Turn::Forward.turning_pane(), Turn::Forward.revealed_pane()),
            (0, 1)
        );
        assert_eq!(
            (Turn::Back.turning_pane(), Turn::Back.revealed_pane()),
            (0, 1),
            "the list is the leaf both ways"
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

    /// The points a `polygon(...)` string names, read back.
    fn points_of(css: &str) -> Vec<Pt> {
        css.strip_prefix("polygon(")
            .and_then(|rest| rest.strip_suffix(')'))
            .expect("a polygon")
            .split(", ")
            .map(|pair| {
                let mut numbers = pair
                    .split(' ')
                    .map(|n| n.trim_end_matches("px").parse::<f64>().expect("a number"));
                Pt::new(numbers.next().unwrap(), numbers.next().unwrap())
            })
            .collect()
    }

    #[test]
    fn a_polygon_clips_to_the_shape_it_was_made_from() {
        let shapes = bend(W, H, Pt::new(W, H), Pt::new(W - 120.0, H - 40.0)).expect("a fold");
        for shape in [&shapes.kept, &shapes.away] {
            let back = points_of(&polygon_css(shape));
            assert_eq!(back.len(), shape.len());
            assert!(
                back.iter()
                    .zip(shape.iter())
                    .all(|(a, b)| a.minus(*b).len() < 0.06),
                "within the one decimal the string keeps"
            );
        }
    }

    #[test]
    fn nothing_to_clip_to_hides_the_whole_page() {
        assert!(
            polygon_css(&[]).starts_with("inset("),
            "an empty shape is not a polygon with no points, which would show everything"
        );
    }
}

//! The page turn on a phone: the whole page turns like a book's, hinged on its left edge.
//!
//! The list is the leaf and the planner is the page under it. Forward, the leaf's right
//! edge lifts and swings over the spine on the left, and the planner shows where it was;
//! back, the leaf swings in from the spine and lands on the planner again. Letting go past
//! a share of the page's width, or in a flick, finishes the turn; short of that the leaf
//! falls back.
//!
//! The whole turn is one number: `open`, 0 with the leaf lying flat and 1 with it standing
//! edge-on at the spine, where nothing of it can be seen (the spine is the screen's left
//! edge, so the leaf's back never comes into view). The finger's edge keeps pace with the
//! drag: grabbed there, the edge is where the finger is.
//!
//! The leaf moves like a real one: `Spring` carries `open` with a speed of its own, so a
//! page let go of mid-flick carries on at that speed, and one dropped short falls, meets the
//! table and rebounds. This file keeps the geometry, the spring and the decisions pure, so
//! they are tested without a browser; `flip_turner` (browser only) reads the pointer, draws
//! the leaf and steps the spring each frame.

use crate::tuning::{
    BEND_COMMIT_SHARE, BEND_FLICK_MIN_PX, BEND_FLICK_PX_PER_MS, BEND_SLOP_PX, FLIP_WALL_BOUNCE,
};

/// Which way the page is turning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Turn {
    /// The list swings away over the spine to show the planner: the finger travels left.
    Forward,
    /// The list swings back in from the spine onto the planner: the finger travels right.
    Back,
}

impl Turn {
    /// The leaf is always the list, pane 0; the planner is the page under it.
    pub(crate) fn turning_pane(self) -> u8 {
        0
    }
    pub(crate) fn revealed_pane(self) -> u8 {
        1
    }
    /// +1 when a finger moving right helps the turn, -1 when moving left does.
    pub(crate) fn inward(self) -> f64 {
        match self {
            Turn::Forward => -1.0,
            Turn::Back => 1.0,
        }
    }
    /// +1 when `open` grows as the turn goes on, -1 when it shrinks.
    pub(crate) fn open_dir(self) -> f64 {
        -self.inward()
    }
    /// `open` when a drag has carried the finger `share` (0 to 1) of the page's width the
    /// way the turn goes.
    pub(crate) fn open_at(self, share: f64) -> f64 {
        let share = share.clamp(0.0, 1.0);
        match self {
            Turn::Forward => share,
            Turn::Back => 1.0 - share,
        }
    }
    /// The `open` a finished turn ends on, and the one a turn that fell back ends on.
    pub(crate) fn open_after(self, outcome: Settle) -> f64 {
        match (self, outcome) {
            (Turn::Forward, Settle::Complete) | (Turn::Back, Settle::SpringBack) => 1.0,
            (Turn::Forward, Settle::SpringBack) | (Turn::Back, Settle::Complete) => 0.0,
        }
    }
    /// The pane showing once the turn has ended.
    pub(crate) fn lands_on(self, outcome: Settle) -> u8 {
        if self.open_after(outcome) > 0.5 { 1 } else { 0 }
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

/// How `open` moves once nobody holds the leaf: a damped spring with a speed, pulled toward
/// where the turn ends. The table (`open` 0) and the spine (`open` 1) are walls: the leaf
/// meets them and rebounds a little, never passes through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spring {
    pub(crate) x: f64,
    /// Speed, in `open` per second.
    pub(crate) v: f64,
}

/// How a spring pulls: `omega` is its quickness (radians per second), `zeta` its damping
/// (1 settles without overshoot; below that the leaf swings past its end and back).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pull {
    pub(crate) omega: f64,
    pub(crate) zeta: f64,
}

/// The longest slice of time stepped in one go, in seconds: a slow frame is split so the
/// spring stays stable.
const SUBSTEP: f64 = 1.0 / 240.0;
/// How near its end, in `open`, and how slow, in `open` per second, the leaf is at rest.
const REST_DISTANCE: f64 = 0.002;
const REST_SPEED: f64 = 0.02;

impl Spring {
    /// The leaf at `x` (0 to 1) moving at `v`.
    pub(crate) fn new(x: f64, v: f64) -> Self {
        Spring {
            x: x.clamp(0.0, 1.0),
            v,
        }
    }

    /// Moves on by `dt` seconds toward `target`.
    pub(crate) fn step(&mut self, target: f64, pull: Pull, dt: f64) {
        let mut left = dt.clamp(0.0, 0.1);
        while left > 0.0 {
            let h = left.min(SUBSTEP);
            left -= h;
            let accel = -pull.omega * pull.omega * (self.x - target)
                - 2.0 * pull.zeta * pull.omega * self.v;
            self.v += accel * h;
            self.x += self.v * h;
            if !(0.0..=1.0).contains(&self.x) {
                self.x = self.x.clamp(0.0, 1.0);
                self.v *= -FLIP_WALL_BOUNCE;
            }
        }
    }

    pub(crate) fn at_rest(&self, target: f64) -> bool {
        (self.x - target).abs() < REST_DISTANCE && self.v.abs() < REST_SPEED
    }
}

/// Where the leaf's free edge is on the screen, in px from the spine, for a page `width`
/// px wide swung to `angle`, with the viewer `distance` px in front of the spine: the
/// edge comes toward the viewer as it swings, so it is carried a little outward.
pub(crate) fn edge_px(width: f64, angle: f64, distance: f64) -> f64 {
    let toward_viewer = width * angle.sin();
    width * angle.cos() * distance / (distance - toward_viewer)
}

/// How far the leaf has swung, in radians, when it is `open` (0 to 1): the angle at which
/// its free edge, seen flat on, is `1 - open` of the page's width from the spine, so a
/// finger holding that edge keeps pace with it. (The viewer's perspective carries the edge
/// a little outward, `edge_px`; solving for that exactly would swing the leaf by a third of
/// a turn at the first pixel of travel, so the drag is mapped flat on.)
pub(crate) fn angle(open: f64) -> f64 {
    (1.0 - open.clamp(0.0, 1.0)).acos()
}

/// The light on the leaf and the shadow it throws on the page under it, 0 to 1 each.
/// The leaf darkens as it turns away from the light; the shadow on the page below is
/// strongest half way over and is gone when the leaf lies flat or stands edge-on.
pub(crate) fn lighting(angle: f64) -> (f64, f64) {
    (1.0 - angle.cos(), (2.0 * angle).sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f64 = 390.0;
    const FAR: f64 = 1e9;

    #[test]
    fn a_left_swipe_turns_the_list_and_a_right_swipe_turns_back_from_the_planner() {
        assert_eq!(turn_for(0, -30.0, 4.0), Some(Turn::Forward));
        assert_eq!(turn_for(1, 30.0, 4.0), Some(Turn::Back));
        assert_eq!(turn_for(0, 30.0, 4.0), None);
        assert_eq!(turn_for(1, -30.0, 4.0), None);
        assert_eq!(
            turn_for(0, -30.0, 40.0),
            None,
            "more up than across is a scroll"
        );
        assert_eq!(turn_for(0, -4.0, 1.0), None, "inside the slop");
    }

    #[test]
    fn the_leaf_is_the_list_and_the_planner_lies_under_it() {
        for turn in [Turn::Forward, Turn::Back] {
            assert_eq!((turn.turning_pane(), turn.revealed_pane()), (0, 1));
        }
        assert_eq!((Turn::Forward.inward(), Turn::Back.inward()), (-1.0, 1.0));
    }

    #[test]
    fn forward_opens_the_leaf_and_back_closes_it() {
        assert_eq!(Turn::Forward.open_at(0.0), 0.0);
        assert_eq!(Turn::Forward.open_at(1.0), 1.0);
        assert_eq!(Turn::Back.open_at(0.0), 1.0);
        assert_eq!(Turn::Back.open_at(1.0), 0.0);
        assert_eq!(Turn::Forward.open_at(7.0), 1.0, "clamped");
    }

    #[test]
    fn each_outcome_ends_on_the_page_it_names() {
        assert_eq!(Turn::Forward.lands_on(Settle::Complete), 1);
        assert_eq!(Turn::Forward.lands_on(Settle::SpringBack), 0);
        assert_eq!(Turn::Back.lands_on(Settle::Complete), 0);
        assert_eq!(Turn::Back.lands_on(Settle::SpringBack), 1);
    }

    #[test]
    fn the_leaf_lies_flat_at_zero_and_stands_edge_on_at_one() {
        assert!(angle(0.0).abs() < 1e-9);
        assert!((angle(1.0) - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        let steps: Vec<f64> = (0..=20).map(|i| angle(f64::from(i) / 20.0)).collect();
        assert!(steps.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn a_finger_holding_the_free_edge_keeps_pace_with_it() {
        // Seen flat on (an eye infinitely far away) the edge is where the drag says.
        for share in [0.1, 0.35, 0.6, 0.9] {
            let forward = edge_px(W, angle(Turn::Forward.open_at(share)), FAR);
            assert!(
                (forward - W * (1.0 - share)).abs() < 1e-3,
                "forward {share}"
            );
            let back = edge_px(W, angle(Turn::Back.open_at(share)), FAR);
            assert!((back - W * share).abs() < 1e-3, "back {share}");
        }
    }

    #[test]
    fn at_the_phones_distance_the_edge_runs_only_a_little_ahead_of_the_finger() {
        let distance = crate::tuning::FLIP_PERSPECTIVE * W;
        for share in [0.1, 0.25, 0.5, 0.75] {
            let at = edge_px(W, angle(Turn::Forward.open_at(share)), distance);
            let ahead = at - W * (1.0 - share);
            assert!((0.0..0.15 * W).contains(&ahead), "{share}: {ahead}px ahead");
        }
    }

    #[test]
    fn nearer_the_viewer_the_free_edge_is_carried_outward() {
        let flat = edge_px(W, 1.0, FAR);
        let near = edge_px(W, 1.0, 3.0 * W);
        assert!(near > flat);
        assert!(
            (edge_px(W, 0.0, 3.0 * W) - W).abs() < 1e-9,
            "flat on the page it is the page"
        );
    }

    #[test]
    fn the_shadow_is_strongest_half_way_and_gone_flat_or_edge_on() {
        let quarter = std::f64::consts::FRAC_PI_2;
        assert!(lighting(0.0).1.abs() < 1e-9);
        assert!(lighting(quarter).1.abs() < 1e-9);
        assert!((lighting(quarter / 2.0).1 - 1.0).abs() < 1e-9);
        assert!(lighting(0.0).0.abs() < 1e-9, "flat leaf: no dimming");
        assert!(
            lighting(quarter).0 > lighting(quarter / 2.0).0,
            "darker as it turns away"
        );
    }

    #[test]
    fn letting_go_past_the_share_or_in_a_flick_turns_the_page() {
        assert_eq!(settle(W * 0.5, 0.0, W), Settle::Complete);
        assert_eq!(settle(W * 0.2, 0.0, W), Settle::SpringBack);
        assert_eq!(settle(40.0, 0.5, W), Settle::Complete, "a quick flick");
        assert_eq!(
            settle(10.0, 0.5, W),
            Settle::SpringBack,
            "too short to be a flick"
        );
        assert_eq!(settle(-50.0, 0.9, W), Settle::SpringBack, "the other way");
    }

    const SETTLE: Pull = Pull {
        omega: 14.0,
        zeta: 1.0,
    };
    const FALL: Pull = Pull {
        omega: 16.0,
        zeta: 0.55,
    };

    /// Steps a spring at 60 frames a second until it rests; returns the frames it took and
    /// every `open` on the way.
    fn run(mut spring: Spring, target: f64, pull: Pull) -> (usize, Vec<f64>) {
        let mut path = vec![spring.x];
        for frame in 1..=600 {
            spring.step(target, pull, 1.0 / 60.0);
            path.push(spring.x);
            if spring.at_rest(target) {
                return (frame, path);
            }
        }
        (600, path)
    }

    #[test]
    fn every_release_comes_to_rest_on_its_end() {
        for pull in [SETTLE, FALL] {
            for start in [0.0, 0.2, 0.5, 0.9, 1.0] {
                for speed in [-6.0, -1.0, 0.0, 1.0, 6.0] {
                    for target in [0.0, 1.0] {
                        let (frames, path) = run(Spring::new(start, speed), target, pull);
                        assert!(frames < 600, "{start} {speed} -> {target}: never rests");
                        assert!(
                            path.iter().all(|open| (0.0..=1.0).contains(open)),
                            "open left its range"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_critically_damped_leaf_never_overshoots_its_end() {
        let (_, up) = run(Spring::new(0.3, 0.0), 1.0, SETTLE);
        assert!(
            up.windows(2).all(|pair| pair[0] <= pair[1] + 1e-12),
            "monotone"
        );
        let (_, down) = run(Spring::new(0.6, 0.0), 0.0, SETTLE);
        assert!(
            down.windows(2).all(|pair| pair[0] >= pair[1] - 1e-12),
            "monotone"
        );
    }

    #[test]
    fn a_flick_carries_on_at_its_speed_and_arrives_sooner() {
        let (rested, _) = run(Spring::new(0.3, 0.0), 1.0, SETTLE);
        let (flicked, path) = run(Spring::new(0.3, 4.0), 1.0, SETTLE);
        assert!(flicked < rested, "{flicked} frames against {rested}");
        assert!(path[1] - path[0] > 0.3 * 4.0 / 60.0, "starts at its speed");
    }

    #[test]
    fn a_page_dropped_short_meets_the_table_and_rebounds_a_little() {
        let (_, path) = run(Spring::new(0.4, 0.0), 0.0, FALL);
        let lowest = (0..path.len())
            .min_by(|a, b| path[*a].total_cmp(&path[*b]))
            .expect("a path");
        assert!(path[lowest] < 0.02, "it comes all the way down");
        let rebound = path[lowest..].iter().copied().fold(0.0, f64::max);
        assert!(rebound > 0.002, "it lifts again: {rebound}");
        assert!(rebound < 0.1, "but only a little: {rebound}");
    }

    #[test]
    fn a_slow_frame_is_the_same_motion_as_many_fast_ones() {
        let mut slow = Spring::new(0.2, 0.0);
        let mut fast = Spring::new(0.2, 0.0);
        slow.step(1.0, SETTLE, 0.1);
        for _ in 0..6 {
            fast.step(1.0, SETTLE, 1.0 / 60.0);
        }
        assert!((slow.x - fast.x).abs() < 0.01);
    }

    #[test]
    fn a_turn_opens_or_closes_with_the_finger() {
        assert_eq!(
            (Turn::Forward.open_dir(), Turn::Back.open_dir()),
            (1.0, -1.0)
        );
    }
}

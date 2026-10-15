//! Characterization tests for the two springs the page moves things with: the nest's
//! (`nest.rs`, damped, in px and degrees) and the page turn's (`flip.rs`, in `open`).
//! They pin how the springs behave now, at the 60 frames a second the page runs, before
//! the physics work replaces them: how far they swing, how long they take to settle,
//! where they end, and that they never blow up. Tests only; no behaviour changes.

use super::Spring;
use crate::flip::{Pull, Spring as Leaf};
use crate::tuning::NEST_OMEGA;

/// One frame at 60 frames a second, in seconds.
const FRAME: f64 = 1.0 / 60.0;

/// Steps a nest spring until it is still, and returns the frame it settled on (if it
/// did within `frames`) and every position on the way.
fn nest_run(mut spring: Spring, frames: usize) -> (Option<usize>, Vec<f64>) {
    let mut path = vec![spring.x];
    for frame in 1..=frames {
        spring.step(FRAME);
        path.push(spring.x);
        if spring.still() {
            return (Some(frame), path);
        }
    }
    (None, path)
}

/// Steps a page-turn spring toward `target` until it is at rest, and returns the frame it
/// settled on (if it did within `frames`) and every state on the way.
fn leaf_run(
    mut spring: Leaf,
    target: f64,
    pull: Pull,
    frames: usize,
) -> (Option<usize>, Vec<Leaf>) {
    let mut states = vec![spring];
    for frame in 1..=frames {
        spring.step_with_push(target, pull, None, FRAME);
        states.push(spring);
        if spring.at_rest(target) {
            return (Some(frame), states);
        }
    }
    (None, states)
}

/// The turning points of a path: the sign changes of its speed, as `(index, value)`.
fn peaks(path: &[f64]) -> Vec<(usize, f64)> {
    path.windows(3)
        .enumerate()
        .filter(|(_, w)| (w[1] - w[0]) * (w[2] - w[1]) < 0.0)
        .map(|(i, w)| (i + 1, w[1]))
        .collect()
}

#[test]
fn a_nest_kick_swings_past_rest_and_settles_at_zero() {
    let (frames, path) = nest_run(Spring { x: 0.0, v: 100.0 }, 60 * 10);
    let frames = frames.expect("the nest settles within ten seconds");
    assert!(path.iter().any(|x| *x < 0.0), "it swings back past rest");
    assert!(
        frames > 60 && frames < 60 * 4,
        "settles in a few seconds: {frames}"
    );
    assert!(path.last().unwrap().abs() < 0.02, "it ends at rest");
}

#[test]
fn each_nest_swing_is_smaller_than_the_one_before() {
    let (_, path) = nest_run(Spring { x: 0.0, v: 100.0 }, 60 * 10);
    let swings: Vec<f64> = peaks(&path).iter().map(|(_, x)| x.abs()).collect();
    assert!(swings.len() >= 3, "it rings more than once: {swings:?}");
    for pair in swings.windows(2) {
        assert!(pair[1] < pair[0], "a damped swing shrinks: {swings:?}");
    }
}

#[test]
fn a_nest_kick_clamps_its_speed_to_the_most_it_may_swing() {
    let mut spring = Spring { x: 0.0, v: 0.0 };
    spring.kick(10_000.0, 2.0);
    assert!(
        (spring.v - 2.0 * NEST_OMEGA).abs() < 1e-9,
        "capped at most * omega: {}",
        spring.v
    );
}

#[test]
fn a_critical_page_turn_reaches_its_end_without_passing_it() {
    let pull = Pull {
        omega: 14.0,
        zeta: 1.0,
    };
    let (frames, states) = leaf_run(Leaf::new(0.0, 0.0), 1.0, pull, 60 * 5);
    let frames = frames.expect("the leaf rests within five seconds");
    let most = states.iter().map(|s| s.x).fold(f64::MIN, f64::max);
    assert!(most <= 1.0 + 1e-9, "no overshoot: {most}");
    assert!(
        frames > 10 && frames < 120,
        "settles in a second or two: {frames}"
    );
    assert!(
        (states.last().unwrap().x - 1.0).abs() < 0.002,
        "ends on its end"
    );
}

#[test]
fn an_underdamped_page_turn_bounces_off_the_spine_and_settles() {
    // The spine is a wall: the swing toward it is cut short there, so the leaf's speed turns
    // from up to down as it strikes, and it comes back down to rest on its end.
    let pull = Pull {
        omega: 16.0,
        zeta: 0.55,
    };
    let (frames, states) = leaf_run(Leaf::new(0.0, 0.0), 1.0, pull, 60 * 5);
    let frames = frames.expect("the leaf rests within five seconds");
    let strikes = states
        .windows(2)
        .any(|w| w[0].v > 0.0 && w[1].v < 0.0 && w[1].x > 0.95);
    assert!(states.iter().all(|s| s.x <= 1.0), "never past the spine");
    assert!(strikes, "it strikes the spine and turns back");
    assert!(frames < 120, "it settles within two seconds: {frames}");
    assert!(
        (states.last().unwrap().x - 1.0).abs() < 0.002,
        "ends on its end"
    );
}

#[test]
fn a_page_turn_stays_between_the_table_and_the_spine() {
    // Thrown hard toward the table (0), the leaf meets the wall and rebounds, never below.
    let pull = Pull {
        omega: 14.0,
        zeta: 1.0,
    };
    let mut leaf = Leaf::new(0.05, -6.0);
    for _ in 0..120 {
        leaf.step_with_push(1.0, pull, None, FRAME);
        assert!(
            (0.0..=1.0).contains(&leaf.x),
            "inside the walls: {}",
            leaf.x
        );
    }
}

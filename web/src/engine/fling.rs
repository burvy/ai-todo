//! Rambo thrown across the page (SPEC-ANSWERS item 31): let go of him mid-flick and
//! he keeps the flick's speed, slides across the paper, curves a little toward the
//! bottom of the screen and bounces off its edges until he comes to rest. Only a hit
//! on an edge hard enough to splat dazes him.
//!
//! Pure: a body, a step and the walls, so the motion is tested without a pointer or a
//! frame clock. Speeds are in px per ms, as `gesture::release_velocity` reads them,
//! and accelerations in px per ms². The numbers are in [`crate::tuning`].

use super::world::Vec2;
use crate::tuning::{
    FLING_BOUNCE, FLING_FRICTION, FLING_GRAVITY, FLING_REST_SPEED, FLING_SPLAT_REACH_PX,
    FLING_SPLAT_SPEED, FLING_SPLAT_STROKES,
};

/// Rambo in flight: where his middle is and how fast it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Body {
    pub(crate) pos: Vec2,
    pub(crate) vel: Vec2,
}

/// The box his middle may move in: the screen's edges pulled in by half his size, so
/// it is his side that touches the edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Walls {
    pub(crate) left: f64,
    pub(crate) top: f64,
    pub(crate) right: f64,
    pub(crate) bottom: f64,
}

/// Which edge he hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Left,
    Top,
    Right,
    Bottom,
}

impl Side {
    /// The way off this edge, back into the screen.
    fn inward(self) -> Vec2 {
        match self {
            Side::Left => (1.0, 0.0),
            Side::Top => (0.0, 1.0),
            Side::Right => (-1.0, 0.0),
            Side::Bottom => (0.0, -1.0),
        }
    }
}

/// A hit on an edge: where he touched it, and how fast he was going into it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Hit {
    pub(crate) side: Side,
    pub(crate) at: Vec2,
    pub(crate) speed: f64,
}

impl Hit {
    /// Hard enough to splat: the ink jumps out of him against the edge, and he is dazed.
    pub(crate) fn splats(&self) -> bool {
        self.speed >= FLING_SPLAT_SPEED
    }
}

fn length((x, y): Vec2) -> f64 {
    x.hypot(y)
}

/// Whether he has stopped sliding.
pub(crate) fn at_rest(body: &Body) -> bool {
    length(body.vel) < FLING_REST_SPEED
}

/// `dt` ms of flight. The paper's friction slows him at a steady rate against his
/// motion, a weaker pull draws him toward the bottom of the screen (too weak to move
/// him once he lies still), and an edge turns him back with [`FLING_BOUNCE`] of the
/// speed he hit it with. Returns where he is next and the hardest edge he hit.
pub(crate) fn step(body: Body, walls: &Walls, dt: f64) -> (Body, Option<Hit>) {
    let speed = length(body.vel);
    // Friction never pushes him backwards: at most it stops him this step.
    let vel = if speed == 0.0 || speed <= FLING_FRICTION * dt {
        (0.0, 0.0)
    } else {
        let slowed = (speed - FLING_FRICTION * dt) / speed;
        (
            body.vel.0 * slowed,
            body.vel.1 * slowed + FLING_GRAVITY * dt,
        )
    };
    let mut pos = (body.pos.0 + vel.0 * dt, body.pos.1 + vel.1 * dt);
    let mut vel = vel;
    let mut hit: Option<Hit> = None;
    let mut bounce = |coord: &mut f64, v: &mut f64, edge: f64, side: Side| {
        // His speed into the edge, against its inward normal.
        let (nx, ny) = side.inward();
        let into = -*v * (nx + ny);
        *coord = edge + (edge - *coord) * FLING_BOUNCE;
        *v = -*v * FLING_BOUNCE;
        if hit.is_none_or(|before| into > before.speed) {
            hit = Some(Hit {
                side,
                at: (0.0, 0.0),
                speed: into,
            });
        }
    };
    if pos.0 < walls.left {
        bounce(&mut pos.0, &mut vel.0, walls.left, Side::Left);
    } else if pos.0 > walls.right {
        bounce(&mut pos.0, &mut vel.0, walls.right, Side::Right);
    }
    if pos.1 < walls.top {
        bounce(&mut pos.1, &mut vel.1, walls.top, Side::Top);
    } else if pos.1 > walls.bottom {
        bounce(&mut pos.1, &mut vel.1, walls.bottom, Side::Bottom);
    }
    let hit = hit.map(|hit| Hit {
        at: touch_point(pos, hit.side, walls),
        ..hit
    });
    (Body { pos, vel }, hit)
}

/// Where on `side` he touched it, with his middle near `pos`.
fn touch_point(pos: Vec2, side: Side, walls: &Walls) -> Vec2 {
    match side {
        Side::Left => (walls.left, pos.1),
        Side::Right => (walls.right, pos.1),
        Side::Top => (pos.0, walls.top),
        Side::Bottom => (pos.0, walls.bottom),
    }
}

/// The ink a splat throws off the edge: short strokes fanned out from where he hit,
/// back into the screen, longer the harder the hit, each a little different. `seed`
/// keeps one splat the same every time it is laid.
pub(crate) fn splat_strokes(hit: &Hit, seed: &str) -> Vec<Vec<Vec2>> {
    let (ix, iy) = hit.side.inward();
    let base = iy.atan2(ix);
    let force = (hit.speed / FLING_SPLAT_SPEED).clamp(1.0, 2.0);
    (0..FLING_SPLAT_STROKES)
        .map(|n| {
            let jitter = crate::row::hash_unit(seed, n);
            let fan = (f64::from(n) + 0.5) / f64::from(FLING_SPLAT_STROKES) - 0.5;
            // Spread across a half circle, never back into the edge.
            let angle = base + (fan + jitter * 0.08) * std::f64::consts::PI * 0.9;
            let reach = FLING_SPLAT_REACH_PX * force * (0.55 + 0.45 * jitter.abs());
            let (dx, dy) = (angle.cos(), angle.sin());
            vec![
                hit.at,
                (hit.at.0 + dx * reach * 0.5, hit.at.1 + dy * reach * 0.5),
                (hit.at.0 + dx * reach, hit.at.1 + dy * reach),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A phone screen: his middle may go from 20 to 370 across and 20 to 800 down.
    const WALLS: Walls = Walls {
        left: 20.0,
        top: 20.0,
        right: 370.0,
        bottom: 800.0,
    };

    /// The engine's step, in ms.
    const DT: f64 = 1000.0 / 120.0;

    fn body(pos: Vec2, vel: Vec2) -> Body {
        Body { pos, vel }
    }

    /// Steps until he rests, with every hit on the way.
    fn fly(mut body: Body) -> (Body, Vec<Hit>) {
        let mut hits = Vec::new();
        for _ in 0..10_000 {
            if at_rest(&body) {
                return (body, hits);
            }
            let (next, hit) = step(body, &WALLS, DT);
            hits.extend(hit);
            body = next;
        }
        panic!("he never came to rest: {body:?}");
    }

    #[test]
    fn friction_slows_him_to_a_stop_without_pushing_him_back() {
        let (rest, hits) = fly(body((100.0, 400.0), (1.0, 0.0)));
        assert!(hits.is_empty(), "{hits:?}");
        assert!(rest.pos.0 > 150.0, "he slid on: {rest:?}");
        let mut b = body((100.0, 400.0), (1.0, 0.0));
        while !at_rest(&b) {
            let next = step(b, &WALLS, DT).0;
            assert!(next.vel.0 >= 0.0, "friction turned him round");
            assert!(length(next.vel) <= length(b.vel) + FLING_GRAVITY * DT);
            b = next;
        }
    }

    #[test]
    fn a_body_at_rest_stays_at_rest() {
        // The pull to the bottom is weaker than the paper's grip.
        let still = body((200.0, 400.0), (0.0, 0.0));
        let (next, hit) = step(still, &WALLS, DT);
        assert_eq!(hit, None);
        assert_eq!(next, still);
    }

    #[test]
    fn a_throw_curves_toward_the_bottom_of_the_screen() {
        let mut b = body((50.0, 400.0), (1.5, 0.0));
        for _ in 0..10 {
            b = step(b, &WALLS, DT).0;
        }
        assert!(b.pos.1 > 400.0, "{b:?}");
    }

    #[test]
    fn smacked_into_an_edge_he_bounces_back_slower_and_splats() {
        let (next, hit) = step(body((360.0, 400.0), (3.0, 0.0)), &WALLS, DT);
        let hit = hit.expect("he hit the right edge");
        assert_eq!(hit.side, Side::Right);
        assert_eq!(hit.at, (WALLS.right, next.pos.1));
        assert!(hit.splats(), "{hit:?}");
        assert!(next.pos.0 <= WALLS.right, "never through the edge");
        assert!(next.vel.0 < 0.0 && next.vel.0.abs() < 3.0, "{next:?}");
    }

    #[test]
    fn a_soft_bump_bounces_without_a_splat() {
        let (_, hit) = step(
            body((25.0, 400.0), (-FLING_SPLAT_SPEED * 0.6, 0.0)),
            &WALLS,
            DT,
        );
        let hit = hit.expect("he touched the left edge");
        assert_eq!(hit.side, Side::Left);
        assert!(!hit.splats(), "{hit:?}");
    }

    #[test]
    fn he_always_comes_to_rest_on_the_screen() {
        for vel in [(4.0, 0.0), (-4.0, -4.0), (0.5, 3.9), (-3.0, 2.0)] {
            let (rest, _) = fly(body((200.0, 400.0), vel));
            assert!(
                (WALLS.left..=WALLS.right).contains(&rest.pos.0)
                    && (WALLS.top..=WALLS.bottom).contains(&rest.pos.1),
                "{vel:?} -> {rest:?}"
            );
        }
    }

    #[test]
    fn a_splat_fans_back_into_the_screen_and_reads_the_same_each_time() {
        let hit = Hit {
            side: Side::Left,
            at: (WALLS.left, 300.0),
            speed: FLING_SPLAT_SPEED * 1.5,
        };
        let strokes = splat_strokes(&hit, "splat");
        assert_eq!(strokes.len(), usize::from(FLING_SPLAT_STROKES));
        assert!(strokes.iter().all(|s| s[0] == hit.at));
        assert!(
            strokes.iter().flatten().all(|&(x, _)| x >= WALLS.left),
            "no ink behind the edge"
        );
        assert_eq!(strokes, splat_strokes(&hit, "splat"));
        let harder = Hit {
            speed: FLING_SPLAT_SPEED * 2.0,
            ..hit
        };
        let reach = |s: &[Vec<Vec2>]| {
            s.iter()
                .map(|p| length((p[2].0 - p[0].0, p[2].1 - p[0].1)))
                .sum::<f64>()
        };
        assert!(reach(&splat_strokes(&harder, "splat")) > reach(&strokes));
    }
}

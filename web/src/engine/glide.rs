//! A few values carried from where they are to where they should be, each on a
//! [`Spring1`]: a strip flying to its pin, the lamp's sheet coming down. The engine steps
//! them on its clock and hands the places to the page to draw; nothing here is a CSS
//! transition or a timer.
// Natively nothing pumps the loop, so only the tests of the pure half run.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use super::body::{Pull, Spring1};
use super::sched::ENGINE_DT_S;
use super::sched::{Flow, Handle};

/// `N` values on springs, all pulled by the same `pull` to their ends.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Glide<const N: usize> {
    springs: [Spring1; N],
    to: [f64; N],
    pull: Pull,
    near: f64,
}

impl<const N: usize> Glide<N> {
    /// From rest at `from` to `to`; it is done once every value is within `near` of its
    /// end and has slowed to a crawl.
    pub(crate) fn new(from: [f64; N], to: [f64; N], pull: Pull, near: f64) -> Self {
        Glide {
            springs: from.map(|x| Spring1::new(x, 0.0)),
            to,
            pull,
            near,
        }
    }

    /// `steps` fixed engine steps onward.
    pub(crate) fn step(&mut self, steps: u32) {
        for _ in 0..steps {
            for (spring, &to) in self.springs.iter_mut().zip(&self.to) {
                spring.step(to, self.pull, 0.0, ENGINE_DT_S);
            }
        }
    }

    pub(crate) fn values(&self) -> [f64; N] {
        self.springs.map(|spring| spring.x)
    }

    /// Whether every value has come to rest at its end.
    pub(crate) fn settled(&self) -> bool {
        let slow = self.near * self.pull.omega;
        self.springs
            .iter()
            .zip(&self.to)
            .all(|(spring, &to)| spring.settled(to, self.near, slow))
    }
}

/// Carries `from` to `to`, calling `draw` with the places on every frame and `done` once
/// they rest (after the last `draw`, which has the exact ends). The start is drawn at
/// once, so the first frame never shows the thing at its end before it leaves. Stopping
/// the handle ends it without `done`.
pub(crate) fn glide<const N: usize>(
    from: [f64; N],
    to: [f64; N],
    pull: Pull,
    near: f64,
    mut draw: impl FnMut([f64; N]) + 'static,
    done: impl FnOnce() + 'static,
) -> Handle {
    draw(from);
    let mut glide = Glide::new(from, to, pull, near);
    let mut done = Some(done);
    super::on_tick(move |tick| {
        glide.step(tick.steps);
        if !glide.settled() {
            draw(glide.values());
            return Flow::Go;
        }
        draw(to);
        if let Some(done) = done.take() {
            done();
        }
        Flow::Stop
    })
}

/// [`glide`] for a body the page names by `key` (a task's id): one glide runs per key, so
/// a body sent somewhere new while it is still on its way is taken over by the new glide
/// (it starts from `from`, and the old one never calls its `done`).
pub(crate) fn glide_keyed<const N: usize>(
    key: &str,
    from: [f64; N],
    to: [f64; N],
    pull: Pull,
    near: f64,
    draw: impl FnMut([f64; N]) + 'static,
    done: impl FnOnce() + 'static,
) {
    let start = super::run::next_start();
    let owned = key.to_owned();
    let handle = glide(from, to, pull, near, draw, move || {
        super::run::release(&owned, start);
        done();
    });
    super::run::hold(key, start, handle);
}

#[cfg(test)]
mod tests {
    use super::*;

    const PULL: Pull = Pull {
        omega: 20.0,
        zeta: 1.0,
    };

    fn seconds_to_settle<const N: usize>(mut glide: Glide<N>) -> f64 {
        let mut steps = 0;
        while !glide.settled() {
            glide.step(1);
            steps += 1;
            assert!(steps < 120 * 20, "never settles");
        }
        f64::from(steps) * ENGINE_DT_S
    }

    #[test]
    fn it_comes_to_every_end_and_rests() {
        let mut glide = Glide::new([-100.0, 90.0, 12.0], [0.0, 0.0, 12.0], PULL, 0.2);
        glide.step(120 * 3);
        assert!(glide.settled());
        for (x, end) in glide.values().into_iter().zip([0.0, 0.0, 12.0]) {
            assert!((x - end).abs() < 0.2, "{x} is not near {end}");
        }
    }

    #[test]
    fn a_critically_damped_glide_never_passes_its_end() {
        let mut glide = Glide::new([-100.0], [0.0], PULL, 0.2);
        for _ in 0..600 {
            glide.step(1);
            assert!(glide.values()[0] <= 1e-9, "{:?}", glide.values());
        }
    }

    #[test]
    fn nothing_that_starts_at_its_end_has_a_distance_to_go() {
        assert!(Glide::new([5.0, 5.0], [5.0, 5.0], PULL, 0.2).settled());
    }

    #[test]
    fn a_quicker_pull_settles_sooner() {
        let slow = Pull {
            omega: 10.0,
            ..PULL
        };
        let fast = Pull {
            omega: 30.0,
            ..PULL
        };
        assert!(
            seconds_to_settle(Glide::new([-100.0], [0.0], fast, 0.2))
                < seconds_to_settle(Glide::new([-100.0], [0.0], slow, 0.2))
        );
    }

    #[test]
    fn the_time_to_land_does_not_depend_on_how_the_steps_are_grouped() {
        let (mut once, mut by_ones) = (
            Glide::new([-100.0], [0.0], PULL, 0.2),
            Glide::new([-100.0], [0.0], PULL, 0.2),
        );
        once.step(60);
        (0..60).for_each(|_| by_ones.step(1));
        assert_eq!(once.values(), by_ones.values());
    }
}

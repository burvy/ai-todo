//! One body (ARCHITECTURE 8.0.1): what moves physically moves by one rule, a speed of
//! its own changed by forces, stepped semi-implicitly at the engine's fixed step so it
//! stays stable. This is the scalar half: one damped spring, [`Spring1`], for anything
//! that swings about a value (the nest's sag and rock, his dance sway, a page's leaf).
//!
//! Pure, so its rules are tested here.

/// How a spring pulls: `omega` is its quickness (radians per second), `zeta` its damping
/// (1 settles without overshoot; below that it swings past its end and back).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pull {
    pub omega: f64,
    pub zeta: f64,
}

/// A value on a damped spring: `x` where it is, `v` its speed per second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spring1 {
    pub x: f64,
    pub v: f64,
}

impl Spring1 {
    pub fn new(x: f64, v: f64) -> Self {
        Spring1 { x, v }
    }

    /// `dt` seconds pulled toward `target`, with `accel` more pushing it (per second
    /// squared): the speed first, then the place, so the step stays stable.
    pub fn step(&mut self, target: f64, pull: Pull, accel: f64, dt: f64) {
        let spring =
            -pull.omega * pull.omega * (self.x - target) - 2.0 * pull.zeta * pull.omega * self.v;
        self.v += (spring + accel) * dt;
        self.x += self.v * dt;
    }

    /// A kick of `dv` to its speed, which never ends past `cap` either way.
    pub fn kick(&mut self, dv: f64, cap: f64) {
        self.v = (self.v + dv).clamp(-cap, cap);
    }

    /// Whether it is within `near` of `target` and slower than `slow`: at rest.
    pub fn settled(&self, target: f64, near: f64, slow: f64) -> bool {
        (self.x - target).abs() < near && self.v.abs() < slow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 120.0;

    fn run(mut spring: Spring1, pull: Pull, seconds: f64) -> Vec<f64> {
        (0..(seconds / DT) as usize)
            .map(|_| {
                spring.step(1.0, pull, 0.0, DT);
                spring.x
            })
            .collect()
    }

    #[test]
    fn critically_damped_it_comes_to_its_target_and_never_passes_it() {
        let path = run(
            Spring1::default(),
            Pull {
                omega: 12.0,
                zeta: 1.0,
            },
            2.0,
        );
        assert!(path.iter().all(|&x| x <= 1.0 + 1e-9), "never overshoots");
        assert!(
            path.windows(2).all(|w| w[1] >= w[0] - 1e-12),
            "never turns back"
        );
        assert!((path.last().unwrap() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn loosely_damped_it_swings_past_and_settles_ever_closer() {
        let path = run(
            Spring1::default(),
            Pull {
                omega: 12.0,
                zeta: 0.2,
            },
            4.0,
        );
        let peak = path.iter().cloned().fold(0.0, f64::max);
        assert!(peak > 1.2, "it swings past: {peak}");
        assert!((path.last().unwrap() - 1.0).abs() < 1e-2, "and settles");
    }

    #[test]
    fn a_kick_moves_it_and_its_speed_never_passes_the_cap() {
        let mut spring = Spring1::default();
        spring.kick(5.0, 2.0);
        assert_eq!(spring.v, 2.0);
        spring.kick(-10.0, 2.0);
        assert_eq!(spring.v, -2.0);
        assert!(!spring.settled(0.0, 0.01, 0.01));
        let pull = Pull {
            omega: 10.0,
            zeta: 1.0,
        };
        for _ in 0..240 {
            spring.step(0.0, pull, 0.0, DT);
        }
        assert!(spring.settled(0.0, 0.01, 0.01), "{spring:?}");
    }

    #[test]
    fn a_steady_push_holds_it_off_its_target() {
        // The spring's pull meets the push where omega squared times the offset equals it.
        let pull = Pull {
            omega: 10.0,
            zeta: 1.0,
        };
        let mut spring = Spring1::default();
        for _ in 0..600 {
            spring.step(0.0, pull, 50.0, DT);
        }
        assert!((spring.x - 0.5).abs() < 1e-3, "{}", spring.x);
    }
}

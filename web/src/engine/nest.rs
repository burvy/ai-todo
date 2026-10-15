//! Rambo's nest as an entity of the engine (SPEC-ANSWERS items 16, 17; ENGINE-PLAN
//! step 7): a soft dish that sags when he lands in it, springs up and rocks when he
//! leaves, and settles. Two damped springs, `sag` (px, down) and `rock` (degrees,
//! clockwise), kicked by impulses and stepped with the world.
//!
//! Pure, so its rules are tested here. The page draws both of its layers (the back
//! behind him, the rim over his tip) moved by [`Nest::offset`], and sits him on the
//! sag.

use super::body::{Pull, Spring1};
use super::world::Vec2;
use crate::tuning::{
    DRY_NIB_TAP_KICK, DRY_NIB_TAP_ROCK, NEST_DAMPING, NEST_IMPACT_SAG, NEST_LAND_KICK,
    NEST_LEAVE_KICK, NEST_OMEGA, NEST_ROCK_MAX_DEG, NEST_ROCK_PER_PX, NEST_SAG_MAX_PX,
};

/// How both of the dish's springs pull it back level.
const PULL: Pull = Pull {
    omega: NEST_OMEGA,
    zeta: NEST_DAMPING,
};

/// Below this (px or degrees) the nest no longer moves visibly.
const STILL: f64 = 0.02;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Nest {
    sag: Spring1,
    rock: Spring1,
}

/// A kick of `dv` to `spring`, kept so the swing it starts never passes `most` (a swing
/// from rest reaches about `v / omega`).
fn kick(spring: &mut Spring1, dv: f64, most: f64) {
    spring.kick(dv, most * NEST_OMEGA);
}

fn still(spring: &Spring1) -> bool {
    spring.settled(0.0, STILL, STILL * NEST_OMEGA)
}

impl Nest {
    /// How it is drawn now: pushed down by `.0` px and turned `.1` degrees about its base.
    pub(crate) fn offset(&self) -> Vec2 {
        (self.sag.x, self.rock.x)
    }

    /// Whether it has settled and nothing moves it until he does.
    pub(crate) fn at_rest(&self) -> bool {
        still(&self.sag) && still(&self.rock)
    }

    /// `dt_s` seconds of springing back.
    pub(crate) fn step(&mut self, dt_s: f64) {
        self.sag.step(0.0, PULL, 0.0, dt_s);
        self.rock.step(0.0, PULL, 0.0, dt_s);
        self.sag.x = self.sag.x.clamp(-NEST_SAG_MAX_PX, NEST_SAG_MAX_PX);
        self.rock.x = self.rock.x.clamp(-NEST_ROCK_MAX_DEG, NEST_ROCK_MAX_DEG);
    }

    /// He comes down in it `off` px right of its middle, still falling at `impact` px/s:
    /// it sags under him, deeper the harder he lands, and tips toward the side he
    /// landed on.
    pub(crate) fn land(&mut self, off: f64, impact: f64) {
        kick(&mut self.sag, give(impact), NEST_SAG_MAX_PX);
        kick(
            &mut self.rock,
            off * NEST_ROCK_PER_PX * NEST_OMEGA,
            NEST_ROCK_MAX_DEG,
        );
    }

    /// A throw comes down in it at `speed` px per ms, heading `dx` across: a bigger sag
    /// the faster he comes, tipped the way he was going.
    pub(crate) fn catch(&mut self, speed: f64, dx: f64) {
        kick(&mut self.sag, give(speed * 1000.0), NEST_SAG_MAX_PX);
        kick(
            &mut self.rock,
            dx.signum() * NEST_LEAVE_KICK,
            NEST_ROCK_MAX_DEG,
        );
    }

    /// He taps his nib on its rim: a small knock down, tipping it toward the page.
    pub(crate) fn tap(&mut self) {
        kick(&mut self.sag, DRY_NIB_TAP_KICK, NEST_SAG_MAX_PX);
        kick(&mut self.rock, -DRY_NIB_TAP_ROCK, NEST_ROCK_MAX_DEG);
    }

    /// He bounces in it to a beat of `strength` (0 to 1).
    pub(crate) fn bounce(&mut self, strength: f64) {
        kick(
            &mut self.sag,
            strength * crate::tuning::DANCE_BOUNCE_KICK,
            NEST_SAG_MAX_PX,
        );
    }

    /// He leaves it heading `dx` across: it springs up as his weight goes and rocks
    /// the other way.
    pub(crate) fn leave(&mut self, dx: f64) {
        kick(&mut self.sag, -NEST_LAND_KICK * 0.5, NEST_SAG_MAX_PX);
        kick(
            &mut self.rock,
            -dx.signum() * NEST_LEAVE_KICK,
            NEST_ROCK_MAX_DEG,
        );
    }
}

/// How fast the dish starts to sag, in px/s, under him coming down at `impact` px/s.
fn give(impact: f64) -> f64 {
    (impact.abs() * NEST_IMPACT_SAG).max(NEST_LAND_KICK)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 120.0;

    /// A small seeded generator, so the property runs are the same every time.
    struct Seeded(u64);

    impl Seeded {
        fn unit(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        }
    }

    /// Steps until it rests, or panics after `max_s` seconds.
    fn settle(nest: &mut Nest, max_s: f64) -> f64 {
        let mut t = 0.0;
        while !nest.at_rest() {
            nest.step(DT);
            t += DT;
            assert!(t < max_s, "never settled: {nest:?}");
        }
        t
    }

    #[test]
    fn at_rest_it_stays_at_rest() {
        let mut nest = Nest::default();
        for _ in 0..1000 {
            nest.step(DT);
        }
        assert_eq!(nest, Nest::default());
        assert!(nest.at_rest());
    }

    #[test]
    fn a_landing_sags_it_then_it_settles_back_level() {
        let mut nest = Nest::default();
        nest.land(0.0, 0.0);
        let mut deepest: f64 = 0.0;
        for _ in 0..30 {
            nest.step(DT);
            deepest = deepest.max(nest.offset().0);
        }
        assert!(deepest > 1.0, "it sagged under him: {deepest}");
        assert_eq!(
            nest.offset().1,
            0.0,
            "a landing in the middle does not tip it"
        );
        settle(&mut nest, 3.0);
        assert!(nest.offset().0.abs() < STILL);
    }

    #[test]
    fn it_tips_toward_where_he_lands_and_away_from_where_he_leaves() {
        let tipped = |kick: &dyn Fn(&mut Nest)| {
            let mut nest = Nest::default();
            kick(&mut nest);
            for _ in 0..12 {
                nest.step(DT);
            }
            nest.offset().1
        };
        assert!(tipped(&|nest| nest.land(10.0, 300.0)) > 0.0);
        assert!(tipped(&|nest| nest.land(-10.0, 300.0)) < 0.0);
        assert!(
            tipped(&|nest| nest.leave(-50.0)) > 0.0,
            "leaving left, it rocks right"
        );
        assert!(tipped(&|nest| nest.leave(50.0)) < 0.0);
    }

    #[test]
    fn a_harder_landing_sags_it_deeper_and_a_settle_still_gives() {
        let deepest = |impact: f64| {
            let mut nest = Nest::default();
            nest.land(0.0, impact);
            (0..120)
                .map(|_| {
                    nest.step(DT);
                    nest.offset().0
                })
                .fold(0.0, f64::max)
        };
        let mut seeded = Seeded(9);
        for _ in 0..100 {
            let soft = seeded.unit().abs() * 1500.0;
            let hard = soft + 1.0 + seeded.unit().abs() * 1500.0;
            assert!(deepest(hard) >= deepest(soft), "{hard} vs {soft}");
        }
        assert!(
            deepest(0.0) > 2.0,
            "even settling in, it gives: {}",
            deepest(0.0)
        );
    }

    #[test]
    fn a_faster_catch_sags_it_deeper() {
        let deepest = |speed: f64| {
            let mut nest = Nest::default();
            nest.catch(speed, 1.0);
            (0..60)
                .map(|_| {
                    nest.step(DT);
                    nest.offset().0
                })
                .fold(0.0, f64::max)
        };
        assert!(
            deepest(2.0) > deepest(0.5),
            "{} vs {}",
            deepest(2.0),
            deepest(0.5)
        );
    }

    #[test]
    fn whatever_kicks_it_it_never_swings_past_its_most_and_always_settles() {
        let mut seeded = Seeded(31);
        for _ in 0..200 {
            let mut nest = Nest::default();
            for _ in 0..8 {
                match (seeded.unit() * 3.0).abs() as u8 {
                    0 => nest.land(seeded.unit() * 40.0, seeded.unit().abs() * 3000.0),
                    1 => nest.leave(seeded.unit()),
                    _ => nest.catch(seeded.unit().abs() * 4.0, seeded.unit()),
                }
                for _ in 0..((seeded.unit().abs() * 30.0) as usize) {
                    nest.step(DT);
                    let (sag, rock) = nest.offset();
                    assert!(sag.abs() <= NEST_SAG_MAX_PX && rock.abs() <= NEST_ROCK_MAX_DEG);
                }
            }
            settle(&mut nest, 4.0);
        }
    }

    #[test]
    fn it_never_jumps_between_steps() {
        let mut nest = Nest::default();
        nest.catch(4.0, 1.0);
        let mut was = nest.offset();
        for _ in 0..240 {
            nest.step(DT);
            let now = nest.offset();
            assert!(
                (now.0 - was.0).abs() < 1.5 && (now.1 - was.1).abs() < 1.0,
                "{was:?} -> {now:?}"
            );
            was = now;
        }
    }
}

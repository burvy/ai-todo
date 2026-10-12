//! A tiny deterministic random source, so a simulated day is reproducible without
//! pulling a dependency into a crate that only ever picks a line.

/// The random source the brain draws from.
pub trait Rng {
    /// A number in `0.0..1.0`, never exactly `1.0`.
    fn next_f32(&mut self) -> f32;
}

/// SplitMix64: small, fast, and more than good enough for choosing between a few lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seeded(pub u64);

impl Seeded {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 24 bits as a fraction. 24 bits is plenty and stays exact in `f32`.
    pub fn fraction(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl Rng for Seeded {
    fn next_f32(&mut self) -> f32 {
        self.fraction()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_stay_below_one() {
        let mut rng = Seeded::new(7);
        for _ in 0..1000 {
            let value = rng.fraction();
            assert!((0.0..1.0).contains(&value), "{value}");
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_day() {
        let mut a = Seeded::new(42);
        let mut b = Seeded::new(42);
        for _ in 0..100 {
            assert_eq!(a.fraction(), b.fraction());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Seeded::new(1);
        let mut b = Seeded::new(2);
        let different = (0..50).any(|_| a.fraction() != b.fraction());
        assert!(different);
    }
}

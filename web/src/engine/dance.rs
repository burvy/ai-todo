//! Rambo dances to the music of the "Everything + music" Sound preset (SPEC-ANSWERS
//! Interview 2, preset C). Nothing here is a loop of frames: he hears how loud the
//! music is every step, picks out each note's attack, and every attack he hears kicks
//! his sway to the other side and bounces the nest under him. When the music stops,
//! nothing kicks him and both settle.
//!
//! Pure, so its rules are tested here; the world steps it and `music.rs` measures the
//! loudness it hears.

use crate::tuning::{
    DANCE_DAMPING, DANCE_FAST_MS, DANCE_FLOOR, DANCE_LEAN_KICK, DANCE_LEAN_MAX_DEG,
    DANCE_MIN_GAP_MS, DANCE_OMEGA, DANCE_ONSET_RATIO, DANCE_SLOW_MS,
};

/// What he hears and how he sways to it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Dance {
    /// How loud the music is now, followed quickly.
    fast: f64,
    /// How loud it has been lately, followed slowly: a note's attack lifts `fast` above it.
    slow: f64,
    /// Ms since the last beat he heard.
    since_ms: f64,
    /// Whether the music has been below a beat's loudness since the last one: a beat is
    /// a rise, so a held sound moves him once, not again and again.
    armed: bool,
    /// The side the next beat sends him to: 1 right, -1 left.
    side: f64,
    /// His sway in degrees, and its speed in degrees/s: a damped spring.
    lean: f64,
    lean_v: f64,
}

/// Below this (degrees, degrees/s over omega) his sway no longer shows.
const STILL: f64 = 0.02;

impl Dance {
    /// One step of `dt_ms` hearing the music at `level` (its loudness, 0 to 1), or `None`
    /// when none plays or he is not where he dances. Returns how strong a beat he heard
    /// now (0 to 1), if he heard one.
    pub(crate) fn hear(&mut self, level: Option<f64>, dt_ms: f64) -> Option<f64> {
        let level = level.unwrap_or(0.0).max(0.0);
        self.fast += (level - self.fast) * (dt_ms / DANCE_FAST_MS).min(1.0);
        self.slow += (level - self.slow) * (dt_ms / DANCE_SLOW_MS).min(1.0);
        self.since_ms += dt_ms;
        let above = self.fast > DANCE_FLOOR && self.fast > self.slow * DANCE_ONSET_RATIO;
        let beat = (above && self.armed && self.since_ms >= DANCE_MIN_GAP_MS)
            .then(|| ((self.fast / self.slow.max(f64::EPSILON)) - 1.0).clamp(0.0, 1.0));
        self.armed = !above || (self.armed && beat.is_none());
        if let Some(strength) = beat {
            self.since_ms = 0.0;
            // A beat pushes the swing he is in, as a hand on a swing does; the spring
            // brings him back. Only from still does a beat start him, to the other side.
            self.side = if self.lean_v.abs() >= STILL * DANCE_OMEGA {
                self.lean_v.signum()
            } else if self.side > 0.0 {
                -1.0
            } else {
                1.0
            };
            let cap = DANCE_LEAN_MAX_DEG * DANCE_OMEGA;
            self.lean_v = (self.lean_v + self.side * strength * DANCE_LEAN_KICK).clamp(-cap, cap);
        }
        // With no music to hear he comes to rest at once, as a dancer stops: critically
        // damped, not swinging on for seconds.
        let damping = if level > 0.0 { DANCE_DAMPING } else { 1.0 };
        self.step_lean(dt_ms / 1000.0, damping);
        beat
    }

    fn step_lean(&mut self, dt_s: f64, damping: f64) {
        let accel =
            -DANCE_OMEGA * DANCE_OMEGA * self.lean - 2.0 * damping * DANCE_OMEGA * self.lean_v;
        self.lean_v += accel * dt_s;
        self.lean = (self.lean + self.lean_v * dt_s).clamp(-DANCE_LEAN_MAX_DEG, DANCE_LEAN_MAX_DEG);
    }

    /// How far he sways now, in degrees.
    pub(crate) fn lean_deg(&self) -> f64 {
        self.lean
    }

    /// Whether he has stopped swaying and hears nothing: the loop may sleep.
    pub(crate) fn still(&self) -> bool {
        self.lean.abs() < STILL
            && self.lean_v.abs() < STILL * DANCE_OMEGA
            && self.fast < DANCE_FLOOR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1000.0 / 120.0;

    /// Piano notes struck every `every_ms`, each loud at once and dying away, for `ms`.
    fn notes(every_ms: f64, ms: f64, loud: f64) -> impl Iterator<Item = Option<f64>> {
        (0..(ms / DT) as usize).map(move |step| {
            let into = (step as f64 * DT) % every_ms;
            Some(loud * (-into / 250.0).exp())
        })
    }

    fn beats(dance: &mut Dance, levels: impl Iterator<Item = Option<f64>>) -> Vec<f64> {
        levels.filter_map(|level| dance.hear(level, DT)).collect()
    }

    #[test]
    fn silence_and_a_steady_sound_are_no_beat() {
        let mut dance = Dance::default();
        assert!(beats(&mut dance, std::iter::repeat_n(None, 600)).is_empty());
        assert!(beats(&mut dance, std::iter::repeat_n(Some(0.0), 600)).is_empty());
        // A held sound swells once, then is just loud: at most the one swell.
        assert!(beats(&mut dance, std::iter::repeat_n(Some(0.3), 1200)).len() <= 1);
        assert!(beats(&mut dance, std::iter::repeat_n(Some(0.3), 1200)).is_empty());
    }

    #[test]
    fn he_hears_one_beat_for_each_note_struck() {
        // Slow and quicker playing, loud and soft: each attack is a beat, and none come
        // closer than his shortest gap.
        for (every_ms, loud) in [(900.0, 0.2), (600.0, 0.05), (450.0, 0.4)] {
            let mut dance = Dance::default();
            let heard = beats(&mut dance, notes(every_ms, 9000.0, loud));
            let struck = (9000.0 / every_ms) as usize;
            assert!(
                heard.len().abs_diff(struck) <= 1,
                "{every_ms} ms apart: heard {} of {struck}",
                heard.len()
            );
            assert!(heard.iter().all(|strength| (0.0..=1.0).contains(strength)));
        }
        let mut dance = Dance::default();
        let mut last = None::<usize>;
        for (step, level) in notes(120.0, 5000.0, 0.3).enumerate() {
            if dance.hear(level, DT).is_some() {
                if let Some(last) = last {
                    assert!((step - last) as f64 * DT + 1e-6 >= DANCE_MIN_GAP_MS);
                }
                last = Some(step);
            }
        }
    }

    #[test]
    fn a_beat_pushes_the_swing_he_is_in_never_against_it() {
        // A quick run of notes, then slow ones: every beat he hears goes with his swing.
        let mut dance = Dance::default();
        let mut pushed = 0;
        for level in notes(120.0, 6000.0, 0.3).chain(notes(700.0, 6000.0, 0.3)) {
            let before = dance.lean_v;
            if dance.hear(level, DT).is_some() && before.abs() > 1.0 {
                assert_eq!(dance.lean_v.signum(), before.signum(), "{dance:?}");
                assert!(dance.lean_v.abs() >= before.abs() - 1.0, "{dance:?}");
                pushed += 1;
            }
        }
        assert!(pushed > 5, "beats came while he swung: {pushed}");
    }

    #[test]
    fn he_sways_side_to_side_within_his_most_and_settles_when_the_music_stops() {
        let mut dance = Dance::default();
        let (mut left, mut right) = (0.0_f64, 0.0_f64);
        for level in notes(700.0, 8000.0, 0.3) {
            dance.hear(level, DT);
            assert!(dance.lean_deg().abs() <= DANCE_LEAN_MAX_DEG);
            left = left.min(dance.lean_deg());
            right = right.max(dance.lean_deg());
        }
        assert!(
            left < -1.0 && right > 1.0,
            "he sways both ways: {left} to {right}"
        );
        let mut settled = None;
        for step in 0..600 {
            dance.hear(None, DT);
            if dance.still() {
                settled = Some(step);
                break;
            }
        }
        assert!(
            settled.is_some(),
            "the music stopped, so does he: {dance:?}"
        );
    }
}

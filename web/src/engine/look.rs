//! How Rambo is posed on top of where he is (ENGINE-PLAN step 2c): his lean, his size,
//! how far he sinks into the nest and how much of the writing pose he wears, all from
//! his state, his velocity and how long he has been in that state.
//!
//! Pure, so every pose is tested here. The page writes one [`Look`] as his button's
//! only transform (`Look::transform`); no style sheet rule moves the button.

use super::world::Vec2;
use crate::mascot::State;
use crate::tuning::{LEAN_EASE_MS, LEAN_MAX_DEG, LEAN_PER_PX_S, POSE_BLEND_MS, WRITE_BLEND_MS};

/// Held, he tilts like a pen in a hand.
const HELD_LEAN_DEG: f64 = -15.0;
/// Held, he pulses so he reads as alive, even where speech recognition is
/// unavailable: a little less tilt and a little bigger, once a cycle.
const HELD_PULSE_MS: f64 = 800.0;
const HELD_PULSE_LEAN_DEG: f64 = 3.0;
const HELD_PULSE_GROW: f64 = 0.05;
/// Dazed, he lies tipped over where he fell.
const DAZED_LEAN_DEG: f64 = 12.0;
/// Writing, he is a pen at work: smaller, at the theme's `--write-lean`.
const WRITING_SCALE: f64 = 0.78;
/// The splat: a big wobble that dies down into the daze's tilt, as
/// `(time from 0 to 1, lean in deg, scale)`.
const SPLAT_WOBBLE: [(f64, f64, f64); 6] = [
    (0.0, -15.0, 1.15),
    (0.2, 10.0, 0.95),
    (0.4, -6.0, 1.02),
    (0.6, 4.0, 0.98),
    (0.8, -2.0, 1.0),
    (1.0, DAZED_LEAN_DEG, 1.0),
];

/// His pose. Every field blends, so a change of state never snaps him.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Look {
    /// His own lean in degrees, clockwise.
    pub(crate) lean_deg: f64,
    pub(crate) scale: f64,
    /// How much of the theme's `--nest-sink` he is sunk by: 1 down in the nest.
    pub(crate) sink: f64,
    /// How much of the writing pose's `--write-lean` he wears.
    pub(crate) write: f64,
    /// How much of his writing hand (`--nib-dip`, `--nib-tilt`, set by `tasks/write.rs`)
    /// he wears: only while he writes a row.
    pub(crate) hand: f64,
    /// Where he turns and shrinks about: 0 his middle, 1 his nib (the bottom of his box,
    /// so writing a row the lean never moves the nib off the ink).
    pub(crate) pivot: f64,
}

impl Look {
    /// Upright, full size, about his middle.
    pub(crate) const PLAIN: Look = Look {
        lean_deg: 0.0,
        scale: 1.0,
        sink: 0.0,
        write: 0.0,
        hand: 0.0,
        pivot: 0.0,
    };

    /// The CSS transform that draws him in this pose with his middle at `at`. The pivot
    /// is a translation either side of the turn, so it blends with the rest.
    pub(crate) fn transform(&self, at: Vec2) -> String {
        let pivot = self.pivot * 50.0;
        format!(
            "translate(calc({x:.1}px - 50%), calc({y:.1}px - 50% + var(--nest-sink) * {sink:.3} + var(--nib-dip, 0px) * {hand:.3})) \
             translate(0, {pivot:.2}%) \
             rotate(calc({lean:.2}deg + var(--write-lean) * {write:.3} + var(--nib-tilt, 0deg) * {hand:.3})) \
             scale({scale:.3}) translate(0, -{pivot:.2}%)",
            x = at.0,
            y = at.1,
            sink = self.sink,
            hand = self.hand,
            lean = self.lean_deg,
            write = self.write,
            scale = self.scale,
        )
    }

    /// `self` a fraction `t` of the way to `to`.
    fn toward(&self, to: &Look, t: f64) -> Look {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        Look {
            lean_deg: mix(self.lean_deg, to.lean_deg),
            scale: mix(self.scale, to.scale),
            sink: mix(self.sink, to.sink),
            write: mix(self.write, to.write),
            hand: mix(self.hand, to.hand),
            pivot: mix(self.pivot, to.pivot),
        }
    }
}

/// The pose `state` asks for after `ms` in it: with reduced motion, the same pose
/// without the pulse or the wobble. His lean into a move comes on top (`sway_toward`).
pub(crate) fn pose(state: &State, ms: f64, reduced_motion: bool) -> Look {
    let plain = Look::PLAIN;
    let writing = Look {
        scale: WRITING_SCALE,
        write: 1.0,
        ..plain
    };
    match state {
        State::Home => Look { sink: 1.0, ..plain },
        State::Dragging if reduced_motion => Look {
            lean_deg: HELD_LEAN_DEG,
            ..plain
        },
        State::Dragging => {
            // 0 at rest, 1 at the top of the pulse.
            let pulse = (1.0 - (std::f64::consts::TAU * ms / HELD_PULSE_MS).cos()) / 2.0;
            Look {
                lean_deg: HELD_LEAN_DEG + HELD_PULSE_LEAN_DEG * pulse,
                scale: 1.0 + HELD_PULSE_GROW * pulse,
                ..plain
            }
        }
        State::Splatting if !reduced_motion => splat(ms),
        State::Dazed | State::Splatting => Look {
            lean_deg: DAZED_LEAN_DEG,
            ..plain
        },
        State::WritingCaret => writing,
        State::WritingRow => Look {
            hand: 1.0,
            pivot: 1.0,
            ..writing
        },
        // Hopping he stands upright; his lean into the move is his sway (`sway_toward`).
        // Caught under the leaf he stands upright, pushing.
        State::Flying
        | State::Skittering
        | State::Mending
        | State::Covered
        | State::Thrown
        | State::Inking
        | State::Asking { .. }
        | State::Noting => plain,
    }
}

/// The sway, in degrees, he leans toward while moving at `vel` px/s in `state`: into
/// where he is going, more the faster he goes, up to `LEAN_MAX_DEG`, while he hops or is
/// carried; upright otherwise, and always with reduced motion.
pub(crate) fn sway_toward(state: &State, vel: Vec2, reduced_motion: bool) -> f64 {
    match state {
        State::Flying | State::Skittering | State::Dragging | State::Thrown if !reduced_motion => {
            (vel.0 * LEAN_PER_PX_S).clamp(-LEAN_MAX_DEG, LEAN_MAX_DEG)
        }
        _ => 0.0,
    }
}

/// His sway after `dt` ms more of easing toward `toward`: a low-pass over
/// `LEAN_EASE_MS`, so his lean follows his speed without ever jumping, though a hop's
/// speed rises and falls within it and drops to nothing as it lands.
pub(crate) fn eased_sway(sway: f64, toward: f64, dt: f64) -> f64 {
    sway + (toward - sway) * (1.0 - (-dt / LEAN_EASE_MS).exp())
}

/// Below this, in degrees, his sway has died away and no longer moves him.
pub(crate) const SWAY_GONE_DEG: f64 = 0.05;

/// The splat's wobble `ms` in, along [`SPLAT_WOBBLE`] over `SPLAT_MS`.
fn splat(ms: f64) -> Look {
    let t = (ms / f64::from(crate::mascot::SPLAT_MS)).clamp(0.0, 1.0);
    let look = |(_, lean_deg, scale): (f64, f64, f64)| Look {
        lean_deg,
        scale,
        ..Look::PLAIN
    };
    let last = SPLAT_WOBBLE[SPLAT_WOBBLE.len() - 1];
    SPLAT_WOBBLE
        .windows(2)
        .find(|pair| t <= pair[1].0)
        .map_or(look(last), |pair| {
            let (from, to) = (pair[0], pair[1]);
            look(from).toward(&look(to), (t - from.0) / (to.0 - from.0))
        })
}

/// How long the change from `from` to `to` eases over: into or out of the writing
/// pose he turns by the theme's whole `--write-lean`, so that takes `WRITE_BLEND_MS`.
fn blend_ms(from: &Look, to: &Look) -> f64 {
    if from.write == to.write {
        POSE_BLEND_MS
    } else {
        WRITE_BLEND_MS
    }
}

/// His pose `ms` after a change of state that found him posed `from`: he eases into
/// the new pose (`blend_ms`), at once with reduced motion.
pub(crate) fn blended(from: &Look, to: &Look, ms: f64, reduced_motion: bool) -> Look {
    let over = blend_ms(from, to);
    if reduced_motion || ms >= over {
        return *to;
    }
    // Ease in and out: no frame of the change is a jump, at its start or its end.
    let x = ms / over;
    let t = x * x * (3.0 - 2.0 * x);
    from.toward(to, t)
}

/// Whether his pose stays the same from `ms` into `state` on, standing still: the
/// loop may sleep. The held pulse and the splat move on their own.
pub(crate) fn still(state: &State, ms: f64) -> bool {
    ms >= WRITE_BLEND_MS && !matches!(state, State::Dragging | State::Splatting)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled(state: &State) -> Look {
        pose(state, POSE_BLEND_MS, false)
    }

    #[test]
    fn at_home_he_sinks_into_the_nest_and_nowhere_else() {
        assert_eq!(settled(&State::Home).sink, 1.0);
        for state in [
            State::Dazed,
            State::Flying,
            State::WritingRow,
            State::Inking,
        ] {
            assert_eq!(settled(&state).sink, 0.0, "{state:?}");
        }
    }

    #[test]
    fn writing_he_is_a_smaller_pen_at_the_theme_lean_and_a_row_turns_about_his_nib() {
        let caret = settled(&State::WritingCaret);
        let row = settled(&State::WritingRow);
        assert_eq!(
            (caret.scale, caret.write, caret.pivot),
            (WRITING_SCALE, 1.0, 0.0)
        );
        assert_eq!(
            (row.scale, row.write, row.pivot, row.hand),
            (WRITING_SCALE, 1.0, 1.0, 1.0)
        );
        assert_eq!(
            caret.hand, 0.0,
            "the row's hand never moves him at the caret"
        );
    }

    #[test]
    fn he_leans_into_where_he_hops_or_is_carried_and_never_past_the_most() {
        let going = |state: &State, vx: f64| sway_toward(state, (vx, 0.0), false);
        assert_eq!(going(&State::Skittering, 0.0), 0.0, "standing, upright");
        assert!(going(&State::Flying, 3000.0) > 0.0 && going(&State::Flying, -3000.0) < 0.0);
        assert!(going(&State::Dragging, 3000.0) > 0.0);
        assert_eq!(going(&State::Skittering, 1e6), LEAN_MAX_DEG);
        assert_eq!(going(&State::Skittering, -1e6), -LEAN_MAX_DEG);
        assert_eq!(
            going(&State::WritingRow, 3000.0),
            0.0,
            "the nib stays on the ink"
        );
        assert_eq!(sway_toward(&State::Flying, (3000.0, 0.0), true), 0.0);
    }

    #[test]
    fn his_sway_follows_his_speed_but_never_jumps() {
        // The worst a hop does: full speed one step, nothing the next.
        let step = 1000.0 / 120.0;
        let mut sway = 0.0;
        for i in 0..240 {
            let toward = if i % 18 < 9 { LEAN_MAX_DEG } else { 0.0 };
            let next = eased_sway(sway, toward, step);
            assert!((next - sway).abs() < 1.0, "{sway} -> {next}");
            sway = next;
        }
        for _ in 0..240 {
            sway = eased_sway(sway, 0.0, step);
        }
        assert!(sway.abs() < SWAY_GONE_DEG, "it dies away: {sway}");
    }

    #[test]
    fn held_he_tilts_and_pulses_and_with_reduced_motion_only_tilts() {
        let top = pose(&State::Dragging, HELD_PULSE_MS / 2.0, false);
        let start = pose(&State::Dragging, 0.0, false);
        assert_eq!(start.lean_deg, HELD_LEAN_DEG);
        assert!(top.scale > start.scale && top.lean_deg > start.lean_deg);
        let still = pose(&State::Dragging, HELD_PULSE_MS / 2.0, true);
        assert_eq!((still.lean_deg, still.scale), (HELD_LEAN_DEG, 1.0));
    }

    #[test]
    fn the_splat_wobbles_and_ends_in_the_daze_tilt_without_a_jump() {
        let ms = f64::from(crate::mascot::SPLAT_MS);
        assert_eq!(pose(&State::Splatting, 0.0, false).scale, 1.15);
        let end = pose(&State::Splatting, ms, false);
        assert_eq!(end, settled(&State::Dazed));
        // Every 1/120 s of the wobble is a small move.
        let steps = (ms / (1000.0 / 120.0)) as usize;
        let looks: Vec<_> = (0..=steps)
            .map(|i| pose(&State::Splatting, ms * i as f64 / steps as f64, false))
            .collect();
        for pair in looks.windows(2) {
            assert!(
                (pair[1].lean_deg - pair[0].lean_deg).abs() < 5.0,
                "{pair:?}"
            );
        }
    }

    #[test]
    fn a_change_of_state_eases_into_the_new_pose_and_never_snaps() {
        // Flying (upright, about his middle) to writing a row (smaller, leaning, about
        // his nib): the step a CSS class switch used to make.
        let from = settled(&State::Flying);
        let to = settled(&State::WritingRow);
        let mut last = blended(&from, &to, 0.0, false);
        assert_eq!(last, from, "the first frame is where he was");
        for i in 1..=(WRITE_BLEND_MS as usize) {
            let now = blended(&from, &to, i as f64, false);
            assert!((now.scale - last.scale).abs() < 0.02, "{last:?} -> {now:?}");
            assert!((now.pivot - last.pivot).abs() < 0.05, "{last:?} -> {now:?}");
            last = now;
        }
        assert_eq!(last, to);
        assert_eq!(
            blended(&from, &to, 0.0, true),
            to,
            "reduced motion: at once"
        );
    }

    #[test]
    fn leaving_the_writing_pose_turns_him_a_little_each_frame() {
        // The theme's `--write-lean`, and a 60 Hz frame.
        let write_lean_deg = 34.0;
        let frame = 1000.0 / 60.0;
        let from = settled(&State::WritingCaret);
        let to = settled(&State::Flying);
        let mut last = blended(&from, &to, 0.0, false);
        for i in 1..=((WRITE_BLEND_MS / frame) as usize + 1) {
            let now = blended(&from, &to, i as f64 * frame, false);
            let turn = (now.write - last.write).abs() * write_lean_deg;
            assert!(turn < 3.0, "{turn} deg in one frame");
            last = now;
        }
        assert_eq!(last, to);
    }

    #[test]
    fn only_the_held_pulse_and_the_splat_keep_his_pose_moving() {
        assert!(!still(
            &State::Asking {
                task_id: String::new(),
                title: String::new()
            },
            0.0
        ));
        for state in [State::Home, State::Dazed, State::Inking, State::WritingRow] {
            assert!(still(&state, WRITE_BLEND_MS), "{state:?}");
        }
        assert!(!still(&State::Dragging, 1e6));
        assert!(!still(&State::Splatting, 1e6));
    }

    #[test]
    fn the_transform_places_his_middle_and_carries_every_part_of_the_pose() {
        let css = settled(&State::WritingRow).transform((12.0, 34.5));
        for part in [
            "calc(12.0px - 50%)",
            "calc(34.5px - 50%",
            "var(--write-lean) * 1.000",
            "var(--nib-tilt, 0deg) * 1.000",
            "scale(0.780)",
            "translate(0, 50.00%)",
            "translate(0, -50.00%)",
        ] {
            assert!(css.contains(part), "{part} in {css}");
        }
    }
}

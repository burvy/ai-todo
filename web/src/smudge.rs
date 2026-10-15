//! Ink dries (SPEC-GAPS I3 idea 1): a row Rambo has just written is glossy wet ink for
//! [`INK_WET_MS`], and a tap on it while it is wet smudges it. The smudge is ink: it
//! stays where the finger dragged it, and nothing of it fades.

use leptos::prelude::*;
use todo_core::TaskId;

use crate::tuning::{INK_WET_MS, SMUDGE_BLOBS, SMUDGE_REACH_PX};

/// Whether ink laid `age_ms` ago is still wet enough to smudge.
pub(crate) fn is_wet(age_ms: f64) -> bool {
    (0.0..INK_WET_MS).contains(&age_ms)
}

/// One smear of wet ink across a row: where the finger met it, in px from the row's
/// top left, and the blots it dragged the ink into.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Smudge {
    pub(crate) task: TaskId,
    pub(crate) blots: Vec<Blot>,
}

/// A blot of smeared ink: its middle, its half lengths along and across the drag, and
/// how much ink it holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Blot {
    pub(crate) at: (f64, f64),
    pub(crate) along: f64,
    pub(crate) across: f64,
    pub(crate) ink: f64,
}

/// A number in `0..1` from `seed` and `i`: the same smear for the same tap.
fn unit(seed: u32, i: u32) -> f64 {
    let mut x = seed ^ i.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    f64::from(x) / f64::from(u32::MAX)
}

/// The smear a fingertip leaves on wet ink at `at`: blots dragged a little way to the
/// right of it (the way a finger leaves a page), thinning and fading as the ink runs
/// out, all within [`SMUDGE_REACH_PX`] of the tap.
pub(crate) fn smear(task: TaskId, at: (f64, f64), seed: u32) -> Smudge {
    let blots = (0..SMUDGE_BLOBS)
        .map(|i| {
            let share = f64::from(i) / f64::from(SMUDGE_BLOBS.max(2) - 1);
            let along = SMUDGE_REACH_PX * (0.18 + 0.1 * unit(seed, i)) * (1.0 - 0.5 * share);
            let wobble = (unit(seed, i + 100) - 0.5) * SMUDGE_REACH_PX * 0.2;
            Blot {
                at: (at.0 + share * (SMUDGE_REACH_PX - along), at.1 + wobble),
                along,
                across: along * (0.35 + 0.2 * unit(seed, i + 200)),
                ink: 0.5 * (1.0 - 0.7 * share),
            }
        })
        .collect();
    Smudge { task, blots }
}

/// The smudges on row `task`, drawn in the row's own ink over its words. They take no
/// pointer.
#[component]
pub(crate) fn Smudges(task: TaskId, smudges: RwSignal<Vec<Smudge>>) -> impl IntoView {
    view! {
        <svg class="ink-smudges" aria-hidden="true">
            {move || {
                smudges.with(|all| {
                    all.iter()
                        .filter(|smudge| smudge.task == task)
                        .flat_map(|smudge| smudge.blots.clone())
                        .map(|blot| view! {
                            <ellipse
                                cx=format!("{:.1}", blot.at.0)
                                cy=format!("{:.1}", blot.at.1)
                                rx=format!("{:.1}", blot.along)
                                ry=format!("{:.1}", blot.across)
                                fill-opacity=format!("{:.2}", blot.ink)
                            />
                        })
                        .collect_view()
                })
            }}
        </svg>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_is_wet_only_for_a_while_after_it_is_laid() {
        assert!(is_wet(0.0));
        assert!(is_wet(INK_WET_MS - 1.0));
        assert!(!is_wet(INK_WET_MS));
        assert!(!is_wet(INK_WET_MS * 10.0));
        assert!(!is_wet(-1.0), "ink not laid yet is not wet");
    }

    #[test]
    fn a_smear_stays_by_the_finger_and_runs_out_the_way_it_was_dragged() {
        for seed in 0..500 {
            let at = (40.0 + f64::from(seed % 7), 12.0);
            let smudge = smear(TaskId("t".into()), at, seed);
            assert_eq!(smudge.blots.len(), SMUDGE_BLOBS as usize);
            for blot in &smudge.blots {
                let (dx, dy) = (blot.at.0 - at.0, blot.at.1 - at.1);
                assert!(dx >= 0.0, "dragged right, never back: {seed}");
                assert!(
                    dx + blot.along <= SMUDGE_REACH_PX + 1e-9
                        && dy.abs() + blot.across <= SMUDGE_REACH_PX,
                    "within reach of the tap: {seed} {blot:?}"
                );
                assert!(blot.along > 0.0 && blot.across > 0.0 && blot.across <= blot.along);
                assert!((0.0..=1.0).contains(&blot.ink));
            }
            // The ink runs out along the drag.
            assert!(
                smudge
                    .blots
                    .windows(2)
                    .all(|pair| pair[1].ink < pair[0].ink)
            );
            assert_eq!(
                smudge,
                smear(TaskId("t".into()), at, seed),
                "the same tap, the same smear"
            );
        }
    }
}

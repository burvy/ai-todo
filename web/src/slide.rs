//! Paper sliding in the planner and the edit sheet: the ribbon's head hopping to another
//! day, a stretched slip, the placing note and the sheet coming into sight and leaving it.
//! The engine runs it on one spring (`tuning::PAPER_SLIDE_OMEGA`); with reduced motion
//! the paper is where it is going at once.

use leptos::prelude::*;

use crate::engine::Pull;

/// The spring paper slides on: critically damped, so it never bounces.
const PULL: Pull = Pull {
    omega: crate::tuning::PAPER_SLIDE_OMEGA,
    zeta: 1.0,
};

/// How near its end an amount in sight (0 out of sight, 1 in it) must come to count as
/// there.
const SHOWN_NEAR: f64 = 0.001;

/// Slides the paper `key` names from `from` to `to`, drawing every place with `draw`
/// (`near` is how close to `to` counts as there), then runs `done`. A new slide of the
/// same key takes over, and the old one's `done` never runs.
pub(crate) fn slide(
    key: &str,
    from: f64,
    to: f64,
    near: f64,
    mut draw: impl FnMut(f64) + 'static,
    done: impl FnOnce() + 'static,
) {
    if crate::mascot::prefers_reduced_motion() {
        crate::engine::cancel(key);
        draw(to);
        done();
        return;
    }
    crate::engine::glide_keyed(key, [from], [to], PULL, near, move |[at]| draw(at), done);
}

/// Slides `shown`, how far the paper `key` names is in sight, to `to`, then runs `done`.
pub(crate) fn show(key: &str, shown: RwSignal<f64>, to: f64, done: impl FnOnce() + 'static) {
    slide(
        key,
        shown.get_untracked(),
        to,
        SHOWN_NEAR,
        move |at| shown.set(at),
        done,
    );
}

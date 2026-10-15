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

/// A node reference for an element that arrives when it is put on the page (`arrive`).
pub(crate) fn arriving<E>() -> NodeRef<E>
where
    E: leptos::tachys::html::element::ElementType,
    E::Output: wasm_bindgen::JsCast + Clone + 'static,
{
    use wasm_bindgen::JsCast as _;
    let node = NodeRef::<E>::new();
    node.on_load(|el| {
        if let Ok(el) = el.dyn_into::<web_sys::HtmlElement>() {
            arrive(&el);
        }
    });
    node
}

/// An element arriving on the page: its `--arrive` goes from 0 to 1 on the paper's spring,
/// and `data-arriving` marks it while that lasts, so the style sheet draws the arrival
/// (an unfold, a drop, a slot opening) only then and the element rests as it is drawn.
/// With reduced motion it is simply there.
pub(crate) fn arrive(el: &web_sys::HtmlElement) {
    if crate::mascot::prefers_reduced_motion() {
        return;
    }
    let _ = el.set_attribute("data-arriving", "");
    let (drawn, finished) = (el.clone(), el.clone());
    crate::engine::glide(
        [0.0],
        [1.0],
        PULL,
        crate::tuning::SHOWN_NEAR,
        move |[at]| {
            let _ = drawn.style().set_property("--arrive", &format!("{at:.4}"));
        },
        move || {
            let _ = finished.style().remove_property("--arrive");
            let _ = finished.remove_attribute("data-arriving");
        },
    );
}

/// Slides `shown`, how far the paper `key` names is in sight, to `to`, then runs `done`.
pub(crate) fn show(key: &str, shown: RwSignal<f64>, to: f64, done: impl FnOnce() + 'static) {
    slide(
        key,
        shown.get_untracked(),
        to,
        crate::tuning::SHOWN_NEAR,
        move |at| shown.set(at),
        done,
    );
}

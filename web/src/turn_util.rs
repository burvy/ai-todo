//! Small browser helpers the two page turns share (`flip_turner.rs`, `fold_turner.rs`).

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement};

use crate::tuning::PHONE_MEDIA;

/// Presses that start here are not page turns: they have their own scroll or swipe.
const OWN_GESTURE: &str = ".task-dialog, .tack-board, input, textarea, select, .paper-picker";
/// Things on the planner's grid that a press picks up or taps, so it is not empty paper.
const ON_THE_GRID: &str = ".placement, .landing";

/// Whether the page is laid out as a phone's two pages.
pub(crate) fn phone() -> bool {
    window()
        .match_media(PHONE_MEDIA)
        .ok()
        .flatten()
        .is_some_and(|query| query.matches())
}

pub(crate) fn now() -> f64 {
    js_sys::Date::now()
}

pub(crate) fn el_style(el: &Element, property: &str, value: &str) {
    if let Some(html) = el.dyn_ref::<HtmlElement>() {
        let _ = html.style().set_property(property, value);
    }
}

pub(crate) fn element_of(ev: &leptos::ev::PointerEvent) -> Option<Element> {
    ev.target()
        .and_then(|target| target.dyn_into::<Element>().ok())
}

/// Whether a press landed where it may start a page turn. The planner's grid is the
/// planner's own to scroll, except on its earliest day: there nothing is left to scroll the
/// way a back swipe goes, and a press on empty paper turns the page.
pub(crate) fn may_turn_from(target: &Element) -> bool {
    if target.closest(OWN_GESTURE).ok().flatten().is_some() {
        return false;
    }
    match target.closest(".week-scroll").ok().flatten() {
        None => true,
        Some(grid) => {
            grid.scroll_left() <= 1 && target.closest(ON_THE_GRID).ok().flatten().is_none()
        }
    }
}

/// Shows page `index` (0 the list, 1 the planner): the attribute the stylesheet reads is
/// set now, so the page that lands is drawn in the same frame the turn ends, and the
/// signal follows.
pub(crate) fn show_pane(main: &Element, index: u8, set_pane: WriteSignal<u8>) {
    let _ = main.set_attribute("data-pane", &index.to_string());
    set_pane.set(index);
}

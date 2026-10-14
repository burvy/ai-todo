//! Small browser helpers the two page turns share (`flip_turner.rs`, `fold_turner.rs`).

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement};

use crate::tuning::PHONE_MEDIA;

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

/// Shows page `index` (0 the list, 1 the planner): the attribute the stylesheet reads is
/// set now, so the page that lands is drawn in the same frame the turn ends, and the
/// signal follows.
pub(crate) fn show_pane(main: &Element, index: u8, set_pane: WriteSignal<u8>) {
    let _ = main.set_attribute("data-pane", &index.to_string());
    set_pane.set(index);
}

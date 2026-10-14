//! The two pages on a phone, the list and the planner, and turning between them.

use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

/// The pane in view on a phone: 0 for tasks, 1 for the week. The two panes sit side by
/// side in a scroll-snap row, so the pane is the scroll offset in whole pane widths.
#[cfg(any(target_arch = "wasm32", test))]
fn pane_at(scroll_left: f64, pane_width: f64) -> u8 {
    if pane_width <= 0.0 {
        return 0;
    }
    (scroll_left / pane_width).round().clamp(0.0, 1.0) as u8
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn show_pane(main: NodeRef<leptos::html::Main>, index: u8, set_pane: WriteSignal<u8>) {
    if let Some(element) = main.get() {
        // Scroll to the pane's own offset, not a multiple of the width: the grid gap
        // sits between panes, so index * client_width lands short of the planner.
        if let Some(pane) = element.children().item(u32::from(index)) {
            let left = pane.unchecked_into::<web_sys::HtmlElement>().offset_left();
            element.set_scroll_left(left);
        }
    }
    set_pane.set(index);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn show_pane(_main: NodeRef<leptos::html::Main>, index: u8, set_pane: WriteSignal<u8>) {
    set_pane.set(index);
}

/// The page `main` is scrolled to, nearest pane first.
#[cfg(target_arch = "wasm32")]
pub(crate) fn nearest_pane(element: &web_sys::HtmlElement) -> u8 {
    pane_at(
        f64::from(element.scroll_left()),
        f64::from(element.client_width()),
    )
}

/// Milliseconds of quiet after the last scroll event before the turn is called
/// over. `scrollend` is not everywhere yet, and a fixed guess is worse than the
/// scroll going quiet: while the pages are still moving, scroll events keep
/// arriving and the curl keeps its lean.
#[cfg(target_arch = "wasm32")]
pub(crate) const SETTLE_MS: u32 = 220;

/// Which way the curl leans while the pages are turning, and nothing at all
/// when the scroll has not left the page it settled on. `from` is the page the
/// last turn ended on, `nearest` the page the scroll is closest to now.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn curl_for(from: u8, nearest: u8) -> Option<bool> {
    (from != nearest).then_some(nearest == 1)
}

#[cfg(test)]
mod tests {
    use super::{curl_for, pane_at};

    #[test]
    fn a_phone_swipe_maps_to_the_nearest_pane() {
        assert_eq!(pane_at(0.0, 400.0), 0);
        assert_eq!(pane_at(180.0, 400.0), 0);
        assert_eq!(pane_at(220.0, 400.0), 1);
        assert_eq!(pane_at(400.0, 400.0), 1);
        assert_eq!(pane_at(900.0, 400.0), 1, "never past the last pane");
        assert_eq!(pane_at(100.0, 0.0), 0, "no width yet, so the first pane");
    }

    #[test]
    fn a_turn_leans_the_curl_towards_the_page_it_is_going_to() {
        // The scroll is the turn now: whatever direction the finger went, the
        // curl leans the way the pages are travelling.
        assert_eq!(curl_for(0, 1), Some(true), "list → planner");
        assert_eq!(curl_for(1, 0), Some(false), "planner → list");
    }

    #[test]
    fn a_scroll_that_stays_on_its_page_is_not_a_turn() {
        // Vertical scrolling, a rubber-band at the end, and the quiet after a
        // turn has settled all leave the pages where they were: no curl.
        assert_eq!(curl_for(0, 0), None);
        assert_eq!(curl_for(1, 1), None);
    }
}

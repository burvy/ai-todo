//! The two pages on a phone, the list and the planner, and turning between them.

#[cfg(target_arch = "wasm32")]
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

/// The page `main` is scrolled to, nearest pane first.
#[cfg(target_arch = "wasm32")]
pub(crate) fn nearest_pane(element: &web_sys::HtmlElement) -> u8 {
    pane_at(
        f64::from(element.scroll_left()),
        f64::from(element.client_width()),
    )
}

#[cfg(test)]
mod tests {
    use super::pane_at;

    /// The body of the first rule in `css` whose selector line is exactly `selector`.
    fn rule<'a>(css: &'a str, selector: &str) -> &'a str {
        let start = css
            .find(&format!(
                "
{selector} {{"
            ))
            .unwrap_or_else(|| panic!("no `{selector}` rule"));
        let body = &css[start..];
        &body[..body
            .find(
                "
}",
            )
            .expect("the rule ends")]
    }

    /// BUG-9: `.week` clips its overflow (so the sticky day names stick to
    /// `.week-scroll`), and a clipped box adds nothing to its parent's scroll width. If
    /// `.week` is only as wide as the screen, the later days are cut off and cannot be
    /// scrolled to. It must be at least as wide as its columns on a phone, and may shrink
    /// to the pane on a wide screen.
    #[test]
    fn the_week_is_as_wide_as_its_columns_so_every_day_can_be_scrolled_to() {
        let css = include_str!("../theme.css");
        let week = rule(css, ".week");
        assert!(
            week.contains("overflow-x: clip"),
            "the clip is why the width matters"
        );
        let width = week
            .lines()
            .skip_while(|line| !line.contains("min-width: calc("))
            .take(3)
            .collect::<String>();
        for part in ["--planner-gutter", "--days", "--day-min-w", "--space-1"] {
            assert!(
                width.contains(part),
                "the week's min-width must count {part}: {width}"
            );
        }
        let wide = &css[css
            .find("@media (min-width: 701px)")
            .expect("the wide rules")..];
        assert!(
            rule(wide, "  .week").contains("min-width: 0"),
            "wide screens fit the pane"
        );
    }

    /// GESTURES FINAL: inside the grid a right swipe scrolls the days back and stops at
    /// the first day; only a swipe from outside the grid turns the page to the list.
    #[test]
    fn a_sideways_swipe_in_the_grid_does_not_turn_the_page() {
        let css = include_str!("../theme.css");
        assert!(rule(css, ".week-scroll").contains("overscroll-behavior-x: contain"));
        assert!(rule(css, "main").contains("scroll-snap-type: x mandatory"));
    }

    #[test]
    fn a_phone_swipe_maps_to_the_nearest_pane() {
        assert_eq!(pane_at(0.0, 400.0), 0);
        assert_eq!(pane_at(180.0, 400.0), 0);
        assert_eq!(pane_at(220.0, 400.0), 1);
        assert_eq!(pane_at(400.0, 400.0), 1);
        assert_eq!(pane_at(900.0, 400.0), 1, "never past the last pane");
        assert_eq!(pane_at(100.0, 0.0), 0, "no width yet, so the first pane");
    }
}

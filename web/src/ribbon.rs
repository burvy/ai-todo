//! The bookmark ribbon (SPEC-GAPS I3 idea 3) marks today's column in two parts. Its head
//! is the control: it sticks out of the band of day names at today's column and, with
//! today scrolled away, at the planner's edge, the way a bookmark sticks out of a closed
//! book. Its tail lies on today's hours under the slips, so it never covers a word.
//! Pull the head down (or tap it) and the planner slides back to today, at the hour it
//! opens on.

use leptos::prelude::*;

use crate::tuning::{RIBBON_PULL_MAX_PX, RIBBON_PULL_PX, RIBBON_TAP_PX};

/// How far the ribbon stretches for a finger pulled `dy` px down: one for one at first,
/// then stiffer, never past [`RIBBON_PULL_MAX_PX`]; pushed up, not at all.
pub(crate) fn pulled(dy: f64) -> f64 {
    RIBBON_PULL_MAX_PX * (1.0 - (-dy.max(0.0) / RIBBON_PULL_MAX_PX).exp())
}

/// Whether letting go after pulling `dy` px takes the planner back to today.
pub(crate) fn jumps(dy: f64) -> bool {
    pulled(dy) >= RIBBON_PULL_PX
}

/// Where the ribbon's head hangs, from the left of the planner page: just inside the
/// right edge of a day's column (`day_rights`, left to right, today first; `width` the
/// ribbon's own), where the band is empty because a day's name is written at its left.
/// It is today's while that spot is in sight between `lo` and `hi`, the paper's visible
/// edges; with today scrolled away, the first day whose spot is, so it waits at the
/// paper's edge and still covers no name. Held at `hi` if none is.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn hang_left(
    day_rights: impl IntoIterator<Item = f64>,
    width: f64,
    lo: f64,
    hi: f64,
) -> f64 {
    day_rights
        .into_iter()
        .map(|right| right - 1.5 * width)
        .find(|spot| *spot >= lo)
        .map_or(hi, |spot| spot.min(hi.max(lo)))
}

/// Where a slide back to today is `ms` into its [`crate::tuning::RIBBON_GLIDE_MS`], from `from` to
/// `to`: easing in and out, there exactly at the end and after.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn glide(from: f64, to: f64, ms: f64) -> f64 {
    let x = (ms / crate::tuning::RIBBON_GLIDE_MS).clamp(0.0, 1.0);
    from + (to - from) * x * x * (3.0 - 2.0 * x)
}

/// The ribbon's head at `left`, `top` on the planner page, `height` tall (the band of
/// day names). A pull past [`RIBBON_PULL_PX`] or a tap runs `back` (to today); `pull` is
/// how far it is stretched, which the tail draws; let go, it springs back.
#[component]
pub(crate) fn Ribbon(
    left: ReadSignal<f64>,
    top: ReadSignal<f64>,
    height: ReadSignal<f64>,
    pull: RwSignal<f64>,
    back: Callback<()>,
) -> impl IntoView {
    // Where the finger went down, while it holds the ribbon.
    let held = StoredValue::new(None::<(i32, f64)>);
    // The last press was a pull, not a tap: its click does nothing more.
    let was_pull = StoredValue::new(false);
    let dy_of = move |ev: &leptos::ev::PointerEvent| {
        held.get_value()
            .filter(|(id, _)| *id == ev.pointer_id())
            .map(|(_, y)| f64::from(ev.client_y()) - y)
    };
    view! {
        <button
            type="button"
            class="bookmark-ribbon"
            class:pulling=move || { pull.get() > 0.0 }
            aria-label="Back to today"
            style:left=move || format!("{:.1}px", left.get())
            style:top=move || format!("{:.1}px", top.get())
            style:height=move || format!("{:.1}px", height.get())
            on:pointerdown=move |ev| {
                held.set_value(Some((ev.pointer_id(), f64::from(ev.client_y()))));
                was_pull.set_value(false);
                #[cfg(target_arch = "wasm32")]
                {
                    use wasm_bindgen::JsCast as _;
                    if let Some(el) = ev
                        .current_target()
                        .and_then(|el| el.dyn_into::<web_sys::Element>().ok())
                    {
                        let _ = el.set_pointer_capture(ev.pointer_id());
                    }
                }
            }
            on:pointermove=move |ev| {
                if let Some(dy) = dy_of(&ev) {
                    pull.set(pulled(dy));
                }
            }
            on:pointerup=move |ev| {
                if let Some(dy) = dy_of(&ev) {
                    was_pull.set_value(dy.abs() > RIBBON_TAP_PX);
                    if jumps(dy) {
                        back.run(());
                    }
                }
                held.set_value(None);
                pull.set(0.0);
            }
            on:pointercancel=move |_| {
                held.set_value(None);
                pull.set(0.0);
            }
            on:click=move |_| {
                if !was_pull.get_value() {
                    back.run(());
                }
                was_pull.set_value(false);
            }
        ></button>
    }
}

/// The ribbon's tail on today's hours: cloth only, under the slips, stretched by the
/// head's `pull`. It takes no touch; the head is what a finger holds.
#[component]
pub(crate) fn RibbonTail(pull: RwSignal<f64>) -> impl IntoView {
    view! {
        <div
            class="ribbon-tail"
            class:pulling=move || { pull.get() > 0.0 }
            style:--pull=move || format!("{:.1}px", pull.get())
            aria-hidden="true"
        ></div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::RIBBON_GLIDE_MS;

    #[test]
    fn the_ribbon_stretches_with_the_pull_ever_stiffer_and_never_past_its_most() {
        let mut was = pulled(0.0);
        assert_eq!(was, 0.0);
        assert_eq!(pulled(-40.0), 0.0, "pushed up, it does not stretch");
        for step in 1..=2000 {
            let dy = f64::from(step) * 0.5;
            let now = pulled(dy);
            assert!(now > was, "more pull, more stretch: {dy}");
            assert!(now <= dy, "never more than the finger moved: {dy}");
            assert!(now < RIBBON_PULL_MAX_PX, "never past its most: {dy}");
            was = now;
        }
    }

    #[test]
    fn only_a_real_pull_takes_the_planner_back() {
        assert!(!jumps(0.0));
        assert!(!jumps(RIBBON_TAP_PX), "a tap's tremble is not a pull");
        assert!(!jumps(-200.0));
        // The first finger position that jumps, and everything past it.
        let first = (0..2000)
            .map(f64::from)
            .find(|dy| jumps(*dy))
            .expect("a long pull jumps");
        assert!(
            first >= RIBBON_PULL_PX,
            "the stretch, not the finger, decides"
        );
        assert!(
            (0..2000)
                .map(f64::from)
                .filter(|dy| *dy >= first)
                .all(jumps)
        );
    }

    #[test]
    fn the_ribbon_hangs_at_today_or_at_the_edge_today_went_past_never_on_a_name() {
        let (lo, hi, width, column) = (0.0, 360.0, 18.0, 120.0);
        // Seven columns, scrolled left by `scroll` px: today is the first.
        for scroll in 0..=(7 * 120 - 360) {
            let rights: Vec<f64> = (1..=7)
                .map(|day| f64::from(day) * column - f64::from(scroll))
                .collect();
            let at = hang_left(rights.iter().copied(), width, lo, hi);
            assert!((lo..=hi).contains(&at), "in sight: {scroll}");
            // Always just inside some day's right edge, where its band has no name.
            assert!(
                rights.iter().any(|right| at == right - 1.5 * width),
                "inside a column's right edge: {scroll}"
            );
            if rights[0] - 1.5 * width >= lo {
                assert_eq!(at, rights[0] - 1.5 * width, "on today's column: {scroll}");
            } else {
                assert!(at < lo + column, "at the edge today went past: {scroll}");
            }
        }
        assert_eq!(
            hang_left([], width, lo, hi),
            hi,
            "no days: held at the edge"
        );
    }

    /// The tail is cloth on the page: the slips lie over it and a finger goes through it
    /// to them; the head, in the band above, is what a finger holds.
    #[test]
    fn the_tail_lies_under_the_slips_and_never_takes_a_touch() {
        let css = include_str!("../theme.css");
        let rule = |selector: &str| {
            // Every block for the selector alone, at the start of a line.
            let blocks: Vec<&str> = css
                .split(&format!("\n{selector} {{"))
                .skip(1)
                .filter_map(|rest| rest.split('}').next())
                .collect();
            assert!(!blocks.is_empty(), "theme.css styles {selector}");
            blocks.concat()
        };
        let layer = |name: &str| -> i32 {
            css.split(&format!("--layer-{name}:"))
                .nth(1)
                .and_then(|rest| rest.split(';').next())
                .and_then(|n| n.trim().parse().ok())
                .unwrap_or_else(|| panic!("--layer-{name} is a number"))
        };
        let tail = rule(".ribbon-tail");
        assert!(tail.contains("pointer-events: none"), "{tail}");
        assert!(tail.contains("z-index: var(--layer-ribbon-tail)"), "{tail}");
        assert!(layer("ribbon-tail") < layer("page"), "under the slips");
        assert!(
            rule(".bookmark-ribbon").contains("z-index: var(--layer-note)"),
            "the head is over the day names"
        );
        assert!(layer("note") > layer("sticky"));
    }

    #[test]
    fn the_slide_back_eases_from_where_it_was_to_today_and_stays() {
        for (from, to) in [(900.0, 0.0), (0.0, 512.0), (37.0, 37.0)] {
            assert_eq!(glide(from, to, 0.0), from);
            assert_eq!(glide(from, to, RIBBON_GLIDE_MS), to);
            assert_eq!(glide(from, to, RIBBON_GLIDE_MS * 3.0), to);
            let steps: Vec<f64> = (0..=60)
                .map(|i| glide(from, to, RIBBON_GLIDE_MS * f64::from(i) / 60.0))
                .collect();
            // Always on the way, never past it.
            assert!(
                steps
                    .windows(2)
                    .all(|pair| (pair[1] - pair[0]) * (to - from) >= 0.0)
            );
            assert!(
                steps
                    .iter()
                    .all(|at| (from.min(to)..=from.max(to)).contains(at))
            );
        }
    }
}

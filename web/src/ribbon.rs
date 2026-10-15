//! The bookmark ribbon (SPEC-GAPS I3 idea 3) marks today's column in two parts. Its head
//! is the control: it sticks out of the band of day names at today's column and, with
//! today scrolled away, at the planner's edge, the way a bookmark sticks out of a closed
//! book. Its tail lies on today's hours under the slips, so it never covers a word.
//! Tap the head and the planner slides back to today, at the hour it opens on.

use leptos::prelude::*;

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
/// day names). A tap runs `back` (to today).
#[component]
pub(crate) fn Ribbon(
    left: ReadSignal<f64>,
    top: ReadSignal<f64>,
    height: ReadSignal<f64>,
    back: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="bookmark-ribbon"
            aria-label="Back to today"
            style:left=move || format!("{:.1}px", left.get())
            style:top=move || format!("{:.1}px", top.get())
            style:height=move || format!("{:.1}px", height.get())
            on:click=move |_| back.run(())
        ></button>
    }
}

/// The ribbon's tail on today's hours: cloth only, under the slips. It takes no touch;
/// the head is what a finger taps.
#[component]
pub(crate) fn RibbonTail() -> impl IntoView {
    view! { <div class="ribbon-tail" aria-hidden="true"></div> }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::RIBBON_GLIDE_MS;

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

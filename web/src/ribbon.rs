//! The bookmark ribbon (SPEC-GAPS I3 idea 3): a ribbon hangs from the top of the planner
//! at today's column and marks it; scrolled away, it stays at the planner's edge, the
//! way a bookmark sticks out of a closed book. Pull it down (or tap it) and the planner
//! slides back to today, at the hour it opens on.

use leptos::prelude::*;

use crate::tuning::{RIBBON_GLIDE_MS, RIBBON_PULL_MAX_PX, RIBBON_PULL_PX, RIBBON_TAP_PX};

/// How far the ribbon stretches for a finger pulled `dy` px down: one for one at first,
/// then stiffer, never past [`RIBBON_PULL_MAX_PX`]; pushed up, not at all.
pub(crate) fn pulled(dy: f64) -> f64 {
    RIBBON_PULL_MAX_PX * (1.0 - (-dy.max(0.0) / RIBBON_PULL_MAX_PX).exp())
}

/// Whether letting go after pulling `dy` px takes the planner back to today.
pub(crate) fn jumps(dy: f64) -> bool {
    pulled(dy) >= RIBBON_PULL_PX
}

/// Where the ribbon hangs, from the left of the planner page: just inside the right edge
/// of today's column (`today_right`, `width` the ribbon's own), held between `lo` and
/// `hi`, the planner's visible edges, when today is scrolled away.
pub(crate) fn hang_left(today_right: f64, width: f64, lo: f64, hi: f64) -> f64 {
    (today_right - 1.5 * width).clamp(lo, hi.max(lo))
}

/// Where a slide back to today is `ms` into its [`RIBBON_GLIDE_MS`], from `from` to
/// `to`: easing in and out, there exactly at the end and after.
pub(crate) fn glide(from: f64, to: f64, ms: f64) -> f64 {
    let x = (ms / RIBBON_GLIDE_MS).clamp(0.0, 1.0);
    from + (to - from) * x * x * (3.0 - 2.0 * x)
}

/// The ribbon at `left`, `top` on the planner page. A pull past [`RIBBON_PULL_PX`] or a
/// tap runs `back` (to today); let go, it springs back to its length.
#[component]
pub(crate) fn Ribbon(
    left: ReadSignal<f64>,
    top: ReadSignal<f64>,
    back: Callback<()>,
) -> impl IntoView {
    let pull = RwSignal::new(0.0_f64);
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
            style:--pull=move || format!("{:.1}px", pull.get())
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_ribbon_hangs_at_today_or_at_the_edge_today_went_past() {
        let (lo, hi, width) = (52.0, 360.0, 18.0);
        for today_right in (-400..800).map(f64::from) {
            let at = hang_left(today_right, width, lo, hi);
            assert!((lo..=hi).contains(&at), "{today_right}");
            if (lo..=hi).contains(&(today_right - 1.5 * width)) {
                assert_eq!(at, today_right - 1.5 * width, "on today's column");
            }
        }
        assert_eq!(
            hang_left(-300.0, width, lo, hi),
            lo,
            "today scrolled off to the left"
        );
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

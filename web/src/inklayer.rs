//! The ink surface: the canvas the trail and the splats are painted on.
//!
//! The rules live in [`rambl_web_core::dabs`] and are pure — this is the thin part that
//! puts their dabs on the page. It sits above the page and below
//! Rambo (DESIGN-DIRECTION: the trail is a layer above the page, below Rambo,
//! and never over the capture line or a dialog), and it never takes a pointer
//! itself: Rambo is the thing you pull, and the trail is what follows him.
//!
//! Ink ages on the page (`ink::age_alpha`), so while any is still drying or
//! fading the canvas is repainted every frame; once the last stroke has faded it
//! is dropped and the frames stop.

use crate::ink::{Stroke, Surface};
use leptos::prelude::*;
use rambl_web_core::dabs::Laid;
#[cfg(target_arch = "wasm32")]
use rambl_web_core::dabs::{Batch, batches};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

/// The ink a dazed Rambo leaves, and the splats he throws, on one surface: painted
/// from the `strokes` laid on it, aged frame by frame, and each stroke dropped from
/// `strokes` once it has faded.
#[component]
pub fn InkLayer(strokes: RwSignal<Vec<Stroke>>, on: Surface) -> impl IntoView {
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    // The dabs are worked out when a stroke changes, not on every frame.
    let laid = Memo::new(move |_| {
        strokes.with(|all| {
            all.iter()
                .filter(|stroke| stroke.on == on)
                .map(Laid::of)
                .collect::<Vec<_>>()
        })
    });
    let painting = std::rc::Rc::new(std::cell::Cell::new(false));
    Effect::new(move |_| {
        if !laid.with(Vec::is_empty) && !painting.get() {
            painting.set(true);
            run_frames(canvas_ref, laid, strokes, painting.clone());
        }
    });

    match on {
        // The page's sheet sticks to the window inside the page, so it is never taller
        // than the screen however long the list is.
        Surface::List => view! {
            <div class="page-ink" aria-hidden="true">
                <canvas class="ink-sheet" node_ref=canvas_ref />
            </div>
        }
        .into_any(),
        Surface::Floor => view! {
            <canvas class="ink-floor" node_ref=canvas_ref aria-hidden="true" />
        }
        .into_any(),
    }
}

/// Paint on every frame while any ink is on the page; stop once it has all faded.
#[cfg(target_arch = "wasm32")]
fn run_frames(
    canvas: NodeRef<leptos::html::Canvas>,
    laid: Memo<Vec<Laid>>,
    strokes: RwSignal<Vec<Stroke>>,
    painting: std::rc::Rc<std::cell::Cell<bool>>,
) {
    crate::engine::on_tick(move |tick| {
        let now = tick.now;
        let Some(context) = ink_context(&canvas) else {
            painting.set(false);
            return crate::engine::Flow::Stop;
        };
        fresh_sheet(&canvas, &context);
        let on_page = laid.with_untracked(|laid| batches(laid, now));
        for batch in &on_page {
            paint(&context, batch);
        }
        // Strokes that have faded are done with.
        if strokes.with_untracked(|all| all.iter().any(|s| crate::ink::faded(now - s.laid_at()))) {
            strokes.update(|all| all.retain(|s| !crate::ink::faded(now - s.laid_at())));
        }
        if laid.with_untracked(Vec::is_empty) {
            // The last ink on this surface has gone: a clean sheet, and the frames stop.
            fresh_sheet(&canvas, &context);
            if let Some(sheet) = canvas.get_untracked() {
                page_layer(&sheet, None);
            }
            painting.set(false);
            return crate::engine::Flow::Stop;
        }
        crate::engine::Flow::Go
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn run_frames(
    _canvas: NodeRef<leptos::html::Canvas>,
    _laid: Memo<Vec<Laid>>,
    _strokes: RwSignal<Vec<Stroke>>,
    _painting: std::rc::Rc<std::cell::Cell<bool>>,
) {
}

/// One batch, filled as one path. The ink already under it is cleared first, so a
/// dab's strength is its own and overlapping dabs never darken into beads; with the
/// batches faintest first, the strongest ink wins wherever two meet.
#[cfg(target_arch = "wasm32")]
fn paint(context: &web_sys::CanvasRenderingContext2d, batch: &Batch) {
    let Ok(colour) = token_colour(batch.token) else {
        return;
    };
    context.begin_path();
    for &(x, y, radius) in &batch.dabs {
        context.move_to(x + radius, y);
        let _ = context.arc(x, y, radius, 0.0, std::f64::consts::TAU);
    }
    // The canvas holds only ink, so clearing under the path touches nothing else.
    let _ = context.set_global_composite_operation("destination-out");
    context.set_global_alpha(1.0);
    context.fill();
    let _ = context.set_global_composite_operation("source-over");
    context.set_global_alpha(batch.alpha);
    context.set_fill_style_str(&colour);
    context.fill();
    context.set_global_alpha(1.0);
}

#[cfg(target_arch = "wasm32")]
fn ink_context(
    canvas: &NodeRef<leptos::html::Canvas>,
) -> Option<web_sys::CanvasRenderingContext2d> {
    canvas
        .get_untracked()?
        .get_context("2d")
        .ok()
        .flatten()?
        .dyn_into::<web_sys::CanvasRenderingContext2d>()
        .ok()
}

/// A blank canvas the size of the page, in CSS pixels: the backing store
/// matches the screen's pixels, so the ink lands on real pixels and not on a
/// stretched low-resolution copy, and every dab of the last paint is gone.
#[cfg(target_arch = "wasm32")]
fn fresh_sheet(
    canvas: &NodeRef<leptos::html::Canvas>,
    context: &web_sys::CanvasRenderingContext2d,
) {
    let Some(canvas) = canvas.get_untracked() else {
        return;
    };
    // The page's layer is as tall as all the rows, scrolled past or not, so its sheet can
    // stick to the part of the page that shows wherever the rows have got to.
    let rows = canvas
        .parent_element()
        .and_then(|layer| layer.parent_element())
        .map(|page| page.scroll_height());
    page_layer(&canvas, rows);
    let ratio = window().device_pixel_ratio();
    let (w, h) = (
        (f64::from(canvas.client_width()) * ratio) as u32,
        (f64::from(canvas.client_height()) * ratio) as u32,
    );
    // Setting the size reallocates the backing store, so only when it changed.
    if canvas.width() != w || canvas.height() != h {
        canvas.set_width(w);
        canvas.set_height(h);
    }
    let _ = context.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    context.clear_rect(0.0, 0.0, f64::from(w), f64::from(h));
    // The sheet shows the part of its surface it is over: the page's canvas sticks to
    // the window as the page scrolls past it, so the ink is drawn shifted by where the
    // canvas has got to on the page. The floor's sits at the window's corner.
    let (dx, dy) = (
        f64::from(canvas.offset_left()),
        f64::from(canvas.offset_top()),
    );
    let _ = context.set_transform(ratio, 0.0, 0.0, ratio, -dx * ratio, -dy * ratio);
}

/// Sizes the list page's ink layer to `height` px, or back to nothing; the floor's
/// canvas has no layer and is left alone.
#[cfg(target_arch = "wasm32")]
fn page_layer(canvas: &web_sys::HtmlCanvasElement, height: Option<i32>) {
    let Some(layer) = canvas
        .parent_element()
        .filter(|layer| layer.class_list().contains("page-ink"))
        .and_then(|layer| layer.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return;
    };
    let height = height.map_or(String::new(), |px| format!("{px}px"));
    let _ = layer.style().set_property("height", &height);
}

/// The page's value of a colour token, so the canvas paints exactly what the CSS
/// would have.
#[cfg(target_arch = "wasm32")]
fn token_colour(token: &str) -> Result<String, ()> {
    let root = document().document_element().ok_or(())?;
    let styles = window()
        .get_computed_style(&root)
        .map_err(|_| ())?
        .ok_or(())?;
    let value = styles.get_property_value(token).map_err(|_| ())?;
    match value.trim() {
        "" => Err(()),
        colour => Ok(colour.to_owned()),
    }
}

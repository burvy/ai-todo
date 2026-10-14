//! The ink surface: the canvas the trail is painted on.
//!
//! The rules live in [`crate::ink`] and are pure — this is the thin part that
//! reads them off and puts ink on the page. It sits above the page and below
//! Rambo (DESIGN-DIRECTION: the trail is a layer above the page, below Rambo,
//! and never over the capture line or a dialog), and it never takes a pointer
//! itself: Rambo is the thing you pull, and the trail is what follows him.

use crate::ink::{Dot, Path};
use crate::settings::Pen;
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

/// One dab, with the ink it is painted in: a CSS token, so the trail follows the
/// page's palette in light and dark without anything here knowing a colour.
///
/// DESIGN-DIRECTION §6: the fountain pen and the marker write in `--ink`, the
/// ballpoint in `--ink-ballpoint`, the quill in `--ink-quill`, and the tail of a
/// stroke in `--rambo-tint` whatever the tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub alpha: f64,
    pub token: &'static str,
}

/// The token a dab is painted in.
pub fn token_for(pen: Pen, tail: bool) -> &'static str {
    if tail {
        return "--rambo-tint";
    }
    match pen {
        Pen::FountainPen | Pen::Pencil | Pen::Marker => "--ink",
        Pen::Ballpoint => "--ink-ballpoint",
        Pen::Quill => "--ink-quill",
    }
}

/// The whole trail as paint: every dab with the ink it belongs in.
pub fn paint_plan(dots: &[Dot], pen: Pen) -> Vec<Paint> {
    dots.iter()
        .map(|dot| Paint {
            x: dot.x,
            y: dot.y,
            radius: dot.radius,
            alpha: dot.alpha,
            token: token_for(pen, dot.tail),
        })
        .collect()
}

/// The trail's dabs, filled in from the paths a dazed Rambo was dragged along.
#[component]
pub fn InkLayer(dots: Signal<Vec<Dot>>, pen: Signal<Pen>) -> impl IntoView {
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();

    // Repaint whenever the trail or the tool changes. The canvas is cleared and
    // the whole trail laid down again: a stroke is a few hundred dabs, and a
    // partial append would have to know which dabs are already on the paper.
    Effect::new(move |_| {
        let Some(context) = ink_context(&canvas_ref) else {
            return;
        };
        fresh_sheet(&canvas_ref, &context);
        let plan = paint_plan(&dots.get(), pen.get());
        for dab in plan {
            let Ok(colour) = token_colour(dab.token) else {
                continue;
            };
            context.save();
            context.set_global_alpha(dab.alpha);
            context.set_fill_style_str(&colour.as_string().unwrap_or_default());
            context.begin_path();
            if context
                .arc(dab.x, dab.y, dab.radius, 0.0, std::f64::consts::TAU)
                .is_ok()
            {
                context.fill();
            }
            context.restore();
        }
    });

    view! {
        <canvas class="ink-layer" node_ref=canvas_ref aria-hidden="true" />
    }
}

#[cfg(target_arch = "wasm32")]
fn ink_context(
    canvas: &NodeRef<leptos::html::Canvas>,
) -> Option<web_sys::CanvasRenderingContext2d> {
    canvas
        .get()?
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
    let (Some(canvas), window) = (canvas.get(), leptos::prelude::window()) else {
        return;
    };
    let (Ok(width), Ok(height)) = (window.inner_width(), window.inner_height()) else {
        return;
    };
    let ratio = window.device_pixel_ratio();
    let px =
        |value: wasm_bindgen::JsValue| js_sys::Number::from(value).as_f64().unwrap_or_default();
    canvas.set_width((px(width) * ratio) as u32);
    canvas.set_height((px(height) * ratio) as u32);
    // Resizing to the same size keeps the old ink, so the sheet is wiped by
    // hand, in device pixels, before the scale to CSS pixels goes back on.
    let _ = context.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    context.clear_rect(
        0.0,
        0.0,
        f64::from(canvas.width()),
        f64::from(canvas.height()),
    );
    let _ = context.set_transform(ratio, 0.0, 0.0, ratio, 0.0, 0.0);
}

#[cfg(not(target_arch = "wasm32"))]
fn ink_context(
    _canvas: &NodeRef<leptos::html::Canvas>,
) -> Option<web_sys::CanvasRenderingContext2d> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn fresh_sheet(
    _canvas: &NodeRef<leptos::html::Canvas>,
    _context: &web_sys::CanvasRenderingContext2d,
) {
}

/// The page's value of a colour token, so the canvas paints exactly what the CSS
/// would have.
#[cfg(target_arch = "wasm32")]
fn token_colour(token: &str) -> Result<js_sys::JsString, ()> {
    let document = leptos::prelude::document();
    let root = document.document_element().ok_or(())?;
    let styles = leptos::prelude::window()
        .get_computed_style(&root)
        .map_err(|_| ())?
        .ok_or(())?;
    let value = styles.get_property_value(token).map_err(|_| ())?;
    if value.is_empty() {
        return Err(());
    }
    Ok(js_sys::JsString::from(value.trim()))
}

#[cfg(not(target_arch = "wasm32"))]
fn token_colour(_token: &str) -> Result<js_sys::JsString, ()> {
    Err(())
}

/// The trail a pointer walked, for the page to hand over in one go.
pub fn trail_of(path: &Path, pen: Pen) -> Vec<Dot> {
    crate::ink::rambo_trail(path, pen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot(tail: bool) -> Dot {
        Dot {
            x: 10.0,
            y: 20.0,
            radius: 3.0,
            alpha: 0.5,
            tail,
        }
    }

    #[test]
    fn every_tool_writes_in_its_own_ink() {
        assert_eq!(token_for(Pen::FountainPen, false), "--ink");
        assert_eq!(token_for(Pen::Pencil, false), "--ink");
        assert_eq!(token_for(Pen::Marker, false), "--ink");
        assert_eq!(token_for(Pen::Ballpoint, false), "--ink-ballpoint");
        assert_eq!(token_for(Pen::Quill, false), "--ink-quill");
    }

    #[test]
    fn the_tail_is_rambo_tint_whichever_tool_drew_it() {
        for pen in [
            Pen::FountainPen,
            Pen::Ballpoint,
            Pen::Pencil,
            Pen::Marker,
            Pen::Quill,
        ] {
            assert_eq!(token_for(pen, true), "--rambo-tint", "{pen:?}");
        }
    }

    #[test]
    fn the_plan_is_the_dots_with_their_ink_and_nothing_lost() {
        let plan = paint_plan(&[dot(false), dot(true)], Pen::Quill);
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].token, "--ink-quill");
        assert_eq!(plan[1].token, "--rambo-tint");
        assert!((plan[0].x - 10.0).abs() < f64::EPSILON);
        assert_eq!(plan[0].alpha, 0.5);
        assert!((plan[0].radius - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn an_empty_trail_paints_nothing() {
        assert!(paint_plan(&[], Pen::FountainPen).is_empty());
    }

    #[test]
    fn a_path_comes_out_as_the_tools_own_dabs() {
        let path: Path = (0..=20).map(|i| (i as f64 * 5.0, 40.0)).collect();
        let quill = trail_of(&path, Pen::Quill);
        let pencil = trail_of(&path, Pen::Pencil);
        assert!(!quill.is_empty());
        assert!(
            quill.len() < pencil.len(),
            "a quill lays its dabs further apart, so there are fewer of them"
        );
        assert!(quill.last().expect("a trail").tail);
        assert!(!pencil[0].tail);
    }
}

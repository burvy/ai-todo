//! The ink surface: the canvas the trail and the splats are painted on.
//!
//! The rules live in [`crate::ink`] and are pure — this is the thin part that
//! reads them off and puts ink on the page. It sits above the page and below
//! Rambo (DESIGN-DIRECTION: the trail is a layer above the page, below Rambo,
//! and never over the capture line or a dialog), and it never takes a pointer
//! itself: Rambo is the thing you pull, and the trail is what follows him.
//!
//! Ink ages on the page (`ink::age_alpha`), so while any is still drying or
//! fading the canvas is repainted every frame; once the last stroke has faded it
//! is dropped and the frames stop.

use crate::ink::{Dot, Stroke, Surface, age_alpha, dryness};
use crate::settings::Pen;
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

/// The token a dab is painted in: a CSS token, so the ink follows the page's
/// palette in light and dark without anything here knowing a colour.
///
/// DESIGN-DIRECTION §6: the fountain pen and the marker write in `--ink`, the
/// ballpoint in `--ink-ballpoint`, the quill in `--ink-quill`, and the tail of a
/// stroke in `--rambo-tint` whatever the tool.
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

/// A stroke as it lies on the page: its dabs, worked out once, and when it was laid.
#[derive(Clone, Debug, PartialEq)]
pub struct Laid {
    pub dots: Vec<Dot>,
    pub pen: Pen,
    pub at: f64,
    /// The nib's full width, against which a dab's thinness is measured: thin ink
    /// fades first.
    pub radius: f64,
    /// How uneven the tool's ink turns as it dries.
    pub dry_grain: f64,
}

impl Laid {
    pub fn of(stroke: &Stroke) -> Self {
        let nib = crate::ink::Nib::for_pen(stroke.pen);
        Laid {
            dots: stroke.dots(),
            pen: stroke.pen,
            at: stroke.laid_at(),
            radius: nib.radius,
            dry_grain: nib.dry_grain,
        }
    }
}

/// Dabs that share one ink and one strength, filled together in one path. A path is
/// filled once however many of its dabs overlap, so neighbouring dabs never bead
/// where they meet and the line reads as one continuous stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub token: &'static str,
    pub alpha: f64,
    pub dabs: Vec<(f64, f64, f64)>,
}

/// How many strengths of ink are told apart, so a frame is a few dozen fills and not
/// one per dab. Fine enough that no step shows.
const ALPHA_LEVELS: f64 = 48.0;

/// The page's ink at `now` (the frames' clock, in ms): every dab, aged, grouped by
/// ink and strength. Ink that has faded away is left out.
pub fn batches(laid: &[Laid], now: f64) -> Vec<Batch> {
    let mut out: Vec<Batch> = Vec::new();
    for stroke in laid {
        for dot in &stroke.dots {
            // Each dab ages from when the nib laid it, so a stroke fades the way it
            // was drawn; thin ink goes first, and dry ink pools a little unevenly.
            let age = now - dot.t;
            let thinness = 1.0 - dot.radius / stroke.radius.max(f64::EPSILON);
            let pooled = 1.0 + stroke.dry_grain * dot.pooling * dryness(age);
            let strength = dot.alpha * age_alpha(age, thinness) * pooled;
            let alpha = (strength.clamp(0.0, 1.0) * ALPHA_LEVELS).round() / ALPHA_LEVELS;
            if alpha <= 0.0 {
                continue;
            }
            let token = token_for(stroke.pen, dot.tail);
            let dab = (dot.x, dot.y, dot.radius);
            match out
                .iter_mut()
                .find(|b| b.token == token && b.alpha == alpha)
            {
                Some(batch) => batch.dabs.push(dab),
                None => out.push(Batch {
                    token,
                    alpha,
                    dabs: vec![dab],
                }),
            }
        }
    }
    // Faintest first: each batch is laid over the ones before it in place of them
    // (`paint`), so where two strengths meet the stronger shows, never their sum.
    out.sort_by(|a, b| a.alpha.total_cmp(&b.alpha));
    out
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ink::Sample;
    use crate::tuning::{INK_FADE_OUT_MS, INK_HOLD_MS};

    fn stroke(pen: Pen) -> Stroke {
        Stroke {
            samples: (0..=40)
                .map(|i| Sample {
                    x: f64::from(i) * 5.0,
                    y: 40.0,
                    t: f64::from(i) * 50.0,
                })
                .collect(),
            pen,
            on: crate::ink::Surface::Floor,
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
    fn a_stroke_of_one_ink_is_filled_in_a_few_paths_with_every_dab() {
        let laid = Laid::of(&stroke(Pen::FountainPen));
        let at = laid.at;
        let on_page = batches(std::slice::from_ref(&laid), at);
        let dabs: usize = on_page.iter().map(|b| b.dabs.len()).sum();
        assert_eq!(dabs, laid.dots.len(), "nothing lost");
        // Laid over two seconds, the stroke has aged along its length, yet it is
        // still a bounded number of fills, the freshest dab is full, and the tail is
        // Rambo's own.
        assert!(
            on_page.len() <= 2 * ALPHA_LEVELS as usize,
            "{} fills",
            on_page.len()
        );
        assert!(
            on_page
                .iter()
                .any(|b| b.token == "--rambo-tint" && b.alpha == 1.0)
        );
    }

    #[test]
    fn fresh_ink_is_full_and_faded_ink_paints_nothing() {
        let laid = Laid::of(&stroke(Pen::FountainPen));
        let fresh = batches(std::slice::from_ref(&laid), laid.at);
        assert!(fresh.iter().any(|b| b.alpha == 1.0));
        let later = batches(
            std::slice::from_ref(&laid),
            laid.at + INK_HOLD_MS + INK_FADE_OUT_MS,
        );
        assert!(later.is_empty(), "{later:?}");
    }

    #[test]
    fn a_frame_is_a_few_dozen_fills_even_mid_fade_in_graphite() {
        let laid = Laid::of(&stroke(Pen::Pencil));
        let mid_fade = laid.at + INK_HOLD_MS + INK_FADE_OUT_MS * 0.3;
        let on_page = batches(std::slice::from_ref(&laid), mid_fade);
        assert!(!on_page.is_empty());
        assert!(
            on_page.len() <= 2 * ALPHA_LEVELS as usize,
            "{} fills",
            on_page.len()
        );
    }

    #[test]
    fn the_faintest_ink_is_laid_first_so_the_strongest_wins() {
        let laid = Laid::of(&stroke(Pen::FountainPen));
        let now = laid.at + INK_HOLD_MS + INK_FADE_OUT_MS * 0.3;
        let order: Vec<f64> = batches(std::slice::from_ref(&laid), now)
            .iter()
            .map(|b| b.alpha)
            .collect();
        assert!(
            order.len() > 1 && order.windows(2).all(|w| w[0] <= w[1]),
            "{order:?}"
        );
    }

    #[test]
    fn a_stroke_fades_the_way_it_was_drawn() {
        // Laid over two seconds: mid-fade its start is fainter than its end.
        let laid = Laid::of(&stroke(Pen::Ballpoint));
        let first = laid.dots[laid.dots.len() / 8];
        let last = laid.dots[laid.dots.len() * 7 / 8];
        assert!(last.t - first.t > 1000.0);
        let now = first.t + INK_HOLD_MS + INK_FADE_OUT_MS * 0.5;
        let on_page = batches(std::slice::from_ref(&laid), now);
        let strength_at = |dot: &Dot| {
            on_page
                .iter()
                .find(|b| b.dabs.contains(&(dot.x, dot.y, dot.radius)))
                .map_or(0.0, |b| b.alpha)
        };
        assert!(
            strength_at(&first) < strength_at(&last),
            "the start goes first"
        );
        // Still a few dozen fills, not one per dab.
        assert!(
            on_page.len() <= 2 * ALPHA_LEVELS as usize,
            "{}",
            on_page.len()
        );
    }

    #[test]
    fn a_stroke_lies_in_the_ink_of_the_tool_that_drew_it() {
        let quill = Laid::of(&stroke(Pen::Quill));
        let on_page = batches(std::slice::from_ref(&quill), quill.at);
        assert!(
            on_page
                .iter()
                .all(|b| b.token == "--ink-quill" || b.token == "--rambo-tint")
        );
    }
}

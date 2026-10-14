//! Glyph-by-glyph writing, DOM side. The pure timing lives in `pen`; these
//! helpers measure the real rendered text so the nib traces what the user
//! actually sees. All wasm-only: no DOM on the test path.

use crate::{pen, sound};
use leptos::prelude::{RwSignal, Set};
use std::collections::HashMap;
use wasm_bindgen::JsCast;

/// The element's inline style, so a frame can move the ink without
/// rewriting the whole `style` attribute and losing what is already on
/// the element. `None` for anything that is not an HTML element, which is
/// never the case for the title and rows we animate.
pub(super) fn inline_style(el: &web_sys::Element) -> Option<web_sys::CssStyleDeclaration> {
    el.clone()
        .dyn_into::<web_sys::HtmlElement>()
        .ok()
        .map(|el| el.style())
}

/// Wait until the self-hosted handwriting fonts have loaded, so glyph
/// widths are measured in the writing font and not the fallback. The
/// caller awaits this before parking the nib, so a cold font cache does
/// not park the pen on fallback metrics and then jump.
pub async fn wait_for_fonts() {
    let Some(fonts) = web_sys::window()
        .and_then(|w| w.document())
        .map(|doc| doc.fonts())
    else {
        return;
    };
    if let Ok(ready) = fonts.ready() {
        let _ = wasm_bindgen_futures::JsFuture::from(ready).await;
    }
}

/// A canvas laid out with `el`'s computed font: the measuring stick for
/// glyph advance widths and the em box around the baseline.
fn measuring_canvas(el: &web_sys::Element) -> Option<web_sys::CanvasRenderingContext2d> {
    let style = web_sys::window()?.get_computed_style(el).ok().flatten()?;
    let property = |name: &str| style.get_property_value(name).ok();
    let font = format!(
        "{} {} {} {}",
        property("font-style")?,
        property("font-weight")?,
        property("font-size")?,
        property("font-family")?
    );
    let canvas = web_sys::window()?
        .document()?
        .create_element("canvas")
        .ok()?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .ok()?;
    let ctx = canvas
        .get_context("2d")
        .ok()?
        .and_then(|ctx| ctx.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())?;
    // The string is the computed font shorthand, so this parses; if it
    // somehow does not, the canvas keeps its default font and the write
    // still runs, just paced off fallback metrics.
    ctx.set_font(&font);
    Some(ctx)
}

/// The writing font measured once: how wide each glyph is on the page,
/// and how far the em box runs above and below the baseline.
struct Writing {
    width_of: Box<dyn Fn(char) -> f64>,
    ascent: f64,
    descent: f64,
}

fn measure(el: &web_sys::Element, text: &str) -> Option<Writing> {
    let ctx = measuring_canvas(el)?;
    // Every distinct glyph is measured once, so a long title does not
    // re-measure the same letter over and over.
    let mut widths: HashMap<char, f64> = HashMap::new();
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        widths.entry(c).or_insert_with(|| {
            ctx.measure_text(&c.to_string())
                .map(|m| m.width())
                .unwrap_or(6.0)
        });
    }
    let space = ctx.measure_text(" ").map(|m| m.width()).unwrap_or(4.0);
    let metrics = ctx.measure_text(text).ok()?;
    Some(Writing {
        width_of: Box::new(move |c: char| {
            if c.is_whitespace() {
                space
            } else {
                widths.get(&c).copied().unwrap_or(6.0)
            }
        }),
        ascent: metrics.font_bounding_box_ascent(),
        descent: metrics.font_bounding_box_descent(),
    })
}

/// Which tool Rambo is holding, from the gear menu's `data-pen` hook. Absent
/// until that menu lands, which is why the pen's ink falls back.
fn held_tool() -> Option<String> {
    web_sys::window()?
        .document()?
        .query_selector(".rambo")
        .ok()
        .flatten()?
        .get_attribute("data-pen")
}

/// Put the pen so its nib sits at `(x, y)`. `pen_pos` is absolute page
/// coordinates (Rambo is a fixed layer), so no offset conversion needed.
pub fn place_nib(pen_pos: RwSignal<(f64, f64)>, x: f64, y: f64) {
    let center = pen::writing_pen_center(x, y);
    pen_pos.set(center);
}

/// Park the nib at the left edge of `row`'s line, before the flight, so
/// the pen arrives exactly where it is about to start writing.
pub fn park_nib(row: &web_sys::Element, pen_pos: RwSignal<(f64, f64)>) {
    let Some(title) = row.query_selector(".pen-title").ok().flatten() else {
        return;
    };
    let Some(line) = one_line(&title) else {
        return;
    };
    place_nib(pen_pos, line.left, line.baseline);
}

/// The title's one line box, when it is only one: a long title wraps onto
/// the next ruled line and the pen has no single edge to trace, so those
/// are left to the CSS wipe.
fn one_line(title: &web_sys::Element) -> Option<Line> {
    let rects = title.get_client_rects();
    // Zero rects means nothing is laid out yet (a title still hidden from
    // the flight); two or more means the title wrapped.
    if rects.length() != 1 {
        return None;
    }
    let rect = rects.get(0)?;
    let writing = measure(title, &title.text_content().unwrap_or_default())?;
    Some(Line {
        left: rect.left(),
        width: rect.width(),
        baseline: pen::baseline_y(rect.top(), rect.height(), writing.ascent, writing.descent),
    })
}

/// One ruled line of a title: where it starts, how wide it is, and the
/// baseline the nib rides on.
struct Line {
    left: f64,
    width: f64,
    baseline: f64,
}

/// Write `row`'s title under the nib.
///
/// The whole title is laid out from the start and only the ink is
/// clipped, so the line never reflows under the pen. Each frame moves the
/// clip and the nib together — the nib is always exactly at the edge of
/// the ink — and the pen scratches for as long as it is moving. A title
/// too long for one ruled line keeps the CSS wipe: the sound still runs
/// for as long as the wipe takes, but there is no single edge to follow.
///
/// Takes the row from `pen-pending` to written, so the caller only marks
/// the row pending, awaits the fonts and parks the nib.
pub async fn write_row_title(row: &web_sys::Element, pen_pos: RwSignal<(f64, f64)>) {
    let unhide = || {
        row.class_list().remove_1("pen-pending").ok();
        row.class_list().remove_1("pen-writing").ok();
    };
    let Some(title) = row.query_selector(".pen-title").ok().flatten() else {
        unhide();
        return;
    };
    let text = title.text_content().unwrap_or_default();
    if text.trim().is_empty() {
        unhide();
        return;
    }
    // The tool's ink goes on before anything is measured. The marker is
    // bolder than the base weight, so a title measured at 500 and drawn
    // at 700 is paced off the wrong widths - and that is the one promise
    // the whole schedule exists to keep.
    let ink = pen::ink_for_pen(held_tool().as_deref());
    row.class_list().add_1(ink).ok();
    row.class_list().add_1("pen-writing").ok();
    // A traced title starts with no ink at all, so the finished title is
    // never on screen before the pen begins writing it. Reaching here is
    // one task: the row leaves `pen-pending`'s full clip and gains an
    // empty one of its own before the browser paints either.
    let traced = one_line(&title);
    let style = inline_style(&title);
    if let (Some(style), Some(traced)) = (&style, &traced) {
        style
            .set_property("clip-path", &format!("inset(0 {:.1}px 0 0)", traced.width))
            .ok();
    }
    row.class_list().remove_1("pen-pending").ok();

    let scratch = sound::looping(sound::Cue::Write);
    if let (Some(style), Some(writing)) = (&style, measure(&title, &text)) {
        let line = pen::schedule(&text, |c| (writing.width_of)(c));
        let total = line.last().map(|glyph| glyph.ends_ms).unwrap_or(0);
        match traced {
            // Wrapped: theme.css is already transitioning the clip, so all
            // that is left is to scratch for as long as it will take.
            None => gloo_timers::future::TimeoutFuture::new(total).await,
            Some(traced) => {
                let mut elapsed = 0;
                loop {
                    let drawn = f64::from(pen::inked(&line, elapsed)) * traced.width;
                    style
                        .set_property(
                            "clip-path",
                            &format!("inset(0 {:.1}px 0 0)", traced.width - drawn),
                        )
                        .ok();
                    place_nib(pen_pos, traced.left + drawn, traced.baseline);
                    if elapsed >= total {
                        break;
                    }
                    gloo_timers::future::TimeoutFuture::new(pen::INK_TICK_MS).await;
                    elapsed += pen::INK_TICK_MS;
                }
            }
        }
    }
    if let Some(scratch) = scratch {
        scratch.stop();
    }
    // The clip has done its job and the row is plain text again, so the
    // title keeps the ink it was written in only while Rambo is writing.
    if let Some(style) = &style {
        style.remove_property("clip-path").ok();
    }
    row.class_list().remove_1(ink).ok();
    row.class_list().remove_1("pen-writing").ok();
}

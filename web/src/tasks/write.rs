//! Glyph-by-glyph writing, DOM side. The pure timing lives in `pen`; these
//! helpers measure the real rendered text so the nib traces what the user
//! actually sees. All wasm-only: no DOM on the test path.

use crate::pen::{Beat, LineBox};
use crate::tuning::{WRITE_FINISH_MS, WRITE_SETTLE_MS};
use crate::{mascot, pen, sound};
use leptos::prelude::{GetUntracked, ReadSignal, RwSignal, Set, WriteSignal};
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

/// Whether `row` shows on screen, so the pen can fly to it.
pub fn in_view(row: &web_sys::Element) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let size = |dim: Result<wasm_bindgen::JsValue, _>| dim.ok().and_then(|v| v.as_f64());
    let (Some(width), Some(height)) = (size(window.inner_width()), size(window.inner_height()))
    else {
        return false;
    };
    let rect = row.get_bounding_client_rect();
    pen::on_screen(
        (rect.left(), rect.top(), rect.right(), rect.bottom()),
        (width, height),
    )
}

/// Park the nib at the start of `row`'s title, before the flight, so the pen
/// arrives exactly where it is about to start writing.
pub fn park_nib(row: &web_sys::Element, pen_pos: RwSignal<(f64, f64)>) {
    let Some(title) = row.query_selector(".pen-title").ok().flatten() else {
        return;
    };
    let Some(writing) = measure(&title, &title.text_content().unwrap_or_default()) else {
        return;
    };
    if let Some(first) = lines_of(&title).first() {
        place_nib(pen_pos, first.left, baseline(first, &writing));
    }
}

/// The title's line boxes as laid out now, in reading order. Measured again on
/// every frame: the page may scroll under the pen while he writes.
fn lines_of(title: &web_sys::Element) -> Vec<LineBox> {
    let rects = title.get_client_rects();
    (0..rects.length())
        .filter_map(|index| rects.get(index))
        .map(|rect| LineBox {
            left: rect.left(),
            top: rect.top(),
            width: rect.width(),
            height: rect.height(),
        })
        .collect()
}

fn baseline(line: &LineBox, writing: &Writing) -> f64 {
    pen::baseline_y(line.top, line.height, writing.ascent, writing.descent)
}

/// Fly him from where he is and land him, `event` taking him off and on again.
async fn fly(
    state: ReadSignal<mascot::State>,
    set_state: WriteSignal<mascot::State>,
    event: mascot::Event,
) {
    set_state.set(mascot::transition(&state.get_untracked(), event.clone()));
    gloo_timers::future::TimeoutFuture::new(crate::tuning::FLY_MS).await;
    set_state.set(mascot::transition(&state.get_untracked(), event));
}

/// Write `row`'s title under the nib.
///
/// The whole title is laid out from the start and only the ink is clipped, so
/// it never reflows under the pen. Each frame moves the clip and the nib
/// together, line after line: the nib is always at the edge of the ink, on the
/// baseline of the line it is writing, and the pen scratches while it moves.
///
/// Interrupted (picked up, called to the caret, the row scrolled away), the ink
/// waits at the letter he reached; once he is home and can see the row he flies
/// back and carries on ([`pen::beat`]). A row that is gone, or has waited
/// [`WRITE_HOLD_MAX_MS`](crate::tuning::WRITE_HOLD_MAX_MS), is shown whole.
///
/// Takes the row from `pen-pending` to written, so the caller only marks the
/// row pending, awaits the fonts and parks the nib.
pub async fn write_row_title(
    row: &web_sys::Element,
    pen_pos: RwSignal<(f64, f64)>,
    state: ReadSignal<mascot::State>,
    set_state: WriteSignal<mascot::State>,
) {
    let unhide = || {
        row.class_list().remove_1("pen-pending").ok();
        row.class_list().remove_1("pen-writing").ok();
    };
    let Some(title) = row.query_selector(".pen-title").ok().flatten() else {
        unhide();
        return;
    };
    let text = title.text_content().unwrap_or_default();
    let (Some(style), Some(writing)) = (inline_style(&title), measure(&title, &text)) else {
        unhide();
        return;
    };
    if text.trim().is_empty() || lines_of(&title).is_empty() {
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
    // The title starts with no ink at all, so the finished title is never on
    // screen before the pen begins writing it: the row leaves `pen-pending`'s
    // full clip and gains an empty one of its own before the browser paints.
    let glyphs = pen::schedule(&text, |c| (writing.width_of)(c));
    let total_ms = glyphs.last().map(|glyph| glyph.ends_ms).unwrap_or(0);
    // Ink the title up to `elapsed` into the schedule, and put the nib at the
    // edge of that ink.
    let ink_to = |lines: &[LineBox], elapsed: u32| {
        let width: f64 = lines.iter().map(|line| line.width).sum();
        let (line, x) = pen::ink_edge(lines, f64::from(pen::inked(&glyphs, elapsed)) * width);
        style
            .set_property("clip-path", &pen::ink_clip(lines, line, x))
            .ok();
        if let Some(at) = lines.get(line) {
            place_nib(pen_pos, at.left + x, baseline(at, &writing));
        }
    };
    ink_to(&lines_of(&title), 0);
    row.class_list().remove_1("pen-pending").ok();

    // The nib touches down and rests a beat before the first stroke.
    gloo_timers::future::TimeoutFuture::new(WRITE_SETTLE_MS).await;
    let (mut elapsed, mut held) = (0, 0);
    let mut scratch: Option<sound::Loop> = None;
    loop {
        let lines = lines_of(&title);
        let on_page = row.is_connected() && !lines.is_empty();
        let on_screen = on_page && in_view(row);
        let beat = pen::beat(&state.get_untracked(), on_screen, on_page, held);
        // He scratches only while the nib moves on the paper.
        if beat != Beat::Write
            && let Some(sound) = scratch.take()
        {
            sound.stop();
        }
        match beat {
            Beat::Write => {
                held = 0;
                scratch = scratch.or_else(|| sound::looping(sound::Cue::Write));
                ink_to(&lines, elapsed);
                if elapsed >= total_ms {
                    if let Some(sound) = scratch.take() {
                        sound.stop();
                    }
                    // The last letter closes; he holds a beat before leaving.
                    gloo_timers::future::TimeoutFuture::new(WRITE_FINISH_MS).await;
                    break;
                }
                elapsed += crate::tuning::INK_TICK_MS;
            }
            Beat::Hold => held += crate::tuning::INK_TICK_MS,
            Beat::FlyHome => {
                held += crate::tuning::FLY_MS;
                fly(state, set_state, mascot::Event::FlyHome).await;
            }
            Beat::FlyBack => {
                held += crate::tuning::FLY_MS;
                set_state.set(mascot::transition(
                    &state.get_untracked(),
                    mascot::Event::WriteRow,
                ));
                // He lands on the letter he stopped at.
                ink_to(&lines, elapsed);
                gloo_timers::future::TimeoutFuture::new(crate::tuning::FLY_MS).await;
                set_state.set(mascot::transition(
                    &state.get_untracked(),
                    mascot::Event::WriteRow,
                ));
            }
            Beat::Finish => break,
        }
        gloo_timers::future::TimeoutFuture::new(crate::tuning::INK_TICK_MS).await;
    }
    if let Some(sound) = scratch {
        sound.stop();
    }
    // The clip has done its job and the row is plain text again, so the
    // title keeps the ink it was written in only while Rambo is writing.
    style.remove_property("clip-path").ok();
    row.class_list().remove_1(ink).ok();
    row.class_list().remove_1("pen-writing").ok();
}

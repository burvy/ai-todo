//! Glyph-by-glyph writing, DOM side. The pure timing lives in `pen`; these
//! helpers measure the real rendered text so the nib traces what the user
//! actually sees. All wasm-only: no DOM on the test path.

use crate::pen::{Beat, Hand, LineBox, Stroke};
use crate::tuning::{INK_FADE_MS, INK_TICK_MS, WRITE_FINISH_MS, WRITE_FLICK_MS, WRITE_SETTLE_MS};
use crate::{mascot, pen, sound};
use leptos::prelude::{GetUntracked, ReadSignal, RwSignal, Set, WriteSignal};
use std::cell::Cell;
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

/// Where the nib stands to write `text` on a line of `el`'s font whose box is `top` and
/// `height` on screen: how far across the text runs (px) and the baseline's height.
pub(crate) fn text_on_line(el: &web_sys::Element, text: &str, top: f64, height: f64) -> (f64, f64) {
    measuring_canvas(el)
        .and_then(|ctx| ctx.measure_text(text).ok())
        .map_or((0.0, top + height / 2.0), |metrics| {
            (
                metrics.width(),
                pen::baseline_y(
                    top,
                    height,
                    metrics.font_bounding_box_ascent(),
                    metrics.font_bounding_box_descent(),
                ),
            )
        })
}

/// The writing font measured once: how wide each glyph is on the page,
/// and how far the em box runs above and below the baseline.
struct Writing {
    width_of: Box<dyn Fn(&str) -> f64>,
    ascent: f64,
    descent: f64,
}

fn measure(el: &web_sys::Element, text: &str) -> Option<Writing> {
    let ctx = measuring_canvas(el)?;
    // Every distinct glyph is measured once, so a long title does not
    // re-measure the same letter over and over.
    let mut widths: HashMap<String, f64> = HashMap::new();
    for glyph in pen::glyphs_of(text).filter(|glyph| !pen::is_gap(glyph)) {
        widths
            .entry(glyph.to_owned())
            .or_insert_with(|| ctx.measure_text(glyph).map(|m| m.width()).unwrap_or(6.0));
    }
    let space = ctx.measure_text(" ").map(|m| m.width()).unwrap_or(4.0);
    let metrics = ctx.measure_text(text).ok()?;
    Some(Writing {
        width_of: Box::new(move |glyph: &str| {
            if pen::is_gap(glyph) {
                space
            } else {
                widths.get(glyph).copied().unwrap_or(6.0)
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
    // His box as laid out, before his lean and scale: its size is the theme's, and
    // differs by screen.
    let box_h = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|doc| doc.query_selector(".rambo").ok().flatten())
        .and_then(|rambo| rambo.dyn_into::<web_sys::HtmlElement>().ok())
        .map_or(0.0, |rambo| f64::from(rambo.offset_height()));
    pen_pos.set(pen::row_nib_center(x, y, box_h));
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

/// Keep the nib parked on `row`'s title on every engine frame until the returned
/// handle is stopped. A new row is still moving into place while he flies to it
/// (arrivals move), and his hops read their far end every frame, so parking once
/// before the flight sent him to where the row had been: he overshot, or waited
/// there, and then snapped onto the row when he began to write.
pub fn follow_row(row: web_sys::Element, pen_pos: RwSignal<(f64, f64)>) -> crate::engine::Handle {
    crate::engine::on_tick(move |_| {
        park_nib(&row, pen_pos);
        crate::engine::Flow::Go
    })
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
    crate::engine::wait(f64::from(crate::tuning::FLY_MS)).await;
    set_state.set(mascot::transition(&state.get_untracked(), event));
}

/// The title spelled out one span a glyph while Rambo writes it, so each glyph's
/// ink can appear on its own. The text the view rendered is set aside, not
/// replaced, and is put back as it was when this is dropped: the view keeps
/// owning it, and whatever ends the writing leaves the title plain text. A screen
/// reader hears the title whole, not letter by letter, while it is spelled out.
struct Spelled {
    title: web_sys::Element,
    kept: Vec<web_sys::Node>,
    glyphs: Vec<web_sys::Element>,
    /// How many glyphs are inked so far: each frame touches only the new ones.
    inked: Cell<usize>,
}

impl Spelled {
    fn new(title: &web_sys::Element, text: &str) -> Option<Self> {
        let doc = title.owner_document()?;
        // Built before anything moves, so a failure part way still puts back
        // whatever was set aside.
        let mut spelled = Spelled {
            title: title.clone(),
            kept: Vec::new(),
            glyphs: Vec::new(),
            inked: Cell::new(0),
        };
        title.set_attribute("aria-label", text).ok();
        while let Some(node) = title.first_child() {
            title.remove_child(&node).ok()?;
            spelled.kept.push(node);
        }
        for text in pen::glyphs_of(text) {
            let glyph = doc.create_element("span").ok()?;
            glyph.set_class_name("ink-glyph");
            glyph.set_attribute("aria-hidden", "true").ok();
            glyph.set_text_content(Some(text));
            title.append_child(&glyph).ok()?;
            spelled.glyphs.push(glyph);
        }
        Some(spelled)
    }

    /// Show the ink of the first `count` glyphs.
    fn ink(&self, count: usize) {
        let count = count.min(self.glyphs.len());
        for glyph in self.glyphs.iter().take(count).skip(self.inked.get()) {
            glyph.class_list().add_1("inked").ok();
        }
        self.inked.set(self.inked.get().max(count));
    }

    /// Whether the title is laid out on the page: a title hidden with its row has
    /// no box.
    fn laid_out(&self) -> bool {
        self.glyph_box(0).is_some_and(|glyph| glyph.height > 0.0)
    }

    /// Where glyph `index` sits on the page now.
    fn glyph_box(&self, index: usize) -> Option<LineBox> {
        let rect = self.glyphs.get(index)?.get_bounding_client_rect();
        Some(LineBox {
            left: rect.left(),
            top: rect.top(),
            width: rect.width(),
            height: rect.height(),
        })
    }
}

impl Drop for Spelled {
    fn drop(&mut self) {
        for glyph in &self.glyphs {
            glyph.remove();
        }
        for node in &self.kept {
            self.title.append_child(node).ok();
        }
        self.title.remove_attribute("aria-label").ok();
    }
}

/// Set his hand: `--nib-dip` and `--nib-tilt` on him, which only the writing rule
/// reads.
fn set_hand(rambo: Option<&web_sys::CssStyleDeclaration>, hand: Hand) {
    if let Some(style) = rambo {
        style
            .set_property("--nib-dip", &format!("{:.2}px", hand.dip_px))
            .ok();
        style
            .set_property("--nib-tilt", &format!("{:.2}deg", hand.tilt_deg))
            .ok();
    }
}

/// Land: the nib touches down and the dip settles, then he rests a beat before
/// the first stroke.
async fn touch_down(rambo: Option<&web_sys::CssStyleDeclaration>) {
    let mut ms = 0;
    while ms < WRITE_SETTLE_MS {
        set_hand(rambo, pen::touch_down(ms));
        crate::engine::wait(f64::from(INK_TICK_MS)).await;
        ms += INK_TICK_MS;
    }
    set_hand(rambo, Hand::REST);
}

/// Write `row`'s title under the nib.
///
/// The title is laid out whole from the start, one span a glyph, and only the
/// ink appears: a glyph's ink fades in as the nib passes its middle, so it never
/// reflows under the pen and wrapped lines need nothing of their own. Each frame
/// the nib moves through the glyph it is stroking, on that glyph's baseline, his
/// hand pressing at each glyph and lifting over each gap, and the pen scratches
/// while it moves. He touches down when he lands and flicks off as the last
/// letter closes.
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
    let (Some(style), Some(writing)) = (inline_style(row), measure(&title, &text)) else {
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
    // Every glyph starts with no ink, so the finished title is never on screen
    // before the pen begins writing it: the row leaves `pen-pending`'s clip only
    // once its glyphs are in.
    let Some(spelled) = Spelled::new(&title, &text) else {
        row.class_list().remove_1(ink).ok();
        unhide();
        return;
    };
    row.class_list().add_1("pen-writing").ok();
    style
        .set_property("--ink-fade-ms", &format!("{INK_FADE_MS}ms"))
        .ok();
    row.class_list().remove_1("pen-pending").ok();
    let gaps: Vec<bool> = pen::glyphs_of(&text).map(pen::is_gap).collect();
    let glyphs = pen::schedule(&text, |glyph| (writing.width_of)(glyph));
    let total_ms = glyphs.last().map(|glyph| glyph.ends_ms).unwrap_or(0);
    let rambo = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|doc| doc.query_selector(".rambo").ok().flatten())
        .and_then(|rambo| inline_style(&rambo));
    let rambo = rambo.as_ref();
    // The nib through the glyph it is stroking, on that glyph's baseline.
    let nib_to = |at: Stroke| {
        if let Some(glyph) = spelled.glyph_box(at.index) {
            place_nib(
                pen_pos,
                glyph.left + glyph.width * at.progress,
                baseline(&glyph, &writing),
            );
        }
    };
    if let Some(start) = pen::stroke_at(&glyphs, 0) {
        nib_to(start);
    }

    touch_down(rambo).await;
    let (mut elapsed, mut held) = (0, 0);
    let mut scratch: Option<sound::Loop> = None;
    let mut hand = Hand::REST;
    loop {
        let on_page = row.is_connected() && spelled.laid_out();
        let on_screen = on_page && in_view(row);
        let beat = pen::beat(&state.get_untracked(), on_screen, on_page, held);
        // He scratches, and his hand moves, only while the nib is on the paper.
        if beat != Beat::Write && hand != Hand::REST {
            hand = Hand::REST;
            set_hand(rambo, hand);
        }
        if beat != Beat::Write
            && let Some(sound) = scratch.take()
        {
            sound.stop();
        }
        match beat {
            Beat::Write => {
                held = 0;
                scratch = scratch.or_else(|| sound::looping(sound::Cue::Write));
                spelled.ink(pen::inked_count(&glyphs, elapsed));
                if let Some(at) = pen::stroke_at(&glyphs, elapsed) {
                    nib_to(at);
                    let gap = gaps.get(at.index).copied().unwrap_or(false);
                    hand = pen::hand(at, gap);
                    set_hand(rambo, hand);
                }
                if elapsed >= total_ms {
                    if let Some(sound) = scratch.take() {
                        sound.stop();
                    }
                    // The last letter closes with a flick off the paper. The
                    // row's meta inks in while he holds a beat before leaving.
                    let mut ms = 0;
                    while ms < WRITE_FLICK_MS {
                        set_hand(rambo, pen::flick(ms, hand));
                        crate::engine::wait(f64::from(INK_TICK_MS)).await;
                        ms += INK_TICK_MS;
                    }
                    set_hand(rambo, Hand::REST);
                    row.class_list().add_1("meta-inked").ok();
                    crate::engine::wait(f64::from(WRITE_FINISH_MS)).await;
                    break;
                }
                elapsed += INK_TICK_MS;
            }
            Beat::Hold => held += INK_TICK_MS,
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
                if let Some(at) = pen::stroke_at(&glyphs, elapsed) {
                    nib_to(at);
                }
                crate::engine::wait(f64::from(crate::tuning::FLY_MS)).await;
                set_state.set(mascot::transition(
                    &state.get_untracked(),
                    mascot::Event::WriteRow,
                ));
                touch_down(rambo).await;
            }
            Beat::Finish => break,
        }
        crate::engine::wait(f64::from(INK_TICK_MS)).await;
    }
    if let Some(sound) = scratch {
        sound.stop();
    }
    set_hand(rambo, Hand::REST);
    // The title goes back to the text the view rendered, and keeps the ink it
    // was written in only while Rambo is writing.
    drop(spelled);
    style.remove_property("--ink-fade-ms").ok();
    row.class_list().remove_1(ink).ok();
    row.class_list().remove_1("pen-writing").ok();
    row.class_list().remove_1("meta-inked").ok();
}

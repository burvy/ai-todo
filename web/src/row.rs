//! The DOM side of a press on a task row; the decisions it feeds are `rambl_web_core::row`.

pub use rambl_web_core::row::*;

/// The DOM side: the numbers `advance` wants, taken off real pointer events,
/// and the classes and transforms they decide.
///
/// Not gated on the target: `web-sys` compiles for the host too, and the row's
/// `view!` names these in both builds. On the host they are never called.
pub mod dom {
    use super::{
        Gesture, HOLD_MS, Release, Slot, Words, advance, ink_drawn, lands_above, lift_clears,
        release, scribble_path,
    };
    use crate::tuning::{SCRIBBLE_FINISH_MS, SCRIBBLE_LIFT_MS};
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;

    /// What the press on this row has become, for the handlers that must not
    /// steal a gesture that is not theirs: a move or a touch only claims the
    /// pointer once the row is lifted.
    pub fn gesture(presses: &Presses) -> Option<Gesture> {
        presses.borrow().as_ref().map(|press| press.gesture)
    }

    /// A pointer's whole time on one row. `gesture` is the only thing that
    /// decides what the row does, and it is decided by [`advance`] alone.
    pub struct Press {
        pointer_id: i32,
        row: web_sys::Element,
        x: f64,
        y: f64,
        started_ms: f64,
        gesture: Gesture,
        /// The row's width when the press began: how far the scribble can go.
        width: f64,
        /// Whether holding it lifts it: not while a dazed Rambo takes every hold.
        lifts: bool,
    }

    impl Press {
        fn held_ms(&self) -> u32 {
            (crate::engine::now() - self.started_ms).max(0.0) as u32
        }
        /// How far the pointer has travelled from where it went down.
        fn travel(&self, ev: &leptos::ev::PointerEvent) -> (f64, f64) {
            (
                f64::from(ev.client_x()) - self.x,
                f64::from(ev.client_y()) - self.y,
            )
        }
    }

    /// The shared press state for one row: at most one pointer at a time, so a
    /// second finger on a row the first one is holding is ignored rather than
    /// fighting it.
    pub type Presses = Rc<RefCell<Option<Press>>>;

    fn style_of(el: &web_sys::Element) -> Option<web_sys::CssStyleDeclaration> {
        el.clone()
            .dyn_into::<web_sys::HtmlElement>()
            .ok()
            .map(|el| el.style())
    }

    fn move_row(row: &web_sys::Element, dx: f64, dy: f64) {
        if let Some(style) = style_of(row) {
            style
                .set_property("transform", &format!("translate({dx:.1}px, {dy:.1}px)"))
                .ok();
        }
    }

    /// Put the row back exactly as the list draws it: no transform, no gesture
    /// class. Called on every way a press can end, so a row can never be left
    /// stuck halfway across the screen.
    fn settle(row: &web_sys::Element) {
        if let Some(style) = style_of(row) {
            style.remove_property("transform").ok();
        }
        row.class_list().remove_1("row-lifted").ok();
        row.class_list().remove_1("row-swiping").ok();
    }

    const SVG: &str = "http://www.w3.org/2000/svg";
    /// The ink's easing: a strong ease-out, so it answers the instant you let go.
    const INK_EASE: &str = "cubic-bezier(0.23, 1, 0.32, 1)";

    /// The scribble's path on `row`, laid across it the first time it is asked for.
    /// The number of the stroke on `row`, if it has one.
    fn stroke_on(row: &web_sys::Element) -> Option<u32> {
        row.query_selector(".scribble-ink")
            .ok()
            .flatten()?
            .get_attribute("data-stroke")?
            .parse()
            .ok()
    }

    fn ink_path(row: &web_sys::Element, width: f64) -> Option<web_sys::Element> {
        static NEXT_STROKE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if let Some(path) = row
            .query_selector(".scribble-ink:not(.lifting) path")
            .ok()
            .flatten()
        {
            return Some(path);
        }
        // A stroke still lifting away from the last swipe gives way to the new one.
        remove_ink(row);
        let doc = row.owner_document()?;
        let height = row.get_bounding_client_rect().height();
        let seed = row.get_attribute("data-id").unwrap_or_default();
        let svg = doc.create_element_ns(Some(SVG), "svg").ok()?;
        svg.set_attribute("class", "scribble-ink").ok()?;
        let stroke = NEXT_STROKE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        svg.set_attribute("data-stroke", &stroke.to_string()).ok()?;
        svg.set_attribute("viewBox", &format!("0 0 {width:.1} {height:.1}"))
            .ok()?;
        svg.set_attribute("preserveAspectRatio", "none").ok()?;
        svg.set_attribute("aria-hidden", "true").ok()?;
        let path = doc.create_element_ns(Some(SVG), "path").ok()?;
        path.set_attribute("d", &scribble_path(words_in(row), &seed))
            .ok()?;
        // A path one unit long, so the dash offset is the share left undrawn.
        path.set_attribute("pathLength", "1").ok()?;
        svg.append_child(&path).ok()?;
        row.append_child(&svg).ok()?;
        Some(path)
    }

    /// Where `row`'s title sits inside it: its first line, start to end. A row
    /// with no title is crossed out whole.
    fn words_in(row: &web_sys::Element) -> Words {
        let frame = row.get_bounding_client_rect();
        let whole = Words {
            left: 0.0,
            right: frame.width(),
            top: 0.0,
            bottom: frame.height(),
        };
        let Some(title) = row.query_selector(".pen-title").ok().flatten() else {
            return whole;
        };
        let Some(first) = title.get_client_rects().get(0) else {
            return whole;
        };
        let text = title.get_bounding_client_rect();
        Words {
            left: text.left() - frame.left(),
            right: text.right() - frame.left(),
            top: first.top() - frame.top(),
            bottom: first.bottom() - frame.top(),
        }
    }

    /// Show the scribble drawn `drawn` of the way across, under the finger.
    fn draw_ink(row: &web_sys::Element, width: f64, drawn: f64) {
        if let Some(path) = ink_path(row, width) {
            path.set_attribute(
                "style",
                &format!("stroke-dasharray: 1; stroke-dashoffset: {:.3}", 1.0 - drawn),
            )
            .ok();
        }
    }

    /// Let go of a swipe: finish the scribble across the row, or lift the ink away.
    fn end_ink(row: &web_sys::Element, commit: bool) {
        let Some(path) = row.query_selector(".scribble-ink path").ok().flatten() else {
            return;
        };
        let ms = match (crate::mascot::prefers_reduced_motion(), commit) {
            (true, _) => 0,
            (false, true) => SCRIBBLE_FINISH_MS,
            (false, false) => SCRIBBLE_LIFT_MS,
        };
        let offset = if commit { 0 } else { 1 };
        path.set_attribute(
            "style",
            &format!(
                "stroke-dasharray: 1; stroke-dashoffset: {offset}; \
                 transition: stroke-dashoffset {ms}ms {INK_EASE}"
            ),
        )
        .ok();
        if !commit && let Some(lifted) = stroke_on(row) {
            if let Some(svg) = row.query_selector(".scribble-ink").ok().flatten() {
                svg.class_list().add_1("lifting").ok();
            }
            let row = row.clone();
            crate::engine::after(f64::from(ms), move || {
                if lift_clears(lifted, stroke_on(&row)) {
                    remove_ink(&row);
                }
            });
        }
    }

    fn remove_ink(row: &web_sys::Element) {
        if let Some(svg) = row.query_selector(".scribble-ink").ok().flatten() {
            svg.remove();
        }
    }

    /// Take the ink off the row `row_id`: its delete did not go through.
    pub fn clear_ink(row_id: &str) {
        let row = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|doc| {
                doc.query_selector(&format!("li[data-id=\"{row_id}\"]"))
                    .ok()
                    .flatten()
            });
        if let Some(row) = row {
            remove_ink(&row);
        }
    }

    /// The row a press landed on, unless it landed on one of the row's own
    /// controls - the box, the title's button, the x - which keep their own
    /// clicks and must not be turned into a drag.
    fn row_under(ev: &leptos::ev::PointerEvent) -> Option<web_sys::Element> {
        let target = ev.target()?.dyn_into::<web_sys::Element>().ok()?;
        if target
            .closest("button, input, select, textarea, a")
            .ok()
            .flatten()
            .is_some()
        {
            return None;
        }
        target.closest("li[data-id]").ok().flatten()
    }

    /// Start following a pointer on a row. Returns whether this press is ours,
    /// so the caller knows not to start anything else on top of it.
    /// With `lifts` false, holding the press never lifts the row: that hold is
    /// someone else's.
    pub fn down(presses: &Presses, ev: &leptos::ev::PointerEvent, lifts: bool) -> bool {
        if presses.borrow().is_some() {
            return false;
        }
        let Some(row) = row_under(ev) else {
            return false;
        };
        let _ = row.set_pointer_capture(ev.pointer_id());
        *presses.borrow_mut() = Some(Press {
            pointer_id: ev.pointer_id(),
            row: row.clone(),
            x: f64::from(ev.client_x()),
            y: f64::from(ev.client_y()),
            started_ms: crate::engine::now(),
            gesture: Gesture::Pressed,
            width: row.get_bounding_client_rect().width(),
            lifts,
        });
        // A finger that never moves still picks the row up when the hold runs
        // out, so the lift cannot depend on a pointermove arriving.
        let presses = presses.clone();
        crate::engine::after(f64::from(HOLD_MS), move || {
            let lifted = match presses.borrow_mut().as_mut() {
                Some(p) if p.gesture == Gesture::Pressed && p.lifts => {
                    p.gesture = Gesture::Lifted;
                    Some(p.row.clone())
                }
                _ => None,
            };
            if let Some(row) = lifted {
                row.class_list().add_1("row-lifted").ok();
            }
        });
        true
    }

    /// Follow the pointer, deciding what the gesture has become as it goes.
    pub fn drag(presses: &Presses, ev: &leptos::ev::PointerEvent) {
        // The borrow ends before anything touches the DOM, so a handler that
        // reads this state back cannot deadlock the cell.
        let (row, gesture, dx, dy, width) = {
            let mut slot = presses.borrow_mut();
            let Some(p) = slot.as_mut() else {
                return;
            };
            if p.pointer_id != ev.pointer_id() {
                return;
            }
            let (dx, dy) = p.travel(ev);
            let held_ms = if p.lifts { p.held_ms() } else { 0 };
            p.gesture = advance(p.gesture, dx, dy, held_ms);
            (p.row.clone(), p.gesture, dx, dy, p.width)
        };
        match gesture {
            Gesture::Pressed => {}
            Gesture::Scrolling => move_row(&row, 0.0, 0.0),
            Gesture::Swiping => {
                // The row stays put; the ink follows the finger across it.
                row.class_list().add_1("row-swiping").ok();
                draw_ink(&row, width, ink_drawn(dx, width));
            }
            Gesture::Lifted => {
                row.class_list().add_1("row-lifted").ok();
                move_row(&row, dx, dy);
            }
        }
    }

    /// The row a drag was let go over, and which side of it the pointer was on.
    /// Empty paper gives `None`, and the caller puts the slip at the end.
    pub fn drop_target(x: f64, y: f64) -> Option<(String, Slot)> {
        let doc = web_sys::window()?.document()?;
        let under = doc.element_from_point(x as f32, y as f32)?;
        let row = under.closest("li[data-id]").ok().flatten()?;
        let id = row.get_attribute("data-id")?;
        let rect = row.get_bounding_client_rect();
        let slot = if lands_above(y, rect.top(), rect.bottom()) {
            Slot::Before
        } else {
            Slot::After
        };
        Some((id, slot))
    }

    /// What letting go means, and where the pointer let go. The row is already
    /// put back: every outcome here is either a new order, a delete, or the
    /// list exactly as it was.
    pub fn up(
        presses: &Presses,
        ev: &leptos::ev::PointerEvent,
    ) -> Option<(Release, String, f64, f64)> {
        let press = presses.borrow_mut().take()?;
        if press.pointer_id != ev.pointer_id() {
            *presses.borrow_mut() = Some(press);
            return None;
        }
        let dx = f64::from(ev.client_x()) - press.x;
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let velocity = dx / f64::from(press.held_ms().max(1));
        let outcome = release(press.gesture, dx, press.width, velocity);
        let _ = press.row.release_pointer_capture(press.pointer_id);
        settle(&press.row);
        match outcome {
            Release::Done => end_ink(&press.row, true),
            Release::SnapBack => end_ink(&press.row, false),
            Release::Tap | Release::Reorder | Release::Scroll => remove_ink(&press.row),
        }
        Some((outcome, press.row.get_attribute("data-id")?, x, y))
    }

    /// A pointer that went away without a release - the browser took it over to
    /// scroll. Put the row back and claim nothing.
    pub fn cancel(presses: &Presses) {
        if let Some(press) = presses.borrow_mut().take() {
            let _ = press.row.release_pointer_capture(press.pointer_id);
            settle(&press.row);
            remove_ink(&press.row);
        }
    }
}

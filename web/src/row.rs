//! What a press on a task row has become.
//!
//! A row answers three gestures from the same finger: a tap, a press-and-hold
//! that picks the row up to reorder it, and a swipe to the right that scribbles
//! it out, the ink following the finger. The browser cannot tell them apart, and guessing wrong is expensive —
//! a tap that becomes a drag throws the user's list around, a hold that stays
//! a tap never reorders anything — so every decision is a pure function of the
//! pointer's travel and how long it has been down, and all of it is unit
//! tested. Only the DOM side that feeds these numbers in lives in `tasks/mod.rs`.

/// How long a finger stays down before the row is picked up. Long enough that
/// nobody picks a row up by accident while scrolling or by resting a thumb on
/// the phone, short enough that it does not feel like a wait.
pub const HOLD_MS: u32 = 280;

/// Travel that ends the hold without picking the row up: past this the finger
/// is scrolling the page, which is what a finger that slides is almost always
/// doing.
pub const SLOP_PX: f64 = 8.0;

/// The least rightward travel that deletes when the pointer lets go: a flick
/// needs this much, a slow swipe a share of the row (see [`scribble_commits`]).
pub const SWIPE_PX: f64 = 64.0;

use crate::tuning::{
    SCRIBBLE_BAND, SCRIBBLE_COMMIT_SHARE, SCRIBBLE_FLICK_PX_PER_MS, SCRIBBLE_JITTER,
    SCRIBBLE_LOOP_PX,
};

/// What a press on a row has turned into so far. Every state is decided, and
/// only `Pressed` is still undecided.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gesture {
    /// Down, but not yet a tap, a lift or a swipe.
    Pressed,
    /// The finger slid without settling: this is a scroll, and the row's
    /// business ends here.
    Scrolling,
    /// Held long enough: the row is off the page and moving with the finger.
    Lifted,
    /// Travelling right: the row is on its way out.
    Swiping,
}

/// One step of the gesture. `dx` and `dy` are how far the pointer has travelled
/// from where it went down; `held_ms` how long it has been down.
///
/// Decided states are final, so a finger that lifts and then travels right
/// reorders rather than deletes - the hold was the more deliberate of the two.
pub fn advance(g: Gesture, dx: f64, dy: f64, held_ms: u32) -> Gesture {
    match g {
        Gesture::Lifted | Gesture::Swiping | Gesture::Scrolling => g,
        Gesture::Pressed => {
            if held_ms >= HOLD_MS {
                Gesture::Lifted
            } else if (dx * dx + dy * dy).sqrt() < SLOP_PX {
                Gesture::Pressed
            } else if dx > dy.abs() {
                // Decided as soon as the pointer leaves the slop: a pointer
                // reports every few pixels, so waiting for a full swipe's
                // travel would call every real swipe a scroll first.
                Gesture::Swiping
            } else {
                Gesture::Scrolling
            }
        }
    }
}

/// What letting go means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Release {
    /// A tap. The row's own controls - the box, the title, the x - take it.
    Tap,
    /// Dropped somewhere in the list: reorder onto whatever row is under it.
    Reorder,
    /// Finish the scribble across the row: the task is done, and tears off.
    Done,
    /// Not far enough to mean it: the ink lifts away and the row stays.
    SnapBack,
    /// A scroll, not ours. Nothing to undo.
    Scroll,
}

/// What releasing a pointer does, given what the gesture had become. A swipe is
/// judged by [`scribble_commits`] on how far right it finished (`dx`) across a row
/// `width` wide, and how fast it was going (`velocity`, px per ms).
pub fn release(g: Gesture, dx: f64, width: f64, velocity: f64) -> Release {
    match g {
        Gesture::Pressed => Release::Tap,
        Gesture::Lifted => Release::Reorder,
        Gesture::Scrolling => Release::Scroll,
        Gesture::Swiping if scribble_commits(dx, width, velocity) => Release::Done,
        Gesture::Swiping => Release::SnapBack,
    }
}

/// Whether letting go of a swipe deletes the row: past a share of its width, or a
/// flick past [`SWIPE_PX`]. Slow and short, the user is still deciding.
pub fn scribble_commits(dx: f64, width: f64, velocity: f64) -> bool {
    let reach = (width * SCRIBBLE_COMMIT_SHARE).max(SWIPE_PX);
    dx >= reach || (dx >= SWIPE_PX && velocity >= SCRIBBLE_FLICK_PX_PER_MS)
}

/// How much of the scribble the finger has drawn, 0 to 1: the ink is under the
/// finger, never ahead of it.
pub fn ink_drawn(dx: f64, width: f64) -> f64 {
    if width <= 0.0 {
        return 0.0;
    }
    (dx / width).clamp(0.0, 1.0)
}

/// Whether the timer of the stroke `lifted`, lifting away after a short swipe, may
/// take the ink now on its row (`on_row`): only its own. A new swipe that starts
/// before the lift ends draws a stroke of its own, and keeps it.
pub fn lift_clears(lifted: u32, on_row: Option<u32>) -> bool {
    on_row == Some(lifted)
}

/// Deterministic small value for a strip, so a given task always hangs from
/// the same offset on the pin and lies at the same jaunty angle. FNV-1a.
pub fn hash_unit(id: &str, salt: u8) -> f64 {
    // The top 24 bits mapped to -1..=1.
    (f64::from(hash32(id, salt) >> 8) / 16_777_215.0 - 0.5) * 2.0
}

fn hash32(id: &str, salt: u8) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in id.bytes().chain([salt]) {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

/// Where a row's words sit inside the row, in px from its top-left corner: the
/// first line's height, and from where the words start to where they end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Words {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

/// The scribble across a row's `words`, as an SVG path: loops from where the words
/// start to where they end, inside their [`SCRIBBLE_BAND`]. Each loop's width and
/// height wander by [`SCRIBBLE_JITTER`], seeded by `seed` (the row's id). It is
/// drawn whole and revealed as far as the swipe has gone.
pub fn scribble_path(words: Words, seed: &str) -> String {
    let (low, high) = SCRIBBLE_BAND;
    let height = words.bottom - words.top;
    let (top, bottom) = (words.top + height * low, words.top + height * high);
    let (middle, reach) = ((top + bottom) / 2.0, (bottom - top) / 2.0);
    let jitter = |i: usize, salt: u8| SCRIBBLE_JITTER * hash_unit(&format!("{seed}:{i}"), salt);
    let mut x = words.left;
    let loops: String = (1_usize..)
        .map_while(|i| {
            (x < words.right).then(|| {
                x = (x + SCRIBBLE_LOOP_PX / 2.0 * (1.0 + jitter(i, 0))).min(words.right);
                let side = if i % 2 == 1 { -1.0 } else { 1.0 };
                let y = middle + side * reach * (1.0 - jitter(i, 1).abs());
                format!(" L{x:.1} {y:.1}")
            })
        })
        .collect();
    format!("M{:.1} {middle:.1}{loops}", words.left)
}

/// Where a row lands relative to the row it was dropped on, which the drop
/// decides from which half of that row the pointer was over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Before,
    After,
}

/// Whether a drop at `y` on a row spanning `top..=bottom` belongs above it.
pub fn lands_above(y: f64, top: f64, bottom: f64) -> bool {
    y < (top + bottom) / 2.0
}

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
            gloo_timers::callback::Timeout::new(ms, move || {
                if lift_clears(lifted, stroke_on(&row)) {
                    remove_ink(&row);
                }
            })
            .forget();
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
        wasm_bindgen_futures::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(HOLD_MS).await;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A desktop list row, where a share of the width is less than SWIPE_PX, and a
    /// pointer moving too slowly to count as a flick.
    const ROW: f64 = 120.0;
    const SLOW: f64 = 0.01;

    fn held(g: Gesture, dx: f64, dy: f64, ms: u32) -> Gesture {
        advance(g, dx, dy, ms)
    }

    #[test]
    fn a_touch_that_does_not_move_or_wait_is_still_a_tap() {
        assert_eq!(held(Gesture::Pressed, 0.0, 0.0, 0), Gesture::Pressed);
        assert_eq!(held(Gesture::Pressed, 1.0, 0.0, 100), Gesture::Pressed);
        assert_eq!(release(Gesture::Pressed, 2.0, ROW, SLOW), Release::Tap);
    }

    #[test]
    fn resting_a_thumb_picks_the_row_up() {
        assert_eq!(held(Gesture::Pressed, 1.0, 0.0, HOLD_MS), Gesture::Lifted);
        assert_eq!(
            held(Gesture::Pressed, 0.0, 0.0, HOLD_MS + 500),
            Gesture::Lifted
        );
    }

    #[test]
    fn a_sliding_finger_scrolls_and_never_lifts_the_row() {
        // Down the list, past the slop but never held: this is a scroll.
        assert_eq!(held(Gesture::Pressed, 0.0, 40.0, 10), Gesture::Scrolling);
        // And once scrolling, staying down for a long time does not pick it up:
        // a finger that has already started scrolling is scrolling.
        assert_eq!(
            held(Gesture::Scrolling, 0.0, 90.0, HOLD_MS + 900),
            Gesture::Scrolling
        );
        assert_eq!(release(Gesture::Scrolling, 0.0, ROW, SLOW), Release::Scroll);
    }

    #[test]
    fn travelling_right_crosses_the_row_out() {
        assert_eq!(held(Gesture::Pressed, SWIPE_PX, 0.0, 50), Gesture::Swiping);
        assert_eq!(
            release(Gesture::Swiping, SWIPE_PX, ROW, SLOW),
            Release::Done
        );
        // A diagonal that happens to go far right is still a scroll unless the
        // rightward travel clearly wins.
        assert_eq!(
            held(Gesture::Pressed, SWIPE_PX, SWIPE_PX, 50),
            Gesture::Scrolling
        );
        assert_eq!(
            held(Gesture::Pressed, SWIPE_PX, SWIPE_PX - 1.0, 50),
            Gesture::Swiping
        );
    }

    #[test]
    fn a_swipe_that_is_backed_out_snaps_home() {
        assert_eq!(held(Gesture::Pressed, 90.0, 0.0, 50), Gesture::Swiping);
        assert_eq!(
            release(Gesture::Swiping, 20.0, ROW, SLOW),
            Release::SnapBack
        );
        // Right, but not far enough to mean it: the row goes back unharmed.
        assert_eq!(
            held(Gesture::Pressed, SWIPE_PX - 1.0, 0.0, 50),
            Gesture::Swiping
        );
        assert_eq!(
            release(Gesture::Swiping, SWIPE_PX - 1.0, ROW, SLOW),
            Release::SnapBack
        );
    }

    #[test]
    fn a_swipe_reported_a_few_pixels_at_a_time_still_deletes() {
        // A mouse or finger sends a move every few pixels, so the gesture
        // passes the slop long before it has travelled a whole swipe.
        let gesture = (1..=40).fold(Gesture::Pressed, |g, step| {
            held(g, f64::from(step) * 2.0, 1.0, step * 4)
        });
        assert_eq!(gesture, Gesture::Swiping);
        assert_eq!(release(gesture, 80.0, ROW, SLOW), Release::Done);
    }

    #[test]
    fn a_swipe_left_is_a_scroll_so_the_page_turns() {
        // Swiping left is the page flip, not ours: it must reach the panes.
        assert_eq!(held(Gesture::Pressed, -90.0, 0.0, 50), Gesture::Scrolling);
    }

    #[test]
    fn the_hold_wins_over_a_swipe_that_starts_after_it() {
        // Lifted, then dragged right: the row is being reordered, and releasing
        // it must not delete it.
        let lifted = held(Gesture::Pressed, 0.0, 0.0, HOLD_MS);
        assert_eq!(held(lifted, 200.0, 0.0, HOLD_MS + 200), Gesture::Lifted);
        assert_eq!(release(Gesture::Lifted, 200.0, ROW, SLOW), Release::Reorder);
    }

    #[test]
    fn a_decided_gesture_is_never_re_decided() {
        // Every decided state is a fixed point, which is what stops a finger
        // that drifts back under the slop from turning into a tap.
        for g in [Gesture::Lifted, Gesture::Swiping, Gesture::Scrolling] {
            assert_eq!(held(g, 0.0, 0.0, 0), g);
            assert_eq!(held(g, -200.0, 0.0, 9999), g);
        }
    }

    #[test]
    fn a_phone_row_needs_a_real_swipe_or_a_flick() {
        let phone = 350.0;
        let reach = phone * SCRIBBLE_COMMIT_SHARE;
        assert!(
            !scribble_commits(reach - 1.0, phone, SLOW),
            "still deciding"
        );
        assert!(scribble_commits(reach, phone, SLOW));
        // A quick flick deletes from SWIPE_PX on, but never from a nudge.
        assert!(scribble_commits(SWIPE_PX, phone, SCRIBBLE_FLICK_PX_PER_MS));
        assert!(!scribble_commits(SWIPE_PX - 1.0, phone, 1.0));
        assert_eq!(
            release(Gesture::Swiping, reach - 1.0, phone, SLOW),
            Release::SnapBack
        );
    }

    #[test]
    fn the_ink_is_under_the_finger_and_never_ahead() {
        assert_eq!(ink_drawn(0.0, 300.0), 0.0);
        assert_eq!(ink_drawn(150.0, 300.0), 0.5);
        assert_eq!(ink_drawn(900.0, 300.0), 1.0, "past the edge: drawn whole");
        assert_eq!(ink_drawn(-40.0, 300.0), 0.0, "back left of where it began");
        assert_eq!(ink_drawn(40.0, 0.0), 0.0, "a row with no width");
    }

    fn points(path: &str) -> Vec<(f64, f64)> {
        path.split(['M', 'L'])
            .filter_map(|point| {
                let mut xy = point.split_whitespace().map(|n| n.parse::<f64>().ok());
                Some((xy.next()??, xy.next()??))
            })
            .collect()
    }

    #[test]
    fn the_scribble_crosses_the_words_and_nothing_else() {
        // A title from 40px (past the checkbox) to 220px, its line 10 to 30px down.
        let words = Words {
            left: 40.0,
            right: 220.0,
            top: 10.0,
            bottom: 30.0,
        };
        let points = points(&scribble_path(words, "t1"));
        assert_eq!(
            points.first().map(|p| p.0),
            Some(40.0),
            "starts at the words"
        );
        assert_eq!(points.last().map(|p| p.0), Some(220.0), "ends with them");
        assert!(
            points.windows(2).all(|w| w[1].0 > w[0].0),
            "always left to right"
        );
        let (low, high) = SCRIBBLE_BAND;
        let band = (10.0 + 20.0 * low - 0.05)..=(10.0 + 20.0 * high + 0.05);
        assert!(
            points.iter().all(|(_, y)| band.contains(y)),
            "inside the letters' height: {points:?}"
        );
        assert!(points.len() > 15, "loops, not a line: {}", points.len());
    }

    #[test]
    fn a_lift_only_takes_its_own_stroke() {
        assert!(lift_clears(3, Some(3)), "its own ink lifts away");
        assert!(
            !lift_clears(3, Some(4)),
            "a new swipe began within the lift: its ink stays"
        );
        assert!(!lift_clears(3, None), "already gone");
    }

    #[test]
    fn each_row_scribbles_its_own_way() {
        let words = Words {
            left: 0.0,
            right: 300.0,
            top: 0.0,
            bottom: 24.0,
        };
        assert_eq!(
            scribble_path(words, "a"),
            scribble_path(words, "a"),
            "seeded"
        );
        assert_ne!(scribble_path(words, "a"), scribble_path(words, "b"));
        // Not a sawtooth: the loops are not all one width or one height.
        let points = points(&scribble_path(words, "a"));
        let widths: Vec<i64> = points
            .windows(2)
            .map(|w| ((w[1].0 - w[0].0) * 10.0).round() as i64)
            .collect();
        let heights: Vec<i64> = points.iter().map(|p| (p.1 * 10.0).round() as i64).collect();
        assert!(widths[..widths.len() - 1].windows(2).any(|w| w[0] != w[1]));
        assert!(
            heights
                .iter()
                .skip(1)
                .step_by(2)
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 1
        );
    }

    #[test]
    fn the_top_half_of_a_row_is_above_it() {
        assert!(lands_above(110.0, 100.0, 140.0));
        assert!(!lands_above(130.0, 100.0, 140.0));
        // Exactly the middle counts as below, so a drop down the list that
        // lands on the seam does not jump a row upwards.
        assert!(!lands_above(120.0, 100.0, 140.0));
    }
}

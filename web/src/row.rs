//! What a press on a task row has become.
//!
//! A row answers three gestures from the same finger: a tap, a press-and-hold
//! that picks the row up to reorder it, and a swipe to the right that crosses
//! it out. The browser cannot tell them apart, and guessing wrong is expensive
//! - a tap that becomes a drag throws the user's list around, a hold that stays
//! a tap never reorders anything - so every decision is a pure function of the
//! pointer's travel and how long it has been down, and all of it is unit
//! tested. Only the DOM side that feeds these numbers in lives in `lib.rs`.

/// How long a finger stays down before the row is picked up. Long enough that
/// nobody picks a row up by accident while scrolling or by resting a thumb on
/// the phone, short enough that it does not feel like a wait.
pub const HOLD_MS: u32 = 280;

/// Travel that ends the hold without picking the row up: past this the finger
/// is scrolling the page, which is what a finger that slides is almost always
/// doing.
pub const SLOP_PX: f64 = 8.0;

/// Rightward travel that turns a press into a delete, in either direction:
/// this far right crosses the row out, back past half of it snaps the row home.
pub const SWIPE_PX: f64 = 64.0;

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
            } else if dx >= SWIPE_PX && dx > dy.abs() {
                Gesture::Swiping
            } else if (dx * dx + dy * dy).sqrt() >= SLOP_PX {
                Gesture::Scrolling
            } else {
                Gesture::Pressed
            }
        }
    }
}

/// What letting go means. `dx` is how far right the pointer finished.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Release {
    /// A tap. The row's own controls - the box, the title, the x - take it.
    Tap,
    /// Dropped somewhere in the list: reorder onto whatever row is under it.
    Reorder,
    /// Cross the row out and delete it.
    Delete,
    /// Not far enough to mean it: put the row back where it was.
    SnapBack,
    /// A scroll, not ours. Nothing to undo.
    Scroll,
}

/// What releasing a pointer does, given what the gesture had become.
pub fn release(g: Gesture, dx: f64) -> Release {
    match g {
        Gesture::Pressed => Release::Tap,
        Gesture::Lifted => Release::Reorder,
        Gesture::Scrolling => Release::Scroll,
        Gesture::Swiping => match swipe_action(dx, SWIPE_PX) {
            SwipeAction::Delete => Release::Delete,
            SwipeAction::SnapBack => Release::SnapBack,
        },
    }
}

/// How far a row travelled to the right, and what that means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwipeAction {
    Delete,
    SnapBack,
}

pub fn swipe_action(delta_x: f64, threshold: f64) -> SwipeAction {
    if delta_x >= threshold {
        SwipeAction::Delete
    } else {
        SwipeAction::SnapBack
    }
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
    use super::{advance, lands_above, release, Gesture, Release, Slot, HOLD_MS};
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
    }

    impl Press {
        fn held_ms(&self) -> u32 {
            (js_sys::Date::now() - self.started_ms).max(0.0) as u32
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
        el.clone().dyn_into::<web_sys::HtmlElement>().ok().map(|el| el.style())
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
    pub fn down(presses: &Presses, ev: &leptos::ev::PointerEvent) -> bool {
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
            started_ms: js_sys::Date::now(),
            gesture: Gesture::Pressed,
        });
        // A finger that never moves still picks the row up when the hold runs
        // out, so the lift cannot depend on a pointermove arriving.
        let presses = presses.clone();
        wasm_bindgen_futures::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(HOLD_MS).await;
            let lifted = match presses.borrow_mut().as_mut() {
                Some(p) if p.gesture == Gesture::Pressed => {
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
        let (row, gesture, dx, dy) = {
            let mut slot = presses.borrow_mut();
            let Some(p) = slot.as_mut() else {
                return;
            };
            if p.pointer_id != ev.pointer_id() {
                return;
            }
            let (dx, dy) = p.travel(ev);
            p.gesture = advance(p.gesture, dx, dy, p.held_ms());
            (p.row.clone(), p.gesture, dx, dy)
        };
        match gesture {
            Gesture::Pressed => {}
            Gesture::Scrolling => move_row(&row, 0.0, 0.0),
            Gesture::Swiping => {
                row.class_list().add_1("row-swiping").ok();
                move_row(&row, dx, 0.0);
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
    pub fn up(presses: &Presses, ev: &leptos::ev::PointerEvent) -> Option<(Release, String, f64, f64)> {
        let press = presses.borrow_mut().take()?;
        if press.pointer_id != ev.pointer_id() {
            *presses.borrow_mut() = Some(press);
            return None;
        }
        let dx = f64::from(ev.client_x()) - press.x;
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let outcome = release(press.gesture, dx);
        let _ = press.row.release_pointer_capture(press.pointer_id);
        settle(&press.row);
        Some((outcome, press.row.get_attribute("data-id")?, x, y))
    }

    /// A pointer that went away without a release - the browser took it over to
    /// scroll. Put the row back and claim nothing.
    pub fn cancel(presses: &Presses) {
        if let Some(press) = presses.borrow_mut().take() {
            let _ = press.row.release_pointer_capture(press.pointer_id);
            settle(&press.row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(g: Gesture, dx: f64, dy: f64, ms: u32) -> Gesture {
        advance(g, dx, dy, ms)
    }

    #[test]
    fn a_touch_that_does_not_move_or_wait_is_still_a_tap() {
        assert_eq!(held(Gesture::Pressed, 0.0, 0.0, 0), Gesture::Pressed);
        assert_eq!(held(Gesture::Pressed, 1.0, 0.0, 100), Gesture::Pressed);
        assert_eq!(release(Gesture::Pressed, 2.0), Release::Tap);
    }

    #[test]
    fn resting_a_thumb_picks_the_row_up() {
        assert_eq!(held(Gesture::Pressed, 1.0, 0.0, HOLD_MS), Gesture::Lifted);
        assert_eq!(held(Gesture::Pressed, 0.0, 0.0, HOLD_MS + 500), Gesture::Lifted);
    }

    #[test]
    fn a_sliding_finger_scrolls_and_never_lifts_the_row() {
        // Down the list, past the slop but never held: this is a scroll.
        assert_eq!(held(Gesture::Pressed, 0.0, 40.0, 10), Gesture::Scrolling);
        // And once scrolling, staying down for a long time does not pick it up:
        // a finger that has already started scrolling is scrolling.
        assert_eq!(held(Gesture::Scrolling, 0.0, 90.0, HOLD_MS + 900), Gesture::Scrolling);
        assert_eq!(release(Gesture::Scrolling, 0.0), Release::Scroll);
    }

    #[test]
    fn travelling_right_crosses_the_row_out() {
        assert_eq!(held(Gesture::Pressed, SWIPE_PX, 0.0, 50), Gesture::Swiping);
        assert_eq!(release(Gesture::Swiping, SWIPE_PX), Release::Delete);
        // A diagonal that happens to go far right is still a scroll unless the
        // rightward travel clearly wins.
        assert_eq!(held(Gesture::Pressed, SWIPE_PX, SWIPE_PX, 50), Gesture::Scrolling);
        assert_eq!(held(Gesture::Pressed, SWIPE_PX, SWIPE_PX - 1.0, 50), Gesture::Swiping);
    }

    #[test]
    fn a_swipe_that_is_backed_out_snaps_home() {
        assert_eq!(held(Gesture::Pressed, 90.0, 0.0, 50), Gesture::Swiping);
        assert_eq!(release(Gesture::Swiping, 20.0), Release::SnapBack);
        // Right, but not far enough to mean it and not held: the row is left
        // alone rather than deleted, and letting go undoes nothing.
        assert_eq!(held(Gesture::Pressed, SWIPE_PX - 1.0, 0.0, 50), Gesture::Scrolling);
        assert_eq!(release(Gesture::Scrolling, SWIPE_PX - 1.0), Release::Scroll);
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
        assert_eq!(release(Gesture::Lifted, 200.0), Release::Reorder);
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
    fn swipe_action_past_threshold_deletes() {
        assert_eq!(swipe_action(80.0, 50.0), SwipeAction::Delete);
    }
    #[test]
    fn short_swipe_snaps_back() {
        assert_eq!(swipe_action(20.0, 50.0), SwipeAction::SnapBack);
    }
    #[test]
    fn exactly_threshold_deletes() {
        assert_eq!(swipe_action(50.0, 50.0), SwipeAction::Delete);
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

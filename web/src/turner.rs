//! The browser side of the page bend (`bend.rs` holds the geometry and the decisions).
//!
//! On a phone the two panes sit in a scroll-snap row. While a page turns they are
//! stacked in one cell instead: the pane being turned on top, clipped to what stays
//! flat of it, the next pane beneath, and the flap and its shadows drawn between by
//! `.bend-fx`. When the turn ends the row comes back with the right pane in view.
//! A press is only a page turn once it is a sideways swipe the right way on a pane that
//! has a page to turn to; everything else, the task rows' own swipes, the planner grid's
//! scroll, the tack board, the edit sheet, is left alone.

#[cfg(target_arch = "wasm32")]
pub(crate) use imp::Turner;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use stub::Turner;

#[cfg(not(target_arch = "wasm32"))]
mod stub {
    use leptos::prelude::*;

    /// The page bends only in the browser; native builds (the tests) have no page.
    #[derive(Clone)]
    pub(crate) struct Turner;

    impl Turner {
        pub(crate) fn new(
            _main: NodeRef<leptos::html::Main>,
            _fx: NodeRef<leptos::html::Div>,
            _set_pane: WriteSignal<u8>,
        ) -> Self {
            Turner
        }
        pub(crate) fn down(&self, _ev: &leptos::ev::PointerEvent) {}
        pub(crate) fn moved(&self, _ev: &leptos::ev::PointerEvent) {}
        pub(crate) fn up(&self, _ev: &leptos::ev::PointerEvent) {}
        pub(crate) fn cancel(&self, _ev: &leptos::ev::PointerEvent) {}
        pub(crate) fn turn_to(&self, _index: u8) {}
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::cell::RefCell;
    use std::rc::Rc;

    use leptos::prelude::*;
    use wasm_bindgen::JsCast;
    use web_sys::{Element, HtmlElement};

    use crate::bend::{self, Pt, Settle, Turn};
    use crate::pane::{nearest_pane, show_pane};
    use crate::tuning::{
        BEND_BUTTON_ARC, BEND_BUTTON_MS, BEND_COMPLETE_MS, BEND_FULL_TURN, BEND_SLIDE_MS,
        BEND_SPRING_MS, PHONE_MEDIA,
    };

    /// Presses that start here are not page turns: they have their own scroll or swipe.
    const OWN_GESTURE: &str =
        ".week-scroll, .task-dialog, .tack-board, input, textarea, select, .paper-picker";

    /// Where the page turn is.
    enum Phase {
        Idle,
        /// Down, not yet a swipe.
        Pressed {
            id: i32,
            x: f64,
            y: f64,
            pane: u8,
        },
        /// The finger has the corner.
        Dragging(Drag),
        /// Let go: the page is finishing or springing back on its own.
        Settling,
    }

    /// One turn's page, in px, and how it is drawn.
    #[derive(Clone, Copy)]
    struct Stage {
        w: f64,
        h: f64,
        corner: Pt,
        turn: Turn,
        /// Reduced motion: slide the page instead of bending it.
        slide: bool,
    }

    struct Drag {
        id: i32,
        stage: Stage,
        /// Where the finger was when the page took hold of the corner.
        origin: Pt,
        /// The corner's travel from where it started.
        drag: Pt,
        /// Time and reach at the last move, for the speed on letting go.
        at: f64,
        reach: f64,
        velocity: f64,
    }

    struct Inner {
        main: NodeRef<leptos::html::Main>,
        fx: NodeRef<leptos::html::Div>,
        set_pane: WriteSignal<u8>,
        phase: RefCell<Phase>,
    }

    #[derive(Clone)]
    pub(crate) struct Turner(Rc<Inner>);

    fn phone() -> bool {
        window()
            .match_media(PHONE_MEDIA)
            .ok()
            .flatten()
            .is_some_and(|query| query.matches())
    }

    fn now() -> f64 {
        js_sys::Date::now()
    }

    fn el_style(el: &Element, property: &str, value: &str) {
        if let Some(html) = el.dyn_ref::<HtmlElement>() {
            let _ = html.style().set_property(property, value);
        }
    }

    impl Turner {
        pub(crate) fn new(
            main: NodeRef<leptos::html::Main>,
            fx: NodeRef<leptos::html::Div>,
            set_pane: WriteSignal<u8>,
        ) -> Self {
            Turner(Rc::new(Inner {
                main,
                fx,
                set_pane,
                phase: RefCell::new(Phase::Idle),
            }))
        }

        pub(crate) fn down(&self, ev: &leptos::ev::PointerEvent) {
            if !matches!(*self.0.phase.borrow(), Phase::Idle) || !ev.is_primary() || !phone() {
                return;
            }
            let own = ev
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .is_some_and(|el| el.closest(OWN_GESTURE).ok().flatten().is_some());
            let Some(main) = self.0.main.get() else {
                return;
            };
            if own {
                return;
            }
            *self.0.phase.borrow_mut() = Phase::Pressed {
                id: ev.pointer_id(),
                x: f64::from(ev.client_x()),
                y: f64::from(ev.client_y()),
                pane: nearest_pane(&main),
            };
        }

        pub(crate) fn moved(&self, ev: &leptos::ev::PointerEvent) {
            let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
            let mut phase = self.0.phase.borrow_mut();
            match &mut *phase {
                Phase::Pressed {
                    id,
                    x: x0,
                    y: y0,
                    pane,
                } if *id == ev.pointer_id() => {
                    let (dx, dy) = (x - *x0, y - *y0);
                    let (id, pane) = (*id, *pane);
                    if let Some(turn) = bend::turn_for(pane, dx, dy) {
                        // A row lifted for reordering owns its drag.
                        let lifted = ev
                            .target()
                            .and_then(|target| target.dyn_into::<Element>().ok())
                            .is_some_and(|el| el.closest("li.row-lifted").ok().flatten().is_some());
                        if lifted {
                            *phase = Phase::Idle;
                            return;
                        }
                        match self.begin(turn, Pt::new(x, y)) {
                            Some(mut drag) => {
                                drag.id = id;
                                if let Some(main) = self.0.main.get() {
                                    let _ = main.set_pointer_capture(id);
                                }
                                *phase = Phase::Dragging(drag);
                            }
                            None => *phase = Phase::Idle,
                        }
                    } else if dy.abs() > dx.abs() && dx.hypot(dy) >= crate::tuning::BEND_SLOP_PX {
                        // Up and down: a scroll, not ours.
                        *phase = Phase::Idle;
                    }
                }
                Phase::Dragging(drag) if drag.id == ev.pointer_id() => {
                    let raw = Pt::new(x - drag.origin.x, y - drag.origin.y);
                    drag.drag = bend::limited(raw, drag.stage.w, BEND_FULL_TURN);
                    let reach = drag.drag.x * drag.stage.turn.inward();
                    let t = now();
                    if t > drag.at {
                        drag.velocity = (reach - drag.reach) / (t - drag.at);
                    }
                    drag.at = t;
                    drag.reach = reach;
                    self.draw(&drag.stage, drag.drag);
                }
                _ => {}
            }
        }

        pub(crate) fn up(&self, ev: &leptos::ev::PointerEvent) {
            self.release(ev, false);
        }

        pub(crate) fn cancel(&self, ev: &leptos::ev::PointerEvent) {
            self.release(ev, true);
        }

        /// A page turn from a button: the same bend, with nobody holding the corner.
        pub(crate) fn turn_to(&self, index: u8) {
            let Some(main) = self.0.main.get() else {
                return;
            };
            let from = nearest_pane(&main);
            if !matches!(*self.0.phase.borrow(), Phase::Idle) || from == index {
                return;
            }
            if !phone() {
                show_pane(self.0.main, index, self.0.set_pane);
                return;
            }
            let turn = if index == 1 {
                Turn::Forward
            } else {
                Turn::Back
            };
            let Some(drag) = self.begin(turn, Pt::new(0.0, 0.0)) else {
                show_pane(self.0.main, index, self.0.set_pane);
                return;
            };
            let stage = drag.stage;
            *self.0.phase.borrow_mut() = Phase::Settling;
            let ms = if stage.slide {
                BEND_SLIDE_MS
            } else {
                BEND_BUTTON_MS
            };
            self.run(
                stage,
                Pt::new(0.0, 0.0),
                self.finish_drag(&stage),
                ms,
                Settle::Complete,
                BEND_BUTTON_ARC,
            );
        }

        fn release(&self, ev: &leptos::ev::PointerEvent, cancelled: bool) {
            let mut phase = self.0.phase.borrow_mut();
            match &*phase {
                Phase::Pressed { id, .. } if *id == ev.pointer_id() => *phase = Phase::Idle,
                Phase::Dragging(drag) if drag.id == ev.pointer_id() => {
                    let (stage, from) = (drag.stage, drag.drag);
                    let outcome = if cancelled {
                        Settle::SpringBack
                    } else {
                        bend::settle(drag.reach, drag.velocity, stage.w)
                    };
                    *phase = Phase::Settling;
                    drop(phase);
                    let (to, ms) = match outcome {
                        Settle::Complete => (
                            self.finish_drag(&stage),
                            if stage.slide {
                                BEND_SLIDE_MS
                            } else {
                                BEND_COMPLETE_MS
                            },
                        ),
                        Settle::SpringBack => (
                            Pt::new(0.0, 0.0),
                            if stage.slide {
                                BEND_SLIDE_MS
                            } else {
                                BEND_SPRING_MS
                            },
                        ),
                    };
                    self.run(stage, from, to, ms, outcome, 0.0);
                }
                _ => {}
            }
        }

        /// Where the corner ends up when the page is turned right over: far enough that
        /// nothing of the page is left.
        fn finish_drag(&self, stage: &Stage) -> Pt {
            Pt::new(stage.turn.inward() * BEND_FULL_TURN * stage.w, 0.0)
        }

        /// Stacks the panes for a turn and takes hold of the corner nearest `finger`.
        fn begin(&self, turn: Turn, finger: Pt) -> Option<Drag> {
            let main = self.0.main.get()?;
            let fx = self.0.fx.get()?;
            let panes = main.children();
            let front = panes.item(u32::from(turn.turning_pane()))?;
            let under = panes.item(u32::from(turn.revealed_pane()))?;
            // The visible part of the page: a long list runs below the screen, and the
            // corner has to be one the finger can see.
            let rect = main.get_bounding_client_rect();
            let view_h = window()
                .inner_height()
                .ok()
                .and_then(|h| h.as_f64())
                .unwrap_or(rect.height());
            let (top, bottom) = (
                (-rect.top()).max(0.0),
                (view_h - rect.top()).min(rect.height()),
            );
            let _ = main.class_list().add_1("bending");
            main.set_scroll_left(0);
            let _ = front.class_list().add_1("bend-front");
            let _ = under.class_list().add_1("bend-under");
            let (w, h) = (f64::from(fx.client_width()), f64::from(fx.client_height()));
            let low = finger.y - rect.top() > (top + bottom) / 2.0 || finger == Pt::new(0.0, 0.0);
            let corner = Pt::new(
                if turn == Turn::Forward { w } else { 0.0 },
                if low { bottom.min(h) } else { top },
            );
            Some(Drag {
                id: 0,
                stage: Stage {
                    w,
                    h,
                    corner,
                    turn,
                    slide: crate::mascot::prefers_reduced_motion(),
                },
                origin: finger,
                drag: Pt::new(0.0, 0.0),
                at: now(),
                reach: 0.0,
                velocity: 0.0,
            })
        }

        /// Draws the page with the corner `drag` away from where it started.
        fn draw(&self, stage: &Stage, drag: Pt) {
            let (Some(main), Some(fx)) = (self.0.main.get(), self.0.fx.get()) else {
                return;
            };
            let Some(front) = main.children().item(u32::from(stage.turn.turning_pane())) else {
                return;
            };
            if stage.slide {
                el_style(
                    &front,
                    "transform",
                    &format!("translateX({:.1}px)", drag.x.clamp(-stage.w, stage.w)),
                );
                return;
            }
            let finger = Pt::new(stage.corner.x + drag.x, stage.corner.y + drag.y);
            let Some(shapes) = bend::bend(stage.w, stage.h, stage.corner, finger) else {
                el_style(&front, "clip-path", "none");
                el_style(&fx, "visibility", "hidden");
                return;
            };
            el_style(&front, "clip-path", &bend::polygon_css(&shapes.kept));
            el_style(&fx, "visibility", "visible");
            el_style(&fx, "--bend-x", &format!("{:.1}px", shapes.fold.x));
            el_style(&fx, "--bend-y", &format!("{:.1}px", shapes.fold.y));
            el_style(&fx, "--bend-angle", &format!("{:.4}rad", shapes.angle));
            if let Some(away) = fx.first_element_child() {
                el_style(&away, "clip-path", &bend::polygon_css(&shapes.away));
            }
            if let Some(flap) = fx
                .last_element_child()
                .and_then(|flapbox| flapbox.first_element_child())
            {
                el_style(&flap, "clip-path", &bend::polygon_css(&shapes.flap));
            }
        }

        /// Animates the corner from `from` to `to` and then ends the turn.
        fn run(&self, stage: Stage, from: Pt, to: Pt, ms: u32, outcome: Settle, arc: f64) {
            let started = now();
            self.frame(stage, from, to, f64::from(ms), outcome, arc, started);
        }

        #[allow(clippy::too_many_arguments)]
        fn frame(
            &self,
            stage: Stage,
            from: Pt,
            to: Pt,
            ms: f64,
            outcome: Settle,
            arc: f64,
            started: f64,
        ) {
            let t = ((now() - started) / ms).clamp(0.0, 1.0);
            let mut at = bend::eased(from, to, t);
            // A button's turn lifts the corner on the way and sets it down at the end.
            at.y -= arc * stage.h * 4.0 * t * (1.0 - t);
            self.draw(&stage, at);
            if t < 1.0 {
                let this = self.clone();
                request_animation_frame(move || {
                    this.frame(stage, from, to, ms, outcome, arc, started);
                });
            } else {
                self.end(&stage, outcome);
            }
        }

        /// Un-stacks the panes and brings the row back with the page that is showing.
        fn end(&self, stage: &Stage, outcome: Settle) {
            if let Some(main) = self.0.main.get() {
                let panes = main.children();
                for index in 0..2 {
                    if let Some(pane) = panes.item(index) {
                        let _ = pane.class_list().remove_2("bend-front", "bend-under");
                        el_style(&pane, "clip-path", "");
                        el_style(&pane, "transform", "");
                    }
                }
                let _ = main.class_list().remove_1("bending");
            }
            if let Some(fx) = self.0.fx.get() {
                el_style(&fx, "visibility", "hidden");
            }
            let shown = match outcome {
                Settle::Complete => stage.turn.revealed_pane(),
                Settle::SpringBack => stage.turn.turning_pane(),
            };
            show_pane(self.0.main, shown, self.0.set_pane);
            *self.0.phase.borrow_mut() = Phase::Idle;
        }
    }
}

//! The browser side of the book flip (`flip.rs` holds the geometry, the spring and the
//! decisions).
//!
//! On a phone the two pages lie stacked in one cell, the list on top and the planner under
//! it, one showing at rest. While a page turns both show: the list as the leaf, hinged on
//! its left edge and swung in 3D, the planner beneath, and `.bend-fx` drawing the light on
//! the leaf and the shadow it throws on the planner. When the turn ends the page that
//! landed is the one shown.
//! A press is only a page turn once it is a sideways swipe the right way on a page that
//! has a page to turn to; everything else, the task rows' own swipes, the planner grid's
//! scroll, the tack board, the edit sheet, is left alone. The planner grid is the one
//! exception: on its earliest day, a right swipe that starts on empty paper turns back.

#[cfg(target_arch = "wasm32")]
pub(crate) use imp::Turner;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use stub::Turner;

#[cfg(not(target_arch = "wasm32"))]
mod stub {
    use leptos::prelude::*;

    /// The page turns only in the browser; native builds (the tests) have no page.
    #[derive(Clone)]
    pub(crate) struct Turner;

    impl Turner {
        pub(crate) fn new(
            _main: NodeRef<leptos::html::Main>,
            _fx: NodeRef<leptos::html::Div>,
            _pane: ReadSignal<u8>,
            _set_pane: WriteSignal<u8>,
        ) -> Self {
            Turner
        }
        pub(crate) fn idle(&self) -> bool {
            true
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

    use crate::flip::{self, Pull, Settle, Spring, Turn};
    use crate::tuning::{
        BEND_BUTTON_MS, FLIP_FALL_DAMPING, FLIP_FALL_OMEGA, FLIP_FINISH_OMEGA, FLIP_PERSPECTIVE,
    };
    use crate::turn_util::{el_style, element_of, may_turn_from, now, phone, show_pane};

    /// Reduced motion slides the page: a quick settle with no swing to it.
    const SLIDE_PULL: Pull = Pull {
        omega: 30.0,
        zeta: 1.0,
    };

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
        /// The finger has the leaf.
        Dragging(Drag),
        /// Let go: the page is finishing or falling back on its own.
        Settling,
    }

    /// One turn's page, in px, and how it is drawn.
    #[derive(Clone, Copy)]
    struct Stage {
        w: f64,
        turn: Turn,
        /// Reduced motion: slide the page instead of swinging it.
        slide: bool,
    }

    /// How the leaf moves once nobody holds it.
    #[derive(Clone, Copy)]
    enum Motion {
        /// Let go of: it carries on at its speed on a spring toward `target`.
        Spring {
            spring: Spring,
            target: f64,
            pull: Pull,
        },
        /// A button's turn on a fixed time, `from` to `to`: the same poses forward and back.
        Glide { from: f64, to: f64, ms: f64 },
    }

    /// The leaf let go, until the turn ends.
    #[derive(Clone, Copy)]
    struct Flight {
        stage: Stage,
        motion: Motion,
        outcome: Settle,
        /// When the flight began and when the last frame was drawn, in ms.
        began: f64,
        last: f64,
    }

    struct Drag {
        id: i32,
        stage: Stage,
        /// Where the finger was when the page took hold.
        origin_x: f64,
        /// How far the finger has gone the way the turn goes, in px.
        reach: f64,
        /// Time at the last move, for the speed on letting go.
        at: f64,
        velocity: f64,
    }

    struct Inner {
        main: NodeRef<leptos::html::Main>,
        fx: NodeRef<leptos::html::Div>,
        pane: ReadSignal<u8>,
        set_pane: WriteSignal<u8>,
        phase: RefCell<Phase>,
    }

    #[derive(Clone)]
    pub(crate) struct Turner(Rc<Inner>);

    impl Turner {
        pub(crate) fn new(
            main: NodeRef<leptos::html::Main>,
            fx: NodeRef<leptos::html::Div>,
            pane: ReadSignal<u8>,
            set_pane: WriteSignal<u8>,
        ) -> Self {
            Turner(Rc::new(Inner {
                main,
                fx,
                pane,
                set_pane,
                phase: RefCell::new(Phase::Idle),
            }))
        }

        /// Whether no turn is under way.
        pub(crate) fn idle(&self) -> bool {
            matches!(*self.0.phase.borrow(), Phase::Idle)
        }

        pub(crate) fn down(&self, ev: &leptos::ev::PointerEvent) {
            if !self.idle() || !ev.is_primary() || !phone() {
                return;
            }
            if !element_of(ev).is_some_and(|target| may_turn_from(&target)) {
                return;
            }
            *self.0.phase.borrow_mut() = Phase::Pressed {
                id: ev.pointer_id(),
                x: f64::from(ev.client_x()),
                y: f64::from(ev.client_y()),
                pane: self.0.pane.get_untracked(),
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
                    if let Some(turn) = flip::turn_for(pane, dx, dy) {
                        // A row lifted for reordering owns its drag.
                        let lifted = element_of(ev)
                            .is_some_and(|el| el.closest("li.row-lifted").ok().flatten().is_some());
                        if lifted {
                            *phase = Phase::Idle;
                            return;
                        }
                        match self.begin(turn) {
                            Some(stage) => {
                                if let Some(main) = self.0.main.get() {
                                    let _ = main.set_pointer_capture(id);
                                }
                                *phase = Phase::Dragging(Drag {
                                    id,
                                    stage,
                                    origin_x: *x0,
                                    reach: 0.0,
                                    at: now(),
                                    velocity: 0.0,
                                });
                            }
                            None => *phase = Phase::Idle,
                        }
                    } else if dy.abs() > dx.abs() && dx.hypot(dy) >= crate::tuning::BEND_SLOP_PX {
                        // Up and down: a scroll, not ours.
                        *phase = Phase::Idle;
                    }
                }
                Phase::Dragging(drag) if drag.id == ev.pointer_id() => {
                    let reach =
                        ((x - drag.origin_x) * drag.stage.turn.inward()).clamp(0.0, drag.stage.w);
                    let t = now();
                    if t > drag.at {
                        drag.velocity = (reach - drag.reach) / (t - drag.at);
                    }
                    drag.at = t;
                    drag.reach = reach;
                    let open = drag.stage.turn.open_at(reach / drag.stage.w);
                    self.draw(&drag.stage, open);
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

        /// A page turn from a button: the same flip, with nobody holding the page.
        pub(crate) fn turn_to(&self, index: u8) {
            if !self.idle() || self.0.pane.get_untracked() == index {
                return;
            }
            let turn = if index == 1 {
                Turn::Forward
            } else {
                Turn::Back
            };
            let stage = phone().then(|| self.begin(turn)).flatten();
            let Some(stage) = stage else {
                self.show(index);
                return;
            };
            *self.0.phase.borrow_mut() = Phase::Settling;
            let (from, to) = (
                turn.open_after(Settle::SpringBack),
                turn.open_after(Settle::Complete),
            );
            let motion = if stage.slide {
                Motion::Spring {
                    spring: Spring::new(from, 0.0),
                    target: to,
                    pull: SLIDE_PULL,
                }
            } else {
                Motion::Glide {
                    from,
                    to,
                    ms: f64::from(BEND_BUTTON_MS),
                }
            };
            self.run(Flight {
                stage,
                motion,
                outcome: Settle::Complete,
                began: 0.0,
                last: 0.0,
            });
        }

        fn show(&self, index: u8) {
            if let Some(main) = self.0.main.get() {
                show_pane(&main, index, self.0.set_pane);
            }
        }

        fn release(&self, ev: &leptos::ev::PointerEvent, cancelled: bool) {
            let mut phase = self.0.phase.borrow_mut();
            match &*phase {
                Phase::Pressed { id, .. } if *id == ev.pointer_id() => *phase = Phase::Idle,
                Phase::Dragging(drag) if drag.id == ev.pointer_id() => {
                    let (stage, reach, finger) = (drag.stage, drag.reach, drag.velocity);
                    let outcome = if cancelled {
                        Settle::SpringBack
                    } else {
                        flip::settle(reach, finger, stage.w)
                    };
                    *phase = Phase::Settling;
                    drop(phase);
                    let pull = match (outcome, stage.slide) {
                        (_, true) => SLIDE_PULL,
                        (Settle::Complete, false) => Pull {
                            omega: FLIP_FINISH_OMEGA,
                            zeta: 1.0,
                        },
                        (Settle::SpringBack, false) => Pull {
                            omega: FLIP_FALL_OMEGA,
                            zeta: FLIP_FALL_DAMPING,
                        },
                    };
                    // The finger's speed goes on as the leaf's own.
                    let speed = stage.turn.open_dir() * finger * 1000.0 / stage.w;
                    self.run(Flight {
                        stage,
                        motion: Motion::Spring {
                            spring: Spring::new(stage.turn.open_at(reach / stage.w), speed),
                            target: stage.turn.open_after(outcome),
                            pull,
                        },
                        outcome,
                        began: 0.0,
                        last: 0.0,
                    });
                }
                _ => {}
            }
        }

        /// Stacks the pages for a turn, with the leaf lying where the turn starts.
        fn begin(&self, turn: Turn) -> Option<Stage> {
            let main = self.0.main.get()?;
            let fx = self.0.fx.get()?;
            // Both pages show while a turn is on: the list is the leaf, the planner under it.
            let _ = main.class_list().add_2("bending", "flipping");
            let panes = main.children();
            let front = panes.item(u32::from(turn.turning_pane()))?;
            let under = panes.item(u32::from(turn.revealed_pane()))?;
            // The visible part of the page: a long list runs below the screen.
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
            let _ = front.class_list().add_1("bend-front");
            let _ = under.class_list().add_1("bend-under");
            // Nothing below the screen is seen during a turn, so the leaf is cut there: a
            // long list is not repainted whole on every frame.
            el_style(&front, "max-height", &format!("{bottom:.0}px"));
            let w = f64::from(fx.client_width());
            let mid = (top + bottom) / 2.0;
            // The hinge is the leaf's left edge, level with the middle of what shows.
            el_style(&front, "transform-origin", &format!("0 {mid:.0}px"));
            el_style(&fx, "--flip-origin", &format!("0 {mid:.0}px"));
            el_style(&fx, "--flip-h", &format!("{bottom:.0}px"));
            let stage = Stage {
                w,
                turn,
                slide: crate::mascot::prefers_reduced_motion(),
            };
            // Back starts with the leaf standing at the spine: put it there before the
            // first frame, or the list would cover the planner for a moment.
            self.draw(&stage, turn.open_after(Settle::SpringBack));
            Some(stage)
        }

        /// Draws the leaf swung `open` (0 flat to 1 edge-on).
        fn draw(&self, stage: &Stage, open: f64) {
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
                    &format!("translateX({:.1}px)", -open * stage.w),
                );
                return;
            }
            let distance = FLIP_PERSPECTIVE * stage.w;
            let swing = flip::angle(open);
            let transform = format!("perspective({distance:.0}px) rotateY({:.3}rad)", -swing);
            el_style(&front, "transform", &transform);
            let (sheen, cast) = flip::lighting(swing);
            el_style(&fx, "visibility", "visible");
            el_style(&fx, "--flip-transform", &transform);
            el_style(
                &fx,
                "--flip-edge",
                &format!("{:.1}px", flip::edge_px(stage.w, swing, distance)),
            );
            el_style(&fx, "--flip-sheen", &format!("{sheen:.3}"));
            el_style(&fx, "--flip-cast", &format!("{cast:.3}"));
        }

        /// Steps the leaf's spring each frame, then ends the turn when it comes to rest.
        fn run(&self, flight: Flight) {
            let t = now();
            self.frame(Flight {
                began: t,
                last: t,
                ..flight
            });
        }

        fn frame(&self, mut flight: Flight) {
            let t = now();
            let done = match &mut flight.motion {
                Motion::Spring {
                    spring,
                    target,
                    pull,
                } => {
                    spring.step(*target, *pull, (t - flight.last) / 1000.0);
                    self.draw(&flight.stage, spring.x);
                    spring.at_rest(*target)
                }
                Motion::Glide { from, to, ms } => {
                    let share = ((t - flight.began) / *ms).clamp(0.0, 1.0);
                    self.draw(&flight.stage, *from + (*to - *from) * flip::glide(share));
                    share >= 1.0
                }
            };
            flight.last = t;
            if done {
                self.end(&flight.stage, flight.outcome);
            } else {
                let this = self.clone();
                request_animation_frame(move || this.frame(flight));
            }
        }

        /// Un-stacks the pages and shows the page that landed.
        fn end(&self, stage: &Stage, outcome: Settle) {
            if let Some(main) = self.0.main.get() {
                let panes = main.children();
                for index in 0..2 {
                    if let Some(pane) = panes.item(index) {
                        let _ = pane.class_list().remove_2("bend-front", "bend-under");
                        el_style(&pane, "transform", "");
                        el_style(&pane, "transform-origin", "");
                        el_style(&pane, "max-height", "");
                    }
                }
                // The landed page is named first, so the frame that un-stacks shows it.
                show_pane(&main, stage.turn.lands_on(outcome), self.0.set_pane);
                let _ = main.class_list().remove_2("bending", "flipping");
            }
            if let Some(fx) = self.0.fx.get() {
                el_style(&fx, "visibility", "hidden");
            }
            *self.0.phase.borrow_mut() = Phase::Idle;
        }
    }
}

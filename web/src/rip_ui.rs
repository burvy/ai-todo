//! The browser side of the page rip (`rip.rs` holds the rules): it reads the pointer on the
//! list page, cuts the hole in the paper, sends Rambo to tape it and lays the tape.
//!
//! On a phone the planner lies under the list page, hidden. While a tear lasts `main` is
//! marked `.torn`: the planner is shown beneath the list, which is paper and has the
//! slit cut out of it (`clip-path`), so what shows through is the live planner and not a
//! picture. The tape and the paper that heals the hole are drawn by `TearLayer`, above
//! both pages.

use leptos::prelude::*;

use crate::rip::{self, Tape, Tear};
use crate::tuning::{TAPE_AT_MS, TAPE_LAY_MS, TEAR_FADE_MS, TEAR_HOLD_MS};

/// The rip, as the page shares it: the tear (if any), how far the list has scrolled, and
/// whether the finger has let go (the repair is under way).
#[derive(Clone)]
pub(crate) struct Ripper {
    tear: RwSignal<Option<Tear>>,
    scroll: RwSignal<f64>,
    /// Where the list page sits in `main`: the tear is drawn in the page's coordinates.
    offset: RwSignal<(f64, f64)>,
    released: RwSignal<bool>,
    #[cfg(target_arch = "wasm32")]
    dom: std::rc::Rc<imp::Dom>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Ripper {
    /// There is no page to tear in a native build (the tests).
    pub(crate) fn new(
        _main: NodeRef<leptos::html::Main>,
        _pane: NodeRef<leptos::html::Div>,
        _pane_shown: ReadSignal<u8>,
        _state: ReadSignal<crate::mascot::State>,
        _set_state: WriteSignal<crate::mascot::State>,
        _pen_pos: RwSignal<(f64, f64)>,
    ) -> Self {
        Ripper {
            tear: RwSignal::new(None),
            scroll: RwSignal::new(0.0),
            offset: RwSignal::new((0.0, 0.0)),
            released: RwSignal::new(false),
        }
    }

    pub(crate) fn down(&self, _ev: &leptos::ev::PointerEvent) {}
    pub(crate) fn moved(&self, _ev: &leptos::ev::PointerEvent) {}
    pub(crate) fn up(&self, _ev: &leptos::ev::PointerEvent) {}
    pub(crate) fn scrolled(&self) {}
}

/// The tape, and the paper that heals the hole, over both pages. Nothing here takes the
/// pointer.
#[component]
pub(crate) fn TearLayer(ripper: Ripper) -> impl IntoView {
    let Ripper {
        tear,
        scroll,
        offset,
        released,
        ..
    } = ripper;
    view! {
        <div
            class="tear-layer"
            class:released=move || released.get()
            aria-hidden="true"
            style:--tape-at=format!("{TAPE_AT_MS}ms")
            style:--tape-lay=format!("{TAPE_LAY_MS}ms")
            style:--tear-hold=format!("{TEAR_HOLD_MS}ms")
            style:--tear-fade=format!("{TEAR_FADE_MS}ms")
        >
            {move || {
                tear.get().map(|tear| {
                    let (left, top) = offset.get();
                    let shift = format!("translate({left:.1}px, {:.1}px)", top - scroll.get());
                    let heal = rip::ring_path(&tear.outline());
                    view! {
                        <div class="tear-sheet" style:transform=shift>
                            <svg class="tear-heal" width="1" height="1">
                                <path class="tear-shade" d=heal.clone() />
                                <path class="tear-edge" d=heal.clone() />
                                <path class="tear-fill" d=heal />
                            </svg>
                            {tear.tapes().into_iter().map(tape).collect_view()}
                        </div>
                    }
                })
            }}
        </div>
    }
}

fn tape(tape: Tape) -> impl IntoView {
    view! {
        <span
            class="tape"
            style:left=format!("{:.1}px", tape.at.0)
            style:top=format!("{:.1}px", tape.at.1)
            style:width=format!("{:.1}px", tape.len)
            style:--tape-angle=format!("{:.1}deg", tape.angle_deg)
        ></span>
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use leptos::prelude::*;
    use wasm_bindgen::JsCast;
    use web_sys::Element;

    use super::Ripper;
    use crate::mascot::{self, Event, State};
    use crate::rip::{self, Tear};
    use crate::tuning::TEAR_SLIDE_MS;
    use crate::turn_util::{el_style, element_of, phone};

    /// What a press on the list page may not start a rip on: a task, or anything with a
    /// gesture or a tap of its own.
    const NOT_PAPER: &str = ".task-row, button, a, input, textarea, select, .task-dialog, \
        .scrap-corner, .scrap-pile, .paper-picker, .page-foot-rail";

    enum Phase {
        Idle,
        /// Down on bare paper: `y` is the level a tear would run along, in page coordinates.
        Pressed {
            id: i32,
            x: f64,
            y: f64,
            client_y: f64,
        },
        /// The finger is tearing.
        Tearing {
            id: i32,
        },
    }

    pub(super) struct Dom {
        pane: NodeRef<leptos::html::Div>,
        shown: ReadSignal<u8>,
        state: ReadSignal<State>,
        set_state: WriteSignal<State>,
        pen_pos: RwSignal<(f64, f64)>,
        phase: RefCell<Phase>,
        /// Counts tears, so each one's ragged edge is its own.
        made: Cell<u32>,
    }

    impl Ripper {
        pub(crate) fn new(
            main: NodeRef<leptos::html::Main>,
            pane: NodeRef<leptos::html::Div>,
            shown: ReadSignal<u8>,
            state: ReadSignal<State>,
            set_state: WriteSignal<State>,
            pen_pos: RwSignal<(f64, f64)>,
        ) -> Self {
            let ripper = Ripper {
                tear: RwSignal::new(None),
                scroll: RwSignal::new(0.0),
                offset: RwSignal::new((0.0, 0.0)),
                released: RwSignal::new(false),
                dom: Rc::new(Dom {
                    pane,
                    shown,
                    state,
                    set_state,
                    pen_pos,
                    phase: RefCell::new(Phase::Idle),
                    made: Cell::new(0),
                }),
            };
            // The page is torn while a tear lasts: the planner shows beneath, and the list,
            // paper, has the slit cut out of it.
            let (tear, scroll, offset) = (ripper.tear, ripper.scroll, ripper.offset);
            Effect::new(move |_| {
                let torn = tear.get();
                let scrolled = scroll.get();
                if let Some(main) = main.get() {
                    let _ = main.class_list().toggle_with_force("torn", torn.is_some());
                    el_style(&main, "--tear-slide-ms", &format!("{TEAR_SLIDE_MS}ms"));
                    // On a desk the planner is the other page of the spread, not under the
                    // list: for the length of a tear it slides under it, so the hole shows
                    // the live planner there too. On a phone it is under it already.
                    if let (Some(pane), Some(planner)) = (pane.get(), planner_in(&main)) {
                        let (tear_x, tear_y) = torn.as_ref().map_or((0.0, 0.0), |tear| {
                            (
                                f64::from(pane.offset_left()) + tear.from.0,
                                f64::from(pane.offset_top()) + tear.from.1,
                            )
                        });
                        let columns = columns_of(&planner);
                        let slide = rip::slide_px(
                            torn.is_some(),
                            phone(),
                            f64::from(planner.offset_left()),
                            f64::from(pane.offset_left()),
                            tear_x,
                            &columns,
                        );
                        let lift = rip::lift_px(
                            torn.is_some(),
                            phone(),
                            f64::from(planner.offset_top()),
                            tear_y,
                            &columns,
                        );
                        el_style(
                            &planner,
                            "transform",
                            &format!("translate({:.1}px, {lift:.1}px)", -slide),
                        );
                    }
                }
                if let Some(pane) = pane.get() {
                    let clip = torn.map_or_else(
                        || "none".to_owned(),
                        |tear| {
                            rip::sheet_path(
                                &tear,
                                f64::from(pane.client_width()),
                                f64::from(pane.client_height()),
                                scrolled,
                            )
                        },
                    );
                    el_style(&pane, "clip-path", &clip);
                    if torn_now(&tear) {
                        offset.set((f64::from(pane.offset_left()), f64::from(pane.offset_top())));
                    }
                }
            });
            ripper
        }

        /// Where the list page's content starts: its left and top on screen, less how far
        /// it is scrolled.
        fn origin(pane: &Element) -> (f64, f64) {
            let rect = pane.get_bounding_client_rect();
            (rect.left(), rect.top() - f64::from(pane.scroll_top()))
        }

        pub(crate) fn down(&self, ev: &leptos::ev::PointerEvent) {
            let dom = &self.dom;
            if !matches!(*dom.phase.borrow(), Phase::Idle)
                || !ev.is_primary()
                || ev.button() != 0
                || dom.shown.get_untracked() != 0
                || self.tear.with_untracked(Option::is_some)
            {
                return;
            }
            let (Some(pane), Some(target)) = (dom.pane.get(), element_of(ev)) else {
                return;
            };
            if !pane.contains(Some(&target)) || target.closest(NOT_PAPER).ok().flatten().is_some() {
                return;
            }
            let (left, top) = Self::origin(&pane);
            let (x, client_y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
            let y = client_y - top;
            let rows = rows_of(&pane, top);
            if let Some(y) = rip::lane(y, &rows) {
                *dom.phase.borrow_mut() = Phase::Pressed {
                    id: ev.pointer_id(),
                    x: x - left,
                    y,
                    client_y,
                };
            }
        }

        pub(crate) fn moved(&self, ev: &leptos::ev::PointerEvent) {
            let dom = &self.dom;
            let Some(pane) = dom.pane.get() else { return };
            let (left, _) = Self::origin(&pane);
            let x = f64::from(ev.client_x()) - left;
            let mut phase = dom.phase.borrow_mut();
            match &*phase {
                Phase::Pressed {
                    id,
                    x: x0,
                    y,
                    client_y,
                } if *id == ev.pointer_id() => {
                    let (dx, dy) = (x - x0, f64::from(ev.client_y()) - client_y);
                    if rip::is_rip(dx, dy) {
                        let seed = dom.made.get().wrapping_add(1);
                        dom.made.set(seed);
                        let tear = Tear {
                            from: (*x0, *y),
                            to: (*x0, *y),
                            seed: seed.wrapping_mul(2_654_435_761) ^ (*y as u32),
                        };
                        let _ = pane.set_pointer_capture(*id);
                        *phase = Phase::Tearing { id: *id };
                        drop(phase);
                        self.tear.set(Some(tear));
                        self.tear_to(&pane, x);
                    } else if dy.abs() > crate::tuning::GESTURE_SLOP_PX && dy.abs() > dx.max(0.0) {
                        // Going up or down, or back left: a scroll or a turn, not a rip.
                        *phase = Phase::Idle;
                    }
                }
                Phase::Tearing { id } if *id == ev.pointer_id() => {
                    drop(phase);
                    self.tear_to(&pane, x);
                }
                _ => {}
            }
        }

        /// The tear reaches `x` (the page's own coordinates): further each time, never
        /// back, and not into the pin's column or off the screen.
        fn tear_to(&self, pane: &Element, x: f64) {
            let pin = web_sys::window()
                .and_then(|window| window.get_computed_style(pane).ok().flatten())
                .and_then(|style| style.get_property_value("padding-right").ok())
                .and_then(|value| value.trim_end_matches("px").parse::<f64>().ok())
                .unwrap_or(0.0);
            let limit = rip::right_limit(f64::from(pane.client_width()), pin);
            self.tear.update(|tear| {
                if let Some(tear) = tear {
                    tear.to.0 = tear.to.0.max(x.min(limit)).max(tear.from.0);
                }
            });
        }

        pub(crate) fn up(&self, ev: &leptos::ev::PointerEvent) {
            let dom = &self.dom;
            let tearing = matches!(
                &*dom.phase.borrow(),
                Phase::Tearing { id } if *id == ev.pointer_id()
            );
            let pressed = matches!(
                &*dom.phase.borrow(),
                Phase::Pressed { id, .. } if *id == ev.pointer_id()
            );
            if pressed {
                *dom.phase.borrow_mut() = Phase::Idle;
            }
            if !tearing {
                return;
            }
            *dom.phase.borrow_mut() = Phase::Idle;
            let (Some(tear), Some(pane)) = (self.tear.get_untracked(), dom.pane.get()) else {
                return;
            };
            // Torn is torn: letting go starts the repair.
            self.released.set(true);
            let (left, top) = Self::origin(&pane);
            let (mx, my) = tear.middle();
            if dom.state.get_untracked() == State::Home {
                dom.pen_pos.set((left + mx, top + my));
                dom.set_state
                    .set(mascot::transition(&State::Home, Event::Mend));
            }
            let (tear_signal, released) = (self.tear, self.released);
            // On the engine's clock, with everything else that moves.
            crate::engine::every(f64::from(rip::total_ms()), move || {
                tear_signal.set(None);
                released.set(false);
                crate::engine::Flow::Stop
            });
        }

        pub(crate) fn scrolled(&self) {
            let Some(pane) = self.dom.pane.get() else {
                return;
            };
            self.scroll.set(f64::from(pane.scroll_top()));
            // The list moved under the tear: he stays on it while he tapes.
            if let (State::Mending, Some(tear)) =
                (self.dom.state.get_untracked(), self.tear.get_untracked())
            {
                let (left, top) = Self::origin(&pane);
                let (mx, my) = tear.middle();
                self.dom.pen_pos.set((left + mx, top + my));
            }
        }
    }

    /// The planner page in `main`.
    fn planner_in(main: &Element) -> Option<web_sys::HtmlElement> {
        main.query_selector(".planner-page")
            .ok()
            .flatten()
            .and_then(|planner| planner.dyn_into().ok())
    }

    /// The planner's day columns, measured from the planner page's left edge.
    fn columns_of(planner: &Element) -> Vec<rip::Column> {
        let page = planner.get_bounding_client_rect();
        let tops = |parts: web_sys::HtmlCollection| -> Vec<f64> {
            (0..parts.length())
                .filter_map(|i| parts.item(i))
                .map(|part| {
                    let rect = part.get_bounding_client_rect();
                    rect.top() - page.top()
                })
                .collect()
        };
        let days = planner.get_elements_by_class_name("day");
        (0..days.length())
            .filter_map(|i| days.item(i))
            .map(|day| {
                let rect = day.get_bounding_client_rect();
                let has_slips = day.get_elements_by_class_name("placement").length() > 0;
                // Its hour lines are what the hole lines up on: a slip may lie far from
                // the tear, and the planner should not be dragged across the screen for it.
                let marks = tops(day.get_elements_by_class_name("hour"));
                rip::Column {
                    left: rect.left() - page.left(),
                    width: rect.width(),
                    has_slips,
                    marks,
                }
            })
            .collect()
    }

    fn torn_now(tear: &RwSignal<Option<Tear>>) -> bool {
        tear.with_untracked(Option::is_some)
    }

    /// Each row's `(top, bottom)` in the page's coordinates, `top` being where the page's
    /// content starts on the screen.
    fn rows_of(pane: &Element, top: f64) -> Vec<(f64, f64)> {
        let rows = pane.get_elements_by_class_name("task-row");
        (0..rows.length())
            .filter_map(|i| rows.item(i))
            .map(|row| {
                let rect = row.get_bounding_client_rect();
                (rect.top() - top, rect.bottom() - top)
            })
            .collect()
    }
}

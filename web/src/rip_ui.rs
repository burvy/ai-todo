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
use crate::scrap::Spot;
use crate::tuning::{TAPE_AT_MS, TAPE_LAY_MS, TEAR_FADE_MS, TEAR_HOLD_MS};

thread_local! {
    /// A row that has just torn out of the page, finished: where it stood. The `Ripper` takes
    /// it and opens the hole.
    static ROW_OUT: RwSignal<Option<Departed>> = RwSignal::new(None);
    /// Whether a row's hole is open now: the rows below it wait where they were until it
    /// heals (`scrap::hold_rows`), so none slides into the hole.
    static HOLE_OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A row that has finished: its task, where it stood, and the row above it (see
/// [`rip::Waiting::anchor`]).
// Only the browser's `Ripper` reads it.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone)]
struct Departed {
    id: String,
    needs_done: bool,
    spot: Spot,
    anchor: Option<(String, f64)>,
}

/// A row, finished, tears out of the page where it stood (`spot`, in viewport px): the hole
/// shows the planner, Rambo tapes it, and the hole heals like any tear.
pub(crate) fn row_out(id: &str, spot: Spot, anchor: Option<(String, f64)>, needs_done: bool) {
    ROW_OUT.with(|row| {
        row.set(Some(Departed {
            id: id.to_owned(),
            needs_done,
            spot,
            anchor,
        }))
    });
}

/// Whether a row's hole is open: the rows below it hold their places until it heals.
pub(crate) fn hole_open() -> bool {
    HOLE_OPEN.with(std::cell::Cell::get)
}

/// The rip, as the page shares it: the tear (if any), how far the list has scrolled, and
/// whether the finger has let go (the repair is under way).
#[derive(Clone)]
pub(crate) struct Ripper {
    tear: RwSignal<Option<Tear>>,
    scroll: RwSignal<f64>,
    /// Where the list page sits in `main`: the tear is drawn in the page's coordinates.
    offset: RwSignal<(f64, f64)>,
    released: RwSignal<bool>,
    /// How far the lower part of the page has slid up to shut a row's hole, in px.
    closed: RwSignal<f64>,
    #[cfg(target_arch = "wasm32")]
    dom: std::rc::Rc<imp::Dom>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Ripper {
    /// There is no page to tear in a native build (the tests).
    pub(crate) fn new(
        _main: NodeRef<leptos::html::Main>,
        _pane: NodeRef<leptos::html::Div>,
        _snapshot: ReadSignal<todo_core::Snapshot>,
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
            closed: RwSignal::new(0.0),
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
        closed,
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
                    let tapes = tear.tapes();
                    let (shade, edge, fill) = (tear.clone(), tear.clone(), tear);
                    view! {
                        <div class="tear-sheet" style:transform=shift>
                            <svg class="tear-heal" width="1" height="1">
                                <path class="tear-shade" d=move || rip::ring_path(&shade.outline_closed(closed.get())) />
                                <path class="tear-edge" d=move || rip::ring_path(&edge.outline_closed(closed.get())) />
                                <path class="tear-fill" d=move || rip::ring_path(&fill.outline_closed(closed.get())) />
                            </svg>
                            {tapes.into_iter().map(tape).collect_view()}
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
            at: rip::Pt,
            client: rip::Pt,
            touch: bool,
        },
        /// The finger is tearing.
        Tearing {
            id: i32,
        },
    }

    pub(super) struct Dom {
        pane: NodeRef<leptos::html::Div>,
        snapshot: ReadSignal<todo_core::Snapshot>,
        shown: ReadSignal<u8>,
        state: ReadSignal<State>,
        set_state: WriteSignal<State>,
        pen_pos: RwSignal<(f64, f64)>,
        phase: RefCell<Phase>,
        /// Counts tears, so each one's ragged edge is its own.
        made: Cell<u32>,
        /// Rows that finished while a tear lasted: each opens its hole, where it stood, when
        /// the one before has healed.
        waiting: RefCell<rip::HoleQueue>,
    }

    /// The one place the page's slide-up is driven: `apply` is called every frame with how far
    /// the slide has come (`0.0` to `1.0`, eased: `rip::eased`) and `done` once it has arrived.
    /// A `duration` of 0 arrives at once. The motion lives behind this function, so it can move
    /// onto the shared physics module (`engine/body.rs`) without touching its callers.
    fn slide(duration: f64, mut apply: impl FnMut(f64) + 'static, done: impl FnOnce() + 'static) {
        let started = std::cell::Cell::new(None::<f64>);
        let mut done = Some(done);
        crate::engine::on_tick(move |tick| {
            let start = started.get().unwrap_or(tick.now);
            started.set(Some(start));
            let progress = if duration > 0.0 {
                ((tick.now - start) / duration).min(1.0)
            } else {
                1.0
            };
            apply(rip::eased(progress));
            if progress < 1.0 {
                return crate::engine::Flow::Go;
            }
            if let Some(done) = done.take() {
                done();
            }
            crate::engine::Flow::Stop
        });
    }

    impl Ripper {
        pub(crate) fn new(
            main: NodeRef<leptos::html::Main>,
            pane: NodeRef<leptos::html::Div>,
            snapshot: ReadSignal<todo_core::Snapshot>,
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
                closed: RwSignal::new(0.0),
                dom: Rc::new(Dom {
                    pane,
                    snapshot,
                    shown,
                    state,
                    set_state,
                    pen_pos,
                    phase: RefCell::new(Phase::Idle),
                    made: Cell::new(0),
                    waiting: RefCell::new(rip::HoleQueue::default()),
                }),
            };
            // The page is torn while a tear lasts: the planner shows beneath, and the list,
            // paper, has the slit cut out of it.
            let (tear, scroll, offset, closed) =
                (ripper.tear, ripper.scroll, ripper.offset, ripper.closed);
            // The part of the planner that has just slid under the hole can be left unpainted
            // (the day lines were missing from the hole on a desk) until the page's style
            // changes again: when the slide ends, a changed custom property on `main` makes
            // the browser paint it.
            let settled = Cell::new(0_u32);
            let _ = window_event_listener(leptos::ev::transitionend, move |ev| {
                let on_planner = ev
                    .target()
                    .and_then(|target| target.dyn_into::<Element>().ok())
                    .is_some_and(|el| el.class_list().contains("planner-page"));
                if on_planner && ev.property_name() == "transform" {
                    settled.set(settled.get().wrapping_add(1));
                    if let Some(main) = main.get_untracked() {
                        el_style(&main, "--planner-settled", &settled.get().to_string());
                    }
                }
            });
            // A list that fits its page has nothing to scroll, so a finger may tear it up
            // or down: the browser is told so (`.fits`), or it would take the drag.
            Effect::new(move |_| {
                if let Some(pane) = pane.get() {
                    keep_fit(&pane);
                }
            });
            // A finished row tears out of the page where it stood.
            let row_out = super::ROW_OUT.with(|row| *row);
            Effect::new({
                let ripper = ripper.clone();
                move |_| {
                    if let Some(super::Departed {
                        id,
                        needs_done,
                        spot,
                        anchor,
                    }) = row_out.get()
                    {
                        row_out.set(None);
                        ripper.tear_out(id, needs_done, spot, anchor);
                    }
                }
            });
            Effect::new(move |_| {
                let torn = tear.get();
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
                        // A hole as wide as a row has the whole grid under it; a slit, the
                        // column that has something to show under where it starts.
                        let planner_left = f64::from(planner.offset_left());
                        let slide = match torn.as_ref() {
                            Some(tear) if tear.profile != rip::Profile::Slit => rip::slide_over(
                                planner_left,
                                f64::from(pane.offset_left()) + tear.from.0,
                                f64::from(pane.offset_left()) + tear.to.0,
                                &columns,
                            ),
                            _ => rip::slide_px(
                                torn.is_some(),
                                phone(),
                                planner_left,
                                f64::from(pane.offset_left()),
                                tear_x,
                                &columns,
                            ),
                        };
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
            });
            // The hole cut in the list page, drawn again as the lower part slides shut.
            Effect::new(move |_| {
                let torn = tear.get();
                let scrolled = scroll.get();
                let shut = closed.get();
                if let Some(pane) = pane.get() {
                    let clip = torn.map_or_else(
                        || "none".to_owned(),
                        |tear| {
                            rip::sheet_path(
                                &tear,
                                shut,
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
            let client = (f64::from(ev.client_x()), f64::from(ev.client_y()));
            let at = (client.0 - left, client.1 - top);
            if rip::starts_on_paper(at, &rows_of(&pane, left, top)) {
                *dom.phase.borrow_mut() = Phase::Pressed {
                    id: ev.pointer_id(),
                    at,
                    client,
                    touch: ev.pointer_type() == "touch",
                };
            }
        }

        pub(crate) fn moved(&self, ev: &leptos::ev::PointerEvent) {
            let dom = &self.dom;
            let Some(pane) = dom.pane.get() else { return };
            let (left, top) = Self::origin(&pane);
            let client = (f64::from(ev.client_x()), f64::from(ev.client_y()));
            let finger = (client.0 - left, client.1 - top);
            let mut phase = dom.phase.borrow_mut();
            match &*phase {
                Phase::Pressed {
                    id,
                    at,
                    client: down,
                    touch,
                } if *id == ev.pointer_id() => {
                    let (dx, dy) = (client.0 - down.0, client.1 - down.1);
                    let scrolls = scrolls(&pane);
                    if dx.hypot(dy) > crate::tuning::GESTURE_SLOP_PX
                        && rip::is_page_gesture(dx, dy, *touch, scrolls)
                    {
                        // The page's own gesture: a turn, or a scroll.
                        *phase = Phase::Idle;
                    } else if rip::is_rip(dx, dy) {
                        let seed = dom.made.get().wrapping_add(1);
                        dom.made.set(seed);
                        let tear =
                            Tear::slit(*at, *at, seed.wrapping_mul(2_654_435_761) ^ (at.1 as u32));
                        let _ = pane.set_pointer_capture(*id);
                        *phase = Phase::Tearing { id: *id };
                        drop(phase);
                        self.tear.set(Some(tear));
                        self.tear_to(&pane, finger);
                    }
                }
                Phase::Tearing { id } if *id == ev.pointer_id() => {
                    drop(phase);
                    self.tear_to(&pane, finger);
                }
                _ => {}
            }
        }

        /// The tear follows the finger (`finger`, in the page's own coordinates): further
        /// each time, never back, in whatever direction it goes, and it stops where it
        /// would touch a task's words, the pin's column or the edge of the page.
        fn tear_to(&self, pane: &Element, finger: rip::Pt) {
            let pin = web_sys::window()
                .and_then(|window| window.get_computed_style(pane).ok().flatten())
                .and_then(|style| style.get_property_value("padding-right").ok())
                .and_then(|value| value.trim_end_matches("px").parse::<f64>().ok())
                .unwrap_or(0.0);
            let (left, top) = Self::origin(pane);
            let bounds = (
                crate::tuning::RIP_EDGE_PX,
                crate::tuning::RIP_EDGE_PX,
                rip::right_limit(f64::from(pane.client_width()), pin),
                f64::from(pane.scroll_height().max(pane.client_height()))
                    - crate::tuning::RIP_EDGE_PX,
            );
            let rows = rows_of(pane, left, top);
            self.tear.update(|tear| {
                if let Some(tear) = tear {
                    tear.follow(rip::reach_towards(tear.from, finger, &rows, bounds));
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
            let Some(tear) = self.tear.get_untracked() else {
                return;
            };
            // Torn is torn, and it vexes him: letting go starts the repair.
            self.mend(&tear, true);
        }

        /// Whether a waiting row's hole is still due: it has not come back to the list
        /// (taped back, refused), and a finished row's task is still done (one deleted from
        /// the pin while its row waited is gone from the snapshot and has no hole).
        fn still_due(&self, waiting: &rip::Waiting) -> bool {
            let done = !waiting.needs_done
                || self.dom.snapshot.with_untracked(|snapshot| {
                    snapshot
                        .tasks
                        .iter()
                        .any(|t| t.id.0 == waiting.id && t.done)
                });
            done && !crate::scrap::row_on_page(&waiting.id)
        }

        /// The page has no list to tear: forget the rows waiting and let the held rows go.
        fn give_up(&self) {
            self.dom.waiting.borrow_mut().clear();
            if super::HOLE_OPEN.with(|open| open.replace(false)) {
                crate::scrap::release_rows();
            }
        }

        /// A row that has finished (`id`, standing at `spot`, in viewport px) tears out of
        /// the page where it stood: the hole is as tall as the row, and Rambo tapes it
        /// without a sulk. One tear at a time: a row that finishes while one lasts waits its
        /// turn (the first few; one beyond that just leaves), and the rows below stay where
        /// they were until the last hole has healed, so each hole opens where its row stood.
        fn tear_out(
            &self,
            id: String,
            needs_done: bool,
            spot: crate::scrap::Spot,
            anchor: Option<(String, f64)>,
        ) {
            let dom = &self.dom;
            let Some(pane) = dom.pane.get() else {
                return self.give_up();
            };
            let (left, top) = Self::origin(&pane);
            let waiting = rip::Waiting {
                id,
                needs_done,
                place: (
                    spot.x - left,
                    spot.y - top,
                    spot.x + spot.w - left,
                    spot.y + spot.h - top,
                ),
                anchor,
            };
            if self.tear.with_untracked(Option::is_some) {
                super::HOLE_OPEN.with(|open| open.set(true));
                dom.waiting.borrow_mut().push(waiting);
                return;
            }
            self.open_hole(&waiting);
        }

        /// Open the hole a row leaves, from its place in the page's coordinates.
        fn open_hole(&self, waiting: &rip::Waiting) {
            let dom = &self.dom;
            // Where its row stood, measured again from the row above it as it is now.
            let anchor_top = waiting.anchor.as_ref().and_then(|(id, _)| {
                let pane = dom.pane.get_untracked()?;
                let (_, top) = Self::origin(&pane);
                Some(crate::scrap::row_top_in_view(id)? - top)
            });
            let (_, top, _, bottom) = waiting.place_now(anchor_top);
            let Some(pane) = dom.pane.get_untracked() else {
                return self.give_up();
            };
            let seed = dom.made.get().wrapping_add(1);
            dom.made.set(seed);
            // The page tears right across at the row.
            let tear = Tear::band(
                f64::from(pane.client_width()),
                (top + bottom) / 2.0,
                bottom - top,
                seed.wrapping_mul(2_654_435_761) ^ (top as u32),
            );
            self.closed.set(0.0);
            super::HOLE_OPEN.with(|open| open.set(true));
            self.tear.set(Some(tear.clone()));
            // It lies open a moment, then the lower part of the page slides up to shut it
            // and Rambo tapes the join.
            let open_ms = if crate::mascot::prefers_reduced_motion() {
                1
            } else {
                crate::tuning::RIP_BAND_OPEN_MS
            };
            let ripper = self.clone();
            crate::engine::every(f64::from(open_ms), move || {
                ripper.close_hole(&tear, top, bottom);
                crate::engine::Flow::Stop
            });
        }

        /// The lower part of the page slides up (the rows below with it) until the two torn
        /// lips of the hole meet; then he tapes the seam.
        fn close_hole(&self, tear: &Tear, top: f64, bottom: f64) {
            let Some(pane) = self.dom.pane.get_untracked() else {
                return self.give_up();
            };
            let (_, origin_top) = Self::origin(&pane);
            let closing = crate::scrap::begin_close(origin_top + top, origin_top + bottom);
            let (gap, closed) = (tear.gap, self.closed);
            let duration = if crate::mascot::prefers_reduced_motion() {
                0.0
            } else {
                f64::from(crate::tuning::RIP_BAND_CLOSE_MS)
            };
            let moved = std::cell::Cell::new(0.0);
            let (ripper, tear) = (self.clone(), tear.clone());
            slide(
                duration,
                move |eased| {
                    let to = closing.amount * eased;
                    crate::scrap::step_close(&closing, to - moved.get());
                    moved.set(to);
                    closed.set(if closing.amount > 0.0 {
                        to.min(gap)
                    } else {
                        gap * eased
                    });
                },
                move || {
                    closed.set(gap);
                    ripper.mend(&tear, false);
                },
            );
        }

        /// The repair: Rambo goes to the tear (put out by it, or not), tapes it, and the tear
        /// heals on the engine's clock. A row's hole lets the rows below close it then.
        fn mend(&self, tear: &Tear, annoyed: bool) {
            let dom = &self.dom;
            let Some(pane) = dom.pane.get() else { return };
            self.released.set(true);
            let (left, top) = Self::origin(&pane);
            let (mx, my) = tear.middle();
            if dom.state.get_untracked() == State::Home {
                dom.pen_pos.set((left + mx, top + my));
                dom.set_state
                    .set(mascot::transition(&State::Home, Event::Mend { annoyed }));
            }
            let (tear_signal, released) = (self.tear, self.released);
            let ripper = self.clone();
            // On the engine's clock, with everything else that moves.
            crate::engine::every(f64::from(rip::total_ms()), move || {
                tear_signal.set(None);
                released.set(false);
                // A row waiting for its turn opens its hole now (unless it is back on the
                // page, undone); the rows below close up only when nothing is waiting.
                if ripper.dom.pane.get_untracked().is_none() {
                    ripper.give_up();
                } else {
                    let next = ripper
                        .dom
                        .waiting
                        .borrow_mut()
                        .next_due(|waiting| ripper.still_due(waiting));
                    match next {
                        Some(waiting) => ripper.open_hole(&waiting),
                        None => ripper.give_up(),
                    }
                }
                crate::engine::Flow::Stop
            });
        }

        pub(crate) fn scrolled(&self) {
            let Some(pane) = self.dom.pane.get() else {
                return;
            };
            self.scroll.set(f64::from(pane.scroll_top()));
            // The list moved under the tear: he stays on it while he tapes.
            if let (State::Mending { .. }, Some(tear)) =
                (self.dom.state.get_untracked(), self.tear.get_untracked())
            {
                let (left, top) = Self::origin(&pane);
                let (mx, my) = tear.middle();
                self.dom.pen_pos.set((left + mx, top + my));
            }
        }
    }

    /// Whether the list page has anything to scroll: it, the `main` around it or the document
    /// is a scroll box with more content than room (which it is depends on the layout).
    fn scrolls(pane: &Element) -> bool {
        let scroll_box = |el: &Element| {
            let overflows = el.scroll_height() > el.client_height();
            let auto = web_sys::window()
                .and_then(|window| window.get_computed_style(el).ok().flatten())
                .and_then(|style| style.get_property_value("overflow-y").ok())
                .is_some_and(|value| matches!(value.as_str(), "auto" | "scroll"));
            overflows && auto
        };
        // On a desk the page itself can scroll, as the document.
        let document_scrolls = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.scrolling_element())
            .is_some_and(|root| root.scroll_height() > root.client_height());
        scroll_box(pane)
            || pane.parent_element().is_some_and(|main| scroll_box(&main))
            || document_scrolls
    }

    /// Keep `.fits` on the list page while its content fits it: it lets a finger drag
    /// anywhere on it (theme.css), as there is nothing to scroll. Checked when the page or
    /// its list changes size.
    fn keep_fit(pane: &Element) {
        let page = pane.clone();
        let check = wasm_bindgen::closure::Closure::<dyn Fn()>::new(move || {
            let fits = !scrolls(&page);
            let _ = page.class_list().toggle_with_force("fits", fits);
        });
        if let Ok(observer) = web_sys::ResizeObserver::new(check.as_ref().unchecked_ref()) {
            observer.observe(pane);
            if let Ok(Some(list)) = pane.query_selector(".task-list") {
                observer.observe(&list);
            }
        }
        // The page lives as long as the app, and so does its watcher.
        check.forget();
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
        // The middle of each part, as a height from the planner page's top.
        let middles = |parts: web_sys::HtmlCollection| -> Vec<f64> {
            (0..parts.length())
                .filter_map(|i| parts.item(i))
                .map(|part| {
                    let rect = part.get_bounding_client_rect();
                    rect.top() - page.top() + rect.height() / 2.0
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
                let slips = middles(day.get_elements_by_class_name("placement"));
                rip::Column {
                    left: rect.left() - page.left(),
                    width: rect.width(),
                    has_slips,
                    marks,
                    slips,
                }
            })
            .collect()
    }

    fn torn_now(tear: &RwSignal<Option<Tear>>) -> bool {
        tear.with_untracked(Option::is_some)
    }

    /// Each row's box in the page's coordinates, `left` and `top` being where the page's
    /// content starts on the screen.
    fn rows_of(pane: &Element, left: f64, top: f64) -> Vec<rip::Rect> {
        let rows = pane.get_elements_by_class_name("task-row");
        (0..rows.length())
            .filter_map(|i| rows.item(i))
            .map(|row| {
                let rect = row.get_bounding_client_rect();
                (
                    rect.left() - left,
                    rect.top() - top,
                    rect.right() - left,
                    rect.bottom() - top,
                )
            })
            .collect()
    }
}

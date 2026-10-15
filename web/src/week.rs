use chrono::{DateTime, Timelike, Utc};
use leptos::portal::Portal;
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use todo_core::When;
#[cfg(not(target_arch = "wasm32"))]
use todo_core::{Clock, SystemClock};
use todo_core::{LocalContext, Placement, Snapshot, TaskId};

use crate::tuning::{HOUR_PX, SHORT_SLIP_PX};
use crate::{clock, settings::Settings};
pub(crate) use rambl_web_core::planner::*;

/// A task being dragged by its grip: who it is, where the finger is, and where on
/// the slip it took hold. Where it may land is drawn from `landing`; the drop's
/// `updated_at` guard is looked up from the snapshot when the slip lands, so a
/// still fresh edit always wins.
#[derive(Clone, Debug)]
struct Drag {
    id: TaskId,
    title: String,
    x: f64,
    y: f64,
    /// The finger's offset from the slip's top-left corner when it was lifted.
    grab: (f64, f64),
}

impl Drag {
    /// The ghost's top-left corner in viewport pixels: the slip stays under the
    /// finger at the very spot it was taken.
    fn ghost_corner(&self) -> (f64, f64) {
        (self.x - self.grab.0, self.y - self.grab.1)
    }
}

/// How far a phone finger must hold still before its slip lifts onto it, so
/// a plain vertical drag still scrolls the paper (the mouse lifts at once).
#[cfg(target_arch = "wasm32")]
const PRESS_HOLD_MS: u32 = 250;

/// How far a not-yet-lifted finger may slide before we call it a scroll and
/// disarm the lift.
#[cfg(target_arch = "wasm32")]
const PRESS_SLOP_PX: f64 = 10.0;

#[component]
pub fn WeekGrid(
    snapshot: ReadSignal<Snapshot>,
    settings: ReadSignal<Settings>,
    set_settings: WriteSignal<Settings>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
    capture_text: RwSignal<String>,
    /// Rambo's state: dazed, every hold on the page is his, so no slip lifts.
    mascot_state: ReadSignal<crate::mascot::State>,
    /// Turn to the list's page: what the note at this page's foot does.
    turn_back: std::rc::Rc<dyn Fn()>,
    /// A task Rambo was asked to move: it waits for a tap on an hour.
    move_request: RwSignal<Option<TaskId>>,
) -> impl IntoView {
    let grid_ref = NodeRef::<leptos::html::Div>::new();
    let (now_minutes, set_now_minutes) = signal(current_minutes());
    crate::engine::every(60_000.0, move || {
        set_now_minutes.set(current_minutes());
        crate::engine::Flow::Go
    });
    // The planner opens on the working morning, not in the middle of the
    // night: about 08:00, or now if it is already later. Once, not on every
    // settings change: a pen swap must not jump the paper back up.
    let opened = RwSignal::new(false);
    Effect::new(move |_| {
        if opened.get() {
            return;
        }
        let s = settings.get();
        let Some(grid) = grid_ref.get() else {
            return;
        };
        #[cfg(target_arch = "wasm32")]
        {
            let target = planner_open_minutes(current_minutes(), s.start_hour, s.end_hour);
            grid.set_scroll_top(now_line_top(target, s.start_hour, HOUR_PX) as i32);
            opened.set(true);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (grid, s);
    });
    // The paper slip: a dragged task and where the finger is.
    let drag = RwSignal::new(None::<Drag>);
    // Where it would land: day + minutes, drawn as a faint slot outline.
    let landing = RwSignal::new(None::<Slot>);
    // Esc puts a slip in hand back down: cancel, never commit.
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast as _;
        let esc_key = std::rc::Rc::new(std::cell::RefCell::new(
            None::<wasm_bindgen::closure::Closure<dyn Fn(leptos::ev::KeyboardEvent)>>,
        ));
        Effect::new(move |_| {
            if drag.get().is_none() {
                if let Some(key) = esc_key.borrow_mut().take() {
                    let _ = leptos::prelude::document().remove_event_listener_with_callback(
                        "keydown",
                        key.as_ref().unchecked_ref(),
                    );
                }
                return;
            }
            if esc_key.borrow().is_some() {
                return;
            }
            let key = wasm_bindgen::closure::Closure::<dyn Fn(leptos::ev::KeyboardEvent)>::wrap(
                Box::new(move |ev: leptos::ev::KeyboardEvent| {
                    if ev.key() != "Escape" {
                        return;
                    }
                    // Cancel, never commit: no change is written anywhere.
                    drag.set(None);
                    landing.set(None);
                }),
            );
            let _ = leptos::prelude::document()
                .add_event_listener_with_callback("keydown", key.as_ref().unchecked_ref());
            *esc_key.borrow_mut() = Some(key);
        });
    }
    // While a slip is held near an edge, the paper keeps sliding along under
    // the finger, so a slot beyond the edge can be reached without letting go.
    // The slide runs only while a slip is held, so the loop sleeps otherwise.
    let edge_sliding = std::rc::Rc::new(std::cell::Cell::new(false));
    Effect::new(move |_| {
        if drag.with(Option::is_none) || edge_sliding.replace(true) {
            return;
        }
        let edge_sliding = edge_sliding.clone();
        crate::engine::every(50.0, move || {
            if drag.with_untracked(Option::is_none) {
                edge_sliding.set(false);
                return crate::engine::Flow::Stop;
            }
            #[cfg(target_arch = "wasm32")]
            {
                let Some(d) = drag.get_untracked() else {
                    return crate::engine::Flow::Go;
                };
                let Some(scroller) = grid_ref.get() else {
                    return crate::engine::Flow::Go;
                };
                let rect = scroller.get_bounding_client_rect();
                let down = edge_nudge(rect.bottom() as f32 - d.y as f32);
                let up = edge_nudge(d.y as f32 - rect.top() as f32);
                let right = edge_nudge(rect.right() as f32 - d.x as f32);
                let left = edge_nudge(d.x as f32 - rect.left() as f32);
                let (dx, dy) = (right - left, down - up);
                if dx == 0.0 && dy == 0.0 {
                    return crate::engine::Flow::Go;
                }
                // scroll_left/top clamp inside the bounds themselves.
                if dy != 0.0 {
                    scroller.set_scroll_top(scroller.scroll_top() - dy as i32);
                }
                if dx != 0.0 {
                    scroller.set_scroll_left(scroller.scroll_left() - dx as i32);
                }
                // The paper under the finger has moved: re-find its slot.
                landing.set(landing_at(d.x, d.y, &settings.get_untracked()));
            }
            crate::engine::Flow::Go
        });
    });
    #[cfg(not(target_arch = "wasm32"))]
    let _ = capture_text;
    // On the earliest day a right swipe has no days left to scroll, so the page takes it
    // (turner.rs turns back to the list); the class tells the browser to let it through.
    let (at_earliest, set_at_earliest) = signal(true);
    // The bookmark ribbon's head (ribbon.rs) sticks out of the day names at today's
    // column, or at the edge today went past, kept up with the paper as it scrolls; its
    // tail lies on today's hours, under the slips.
    let (ribbon_left, set_ribbon_left) = signal(0.0_f64);
    let (ribbon_top, set_ribbon_top) = signal(0.0_f64);
    let (ribbon_height, set_ribbon_height) = signal(0.0_f64);
    let hang_ribbon = move || {
        #[cfg(target_arch = "wasm32")]
        if let Some((left, top, height)) =
            grid_ref.get_untracked().and_then(|grid| ribbon_spot(&grid))
        {
            set_ribbon_left.set(left);
            set_ribbon_top.set(top);
            set_ribbon_height.set(height);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (set_ribbon_left, set_ribbon_top, set_ribbon_height);
    };
    // It finds today again whenever the days change, and whenever the paper changes
    // size: laid out, shown by a phone's page turn, or the window resized.
    Effect::new(move |_| {
        let _ = settings.with(|s| s.days);
        crate::engine::next_frame(hang_ribbon);
    });
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        if let Some(grid) = grid_ref.get() {
            on_resized(&grid, hang_ribbon);
        }
    });
    // Pulled, the paper slides back to today, at the hour the planner opens on.
    let back_to_today = Callback::new(move |()| {
        #[cfg(target_arch = "wasm32")]
        if let Some(grid) = grid_ref.get_untracked() {
            let s = settings.get_untracked();
            let opens = planner_open_minutes(current_minutes(), s.start_hour, s.end_hour);
            let top = f64::from(now_line_top(opens, s.start_hour, HOUR_PX));
            slide_to(grid.into(), (0.0, top));
        }
    });
    let on_scroll = move |_: leptos::ev::Event| {
        #[cfg(target_arch = "wasm32")]
        if let Some(grid) = grid_ref.get() {
            set_at_earliest.set(grid.scroll_left() <= 1);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = set_at_earliest;
        hang_ribbon();
    };
    // The waiting task lands where it is told, and the planner stops waiting.
    let place = Callback::new(move |slot: Slot| {
        let Some(id) = move_request.get_untracked() else {
            return;
        };
        move_request.set(None);
        landing.set(None);
        #[cfg(target_arch = "wasm32")]
        place_at(&id, slot, snapshot, set_snapshot, set_error);
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (id, slot);
    });
    // While a task waits, the hour under the pointer is where a tap puts it: outlined
    // under a mouse, placed on the tap. A held slip keeps its own outline.
    let waiting_slot = move |ev: &leptos::ev::PointerEvent| {
        if move_request.with_untracked(Option::is_none) || drag.with_untracked(Option::is_some) {
            return None;
        }
        #[cfg(target_arch = "wasm32")]
        return hour_at(
            f64::from(ev.client_x()),
            f64::from(ev.client_y()),
            &settings.get_untracked(),
        );
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ev;
            None
        }
    };
    let days = move || (0..settings.get().days).collect::<Vec<_>>();
    let hours = move || (settings.get().start_hour..settings.get().end_hour).collect::<Vec<_>>();
    let ctx = move || planner_context(&snapshot.get().tz);
    view! {
        <section class="planner-page" aria-label="Planner">
            // The settings gear and the account link live on this page only, at its top right.
            <div class="planner-corner">
                <crate::account::HeaderLink />
                <crate::settings::GearMenu settings set_settings set_snapshot set_error />
            </div>
            <crate::PageFoot page=1 turn=turn_back />
            <crate::placing::PlacingNote snapshot settings move_request place />
            <crate::ribbon::Ribbon
                left=ribbon_left
                top=ribbon_top
                height=ribbon_height
                back=back_to_today
            />
            <div
                class="week-scroll"
                class:at-earliest=at_earliest
                node_ref=grid_ref
                on:scroll=on_scroll
            >
                <div class="week" style=move || format!("--days: {}", settings.get().days)>
                    <For each=days key=|day| *day let:day>
                        <div class="day" data-day=day>
                            <h3 class="day-name" class:today=day == 0>
                                {move || {
                                    // Re-read every minute, so the header moves on at midnight.
                                    let _ = now_minutes.get();
                                    day_label(&ctx(), i64::from(day))
                                }}
                            </h3>
                            <div
                                class="hours"
                                class:placing=move || move_request.with(Option::is_some)
                                on:pointermove=move |ev| {
                                    if ev.pointer_type() == "mouse"
                                        && let Some(slot) = waiting_slot(&ev)
                                    {
                                        landing.set(Some(slot));
                                    }
                                }
                                on:pointerleave=move |_| {
                                    if move_request.with_untracked(Option::is_some)
                                        && drag.with_untracked(Option::is_none)
                                    {
                                        landing.set(None);
                                    }
                                }
                                on:pointerup=move |ev| {
                                    if let Some(slot) = waiting_slot(&ev) {
                                        place.run(slot);
                                    }
                                }
                            >
                                <Show
                                    when=move || {
                                        landing.get().is_some_and(|slot| slot.day == i64::from(day))
                                    }
                                >
                                    {move || view! {
                                            <div
                                                class="landing"
                                                style=move || {
                                                    let Some(slot) = landing.get() else {
                                                        return "top: 0px; height: 0px;".to_owned();
                                                    };
                                                    let s = settings.get();
                                                    let (top, height) = placement_style(
                                                        slot.minutes,
                                                        slot.minutes + 30,
                                                        s.start_hour,
                                                        HOUR_PX,
                                                        s.end_hour,
                                                    );
                                                    format!("top: {top}px; height: {height}px;")
                                                }
                                                aria-hidden="true"
                                            ></div>
                                    }}
                                </Show>
                                {(day == 0).then(|| view! {
                                    <crate::ribbon::RibbonTail />
                                })}
                                {move || (day == 0).then(|| view! {
                                    <div
                                        class="now-line"
                                        style=move || format!(
                                            "top: {}px",
                                            now_line_top(now_minutes.get(), settings.get().start_hour, HOUR_PX)
                                        )
                                        aria-label="Current time"
                                    ></div>
                                })}
                            <For
                                each=move || {
                                    // Re-read every minute, so "today" moves on at midnight.
                                    let _ = now_minutes.get();
                                    let snapshot = snapshot.get();
                                    let ctx = planner_context(&snapshot.tz);
                                    placements_on_day(&snapshot.plan.placements, i64::from(day), &ctx)
                                }
                                key=placement_key let:placement>
                                <PlacementBlock placement snapshot settings set_snapshot set_error drag landing mascot_state />
                            </For>
                                <For each=hours key=|hour| *hour let:hour>
                                    <div
                                        class="hour"
                                        // Long-press an empty hour and the capture box is
                                        // pre-filled with that slot, ready to type the task.
                                        on:pointerdown=long_press_start(capture_text, snapshot, i64::from(day), hour)
                                        on:pointermove=long_press_move
                                        on:pointerup=long_press_stop
                                        on:pointerleave=long_press_stop
                                        on:pointercancel=long_press_stop
                                    >
                                        {(day == 0).then(|| view! {
                                            <span>{move || clock::format_time(
                                                u16::from(hour) * 60,
                                                settings.get().clock,
                                            )}</span>
                                        })}
                                    </div>
                                </For>
                            </div>
                        </div>
                    </For>
                </div>
            </div>
            // The ghost is fixed to the viewport, so it lives on the body: the planner
            // page carries a transform (its slide), and a fixed child of a transformed
            // box is placed against that box, far off the pointer.
            <Portal>
            {move || drag.get().map(|d| view! {
                <div
                    class="drag-ghost"
                    style={
                        let (left, top) = d.ghost_corner();
                        format!("left: {left:.1}px; top: {top:.1}px;")
                    }
                    aria-hidden="true"
                >
                    <strong>{d.title.clone()}</strong>
                    // The time the slip would get, live as the finger moves it.
                    <span class="ghost-time">
                        {move || {
                            landing.get().map(|slot| {
                                let ctx = planner_context(&snapshot.get().tz);
                                let weekday = (ctx.today() + chrono::Duration::days(slot.day))
                                    .format("%a")
                                    .to_string();
                                clock::format_day_time(&weekday, slot.minutes, settings.get().clock)
                            })
                        }}
                    </span>
                </div>
            })}
            </Portal>
        </section>
    }
}

/// The hour cell a pointer event landed in: `.hour` from the pointer's
/// target, so a stamp never lands on a child element by mistake.
#[cfg(target_arch = "wasm32")]
fn press_cell(event: &leptos::ev::PointerEvent) -> Option<web_sys::HtmlElement> {
    use wasm_bindgen::JsCast;
    event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| el.closest(".hour").ok().flatten())
        .and_then(|cell| cell.dyn_into::<web_sys::HtmlElement>().ok())
}

/// Arms a long-press on one empty hour cell. Holding still for
/// `LONG_PRESS_MS` pre-fills the capture box with that slot and focuses it;
/// any release, leave, cancel or move unstamps the cell quietly.
#[cfg(target_arch = "wasm32")]
fn long_press_start(
    capture_text: RwSignal<String>,
    snapshot: ReadSignal<Snapshot>,
    day: i64,
    hour: u8,
) -> impl Fn(leptos::ev::PointerEvent) + 'static {
    move |event: leptos::ev::PointerEvent| {
        let Some(cell) = press_cell(&event) else {
            return;
        };
        let _ = cell.set_attribute("data-press", "armed");
        gloo_timers::callback::Timeout::new(LONG_PRESS_MS, move || {
            use wasm_bindgen::JsCast;
            let armed = cell.get_attribute("data-press").as_deref() == Some("armed");
            let _ = cell.remove_attribute("data-press");
            if !armed {
                return;
            }
            let ctx = planner_context(&snapshot.get_untracked().tz);
            capture_text.set(hour_prefill_text(&ctx, day, u16::from(hour) * 60));
            if let Some(input) = leptos::prelude::document()
                .query_selector(".capture input")
                .ok()
                .flatten()
                .and_then(|input| input.dyn_into::<web_sys::HtmlInputElement>().ok())
            {
                let _ = input.focus();
            }
        })
        .forget();
    }
}

/// Any move unstamps the held cell (a no-op when nothing is held).
#[cfg(target_arch = "wasm32")]
fn long_press_move(event: leptos::ev::PointerEvent) {
    if let Some(cell) = press_cell(&event) {
        let _ = cell.remove_attribute("data-press");
    }
}

/// Any release, leave or cancel: no pre-fill.
#[cfg(target_arch = "wasm32")]
fn long_press_stop(event: leptos::ev::PointerEvent) {
    if let Some(cell) = press_cell(&event) {
        let _ = cell.remove_attribute("data-press");
    }
}

/// How long an empty hour must be held before the capture box pre-fills.
#[cfg(target_arch = "wasm32")]
const LONG_PRESS_MS: u32 = 600;

// On a native build (tests, tooling) the hour cells do not recognise a
// pointer: the page only exists in a browser.
#[cfg(not(target_arch = "wasm32"))]
fn long_press_start(
    _capture_text: RwSignal<String>,
    _snapshot: ReadSignal<Snapshot>,
    _day: i64,
    _hour: u8,
) -> impl Fn(leptos::ev::PointerEvent) + 'static {
    move |_: leptos::ev::PointerEvent| {}
}

#[cfg(not(target_arch = "wasm32"))]
fn long_press_move(_: leptos::ev::PointerEvent) {}

#[cfg(not(target_arch = "wasm32"))]
fn long_press_stop(_: leptos::ev::PointerEvent) {}

/// Where the bookmark ribbon's head sticks out on the planner page (its left, top and
/// height, in px from the page's top left): in the band of day names, just inside
/// today's column, whose name is written at its left. With today scrolled away it waits
/// in the first column still in sight, at the same spot, so it covers no name.
#[cfg(target_arch = "wasm32")]
fn ribbon_spot(grid: &web_sys::HtmlDivElement) -> Option<(f64, f64, f64)> {
    let page_el = grid.closest(".planner-page").ok()??;
    let page = page_el.get_bounding_client_rect();
    let paper = grid.get_bounding_client_rect();
    let today_el = grid.query_selector(".day[data-day='0']").ok()??;
    let ribbon = page_el
        .query_selector(".bookmark-ribbon")
        .ok()??
        .get_bounding_client_rect();
    // Every day's right edge, today's first: the week's columns are in day order.
    let days = grid.query_selector(".week").ok()??.children();
    let rights = (0..days.length())
        .filter_map(|i| days.item(i))
        .map(|day| day.get_bounding_client_rect().right() - page.left());
    let left = crate::ribbon::hang_left(
        rights,
        ribbon.width(),
        paper.left() - page.left(),
        paper.right() - page.left() - 1.5 * ribbon.width(),
    );
    // The names stick to the top of the paper as it scrolls, so the band stays put.
    let band = today_el
        .query_selector(".day-name")
        .ok()??
        .get_bounding_client_rect();
    Some((
        left,
        (band.top() - page.top()).floor(),
        band.height().ceil(),
    ))
}

/// Calls `resized` whenever `paper` changes size, for as long as the page lives.
#[cfg(target_arch = "wasm32")]
fn on_resized(paper: &web_sys::Element, resized: impl Fn() + 'static) {
    use wasm_bindgen::JsCast as _;
    let listener = wasm_bindgen::closure::Closure::<dyn Fn()>::new(resized);
    if let Ok(observer) = web_sys::ResizeObserver::new(listener.as_ref().unchecked_ref()) {
        observer.observe(paper);
    }
    listener.forget();
}

/// Slides the paper to `to` (its scroll left and top) over `RIBBON_GLIDE_MS` on the
/// engine's loop, or at once with reduced motion.
#[cfg(target_arch = "wasm32")]
fn slide_to(paper: web_sys::HtmlElement, to: (f64, f64)) {
    if crate::mascot::prefers_reduced_motion() {
        paper.set_scroll_left(to.0 as i32);
        paper.set_scroll_top(to.1 as i32);
        return;
    }
    let from = (
        f64::from(paper.scroll_left()),
        f64::from(paper.scroll_top()),
    );
    let began = crate::engine::now();
    crate::engine::on_tick(move |tick| {
        let ms = tick.now - began;
        paper.set_scroll_left(crate::ribbon::glide(from.0, to.0, ms).round() as i32);
        paper.set_scroll_top(crate::ribbon::glide(from.1, to.1, ms).round() as i32);
        if ms >= crate::tuning::RIBBON_GLIDE_MS {
            crate::engine::Flow::Stop
        } else {
            crate::engine::Flow::Go
        }
    });
}

/// The planner's clock and zone: now, and the zone the server planned with. An unknown
/// zone name falls back to UTC, so the tasks still show.
pub(crate) fn planner_context(tz_name: &str) -> LocalContext {
    LocalContext {
        now: now_utc(),
        tz: tz_name.parse().unwrap_or(chrono_tz::UTC),
    }
}

/// A placement's start as the planner shows it: weekday and clock time in the
/// zone, written in the paper's clock, short or long as the gear set it.
pub(crate) fn local_time_label(
    ctx: &LocalContext,
    start: DateTime<Utc>,
    clock: clock::Clock,
) -> String {
    let local = ctx.local(start);
    clock::format_day_time(
        &local.format("%a").to_string(),
        local.hour() as u16 * 60 + local.minute() as u16,
        clock,
    )
}

/// The current instant. On the page it comes from the browser: the platform clock that
/// chrono and std use is not there on wasm32.
fn now_utc() -> DateTime<Utc> {
    #[cfg(target_arch = "wasm32")]
    {
        DateTime::from_timestamp_millis(js_sys::Date::now() as i64).unwrap_or(DateTime::UNIX_EPOCH)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        SystemClock.now()
    }
}

#[cfg(target_arch = "wasm32")]
fn current_minutes() -> u16 {
    let now = js_sys::Date::new_0();
    now.get_hours() as u16 * 60 + now.get_minutes() as u16
}

#[cfg(not(target_arch = "wasm32"))]
fn current_minutes() -> u16 {
    0
}

/// A touch that has gone down on a slip but the slip has not lifted yet: the
/// armed hold, and how far the finger has slid since it went down.
#[cfg(target_arch = "wasm32")]
struct Press {
    /// The hold that will lift the slip; taken and cancelled on a real slide.
    lift: Option<gloo_timers::callback::Timeout>,
    /// Total distance the finger has slid since it went down, in pixels.
    slide: f64,
    /// Where the finger was last seen, client coordinates.
    x: f64,
    y: f64,
}

#[component]
fn PlacementBlock(
    placement: Placement,
    snapshot: ReadSignal<Snapshot>,
    settings: ReadSignal<Settings>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
    drag: RwSignal<Option<Drag>>,
    landing: RwSignal<Option<Slot>>,
    mascot_state: ReadSignal<crate::mascot::State>,
) -> impl IntoView {
    let task_id = placement.task.clone();
    let label_task_id = task_id.clone();
    let slip_id = task_id.clone();
    let drag_task_id = task_id.clone();
    // Each handler owns its id clone: none of them may move the shared one.
    #[cfg(target_arch = "wasm32")]
    let down_id = task_id.clone();
    #[cfg(target_arch = "wasm32")]
    let move_id = task_id.clone();
    #[cfg(target_arch = "wasm32")]
    let end_id = task_id.clone();
    #[cfg(target_arch = "wasm32")]
    let cancel_id = task_id.clone();
    // The slip's time rides the gear's clock: it re-writes when the clock flips.
    let render_time = move || {
        local_time_label(
            &planner_context(&snapshot.get().tz),
            placement.start,
            settings.get().clock,
        )
    };

    // A phone finger's press state for this slip, shared by the handlers: the
    // armed lift, and how far the finger has slid since it went down.
    #[cfg(target_arch = "wasm32")]
    let press = std::rc::Rc::new(std::cell::RefCell::new(None::<Press>));

    // Lift a slip onto its finger at a point: the title comes from the
    // snapshot. The pointer is usually captured already, at pointer-down.
    #[cfg(target_arch = "wasm32")]
    let lift = move |x: f64, y: f64, grab: (f64, f64)| {
        let snap = snapshot.get_untracked();
        let Some(task) = snap.tasks.iter().find(|task| task.id == down_id) else {
            return;
        };
        let title = task.title.clone();
        drop(snap);
        drag.set(Some(Drag {
            id: down_id.clone(),
            title,
            x,
            y,
            grab,
        }));
    };

    #[cfg(target_arch = "wasm32")]
    let grip_down = {
        // Each handler owns its own press handle: no closure steals another's.
        let press = press.clone();
        move |event: leptos::ev::PointerEvent| {
            use wasm_bindgen::JsCast;
            if event.pointer_type() == "mouse" && event.buttons() != 1 {
                return;
            }
            // Only one slip is in hand at a time.
            if drag.get_untracked().is_some() {
                return;
            }
            // Dazed, Rambo takes every hold on the page, and a mouse would lift the
            // slip before he could: the slip stays put until he is home.
            if crate::mascot::drawn_over(
                &mascot_state.get_untracked(),
                &crate::gesture::Gesture::Hold,
            )
            .is_some()
            {
                return;
            }
            // Take the pointer at once, so both the hold and a later slide stay
            // ours over other elements. No preventDefault: whether a touch may
            // scroll is the slip's `touch-action`, which is none (below).
            if let Some(slip) = event
                .target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            {
                let _ = slip.set_pointer_capture(event.pointer_id());
            }
            let x = f64::from(event.client_x());
            let y = f64::from(event.client_y());
            // Where on the slip the finger is: the handlers sit on the slip itself.
            let grab = event
                .current_target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                .map(|slip| {
                    let rect = slip.get_bounding_client_rect();
                    (x - rect.left(), y - rect.top())
                })
                .unwrap_or_default();
            if event.pointer_type() == "mouse" {
                // A mouse lifts a slip the moment it takes hold: no hold to mistime.
                lift(x, y, grab);
                return;
            }
            // A touch lifts when its finger has held still a beat; until then a
            // slide is a scroll, panned by hand under the finger, because a slip
            // is touch-action: none and never lets the browser scroll itself.
            *press.borrow_mut() = Some(Press {
                lift: None,
                slide: 0.0,
                x,
                y,
            });
            // The hold gets its own clones: the handlers stay Fn and reusable.
            let hold = {
                let press = press.clone();
                let lift = lift.clone();
                gloo_timers::callback::Timeout::new(PRESS_HOLD_MS, move || {
                    press.borrow_mut().take();
                    lift(x, y, grab);
                })
            };
            if let Some(p) = press.borrow_mut().as_mut() {
                p.lift = Some(hold);
            }
        }
    };

    #[cfg(target_arch = "wasm32")]
    let grip_move = {
        let press = press.clone();
        move |event: leptos::ev::PointerEvent| {
            use wasm_bindgen::JsCast;
            let x = f64::from(event.client_x());
            let y = f64::from(event.client_y());
            // The slip is not up yet: a real slide is a scroll, panned by hand.
            if drag.get_untracked().is_none() {
                let mut state = press.borrow_mut();
                let Some(p) = state.as_mut() else {
                    return;
                };
                if event.buttons() == 0 {
                    return;
                }
                let dx = x - p.x;
                let dy = y - p.y;
                p.slide += (dx.powi(2) + dy.powi(2)).sqrt();
                p.x = x;
                p.y = y;
                if p.slide <= PRESS_SLOP_PX {
                    // A tremble: the hold stays armed.
                    return;
                }
                // A real slide: the hold is off and the paper pans with the finger.
                p.lift.take();
                drop(state);
                if let Some(slip) = event
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                {
                    pan_under(dx, dy, &slip);
                }
                return;
            }
            let Some(mut d) = drag.get_untracked() else {
                return;
            };
            if d.id != move_id || event.buttons() == 0 {
                return;
            }
            d.x = x;
            d.y = y;
            drag.set(Some(d.clone()));
            landing.set(landing_at(x, y, &settings.get_untracked()));
        }
    };

    #[cfg(target_arch = "wasm32")]
    let grip_end = {
        let press = press.clone();
        move |_event: leptos::ev::PointerEvent| {
            // Whatever else this release was, the hold is off.
            press.borrow_mut().take();
            let Some(d) = drag.get_untracked() else {
                return;
            };
            if d.id != end_id {
                return;
            }
            drag.set(None);
            let landing_slot = landing.get_untracked();
            landing.set(None);
            let Some(slot) = landing_slot else {
                return;
            };
            // Dropped where it already sat: nothing to change.
            let ctx = planner_context(&snapshot.get_untracked().tz);
            let local_start = ctx.local(placement.start);
            let current = Slot {
                day: day_index(placement.start, &ctx),
                minutes: local_start.hour() as u16 * 60 + local_start.minute() as u16,
            };
            if slot == current {
                return;
            }
            place_at(&end_id, slot, snapshot, set_snapshot, set_error);
        }
    };

    #[cfg(target_arch = "wasm32")]
    let grip_cancel = {
        let press = press.clone();
        move |_| {
            press.borrow_mut().take();
            if drag.get_untracked().is_some_and(|d| d.id == cancel_id) {
                drag.set(None);
                landing.set(None);
            }
        }
    };
    // On a native build the patch paths never run; keep the unused-shelf silent.
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (&set_snapshot, &set_error, &drag, &landing, &mascot_state);
    #[cfg(not(target_arch = "wasm32"))]
    let (grip_down, grip_move, grip_end, grip_cancel) = (
        |_: leptos::ev::PointerEvent| {},
        |_: leptos::ev::PointerEvent| {},
        |_: leptos::ev::PointerEvent| {},
        |_: leptos::ev::PointerEvent| {},
    );

    view! {
        <div class="placement" class:short=move || {
            let (.., height) = placement_box(
                placement.start,
                placement.end,
                &planner_context(&snapshot.get().tz),
                &settings.get(),
            );
            height < SHORT_SLIP_PX
        } class:dragging=move || drag.get().is_some_and(|d| d.id == drag_task_id) style=move || {
            let (top, height) = placement_box(
                placement.start,
                placement.end,
                &planner_context(&snapshot.get().tz),
                &settings.get(),
            );
            format!("top: {}px; height: {}px;", top, height)
        }
            // The whole slip is the grab area: pointer handlers sit on the
            // block itself. On a phone they lift only after the hold (so a
            // slide stays a scroll), and touching a slip is touch-action:
            // none, so it can never start the page flip.
            on:pointerdown=grip_down
            on:pointermove=grip_move
            on:pointerup=grip_end
            on:pointercancel=grip_cancel
            // Who the slip is, for Rambo dropped on it (`slip_at`).
            data-id=slip_id.0.clone()
            data-repeats=occurrence_of(&slip_id).is_some().then_some("")
            data-title=move || placement_label(&snapshot.get(), &slip_id)
        >
            <span
                class="placement-grip"
                aria-label=format!("Drag {} to another slot", placement_label(&snapshot.get_untracked(), &task_id))
            ></span>
            <span class="slip-text">
                {move || view! { <strong>{placement_label(&snapshot.get(), &label_task_id)}</strong> }}
                " · " {render_time}
            </span>
        </div>
    }
}

/// Starts task `id` at `slot`, as a dropped slip or a tapped hour says.
#[cfg(target_arch = "wasm32")]
fn place_at(
    id: &TaskId,
    slot: Slot,
    snapshot: ReadSignal<Snapshot>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
) {
    let snap = snapshot.get_untracked();
    let ctx = planner_context(&snap.tz);
    let Some(instant) = drop_start(&ctx, slot.day, slot.minutes) else {
        set_error.set(Some(
            "The clock skips that time that day (daylight change), so it cannot start then.".into(),
        ));
        return;
    };
    // The guard stays fresh from the last snapshot the page saw: if the
    // task changed elsewhere meanwhile, the server refuses the move.
    let Some(updated_at) = snap
        .tasks
        .iter()
        .find(|task| task.id == *id)
        .map(|task| task.updated_at)
    else {
        return;
    };
    // One day of a repeating series is a 'moved' exception on the series,
    // never a task PATCH: the row here is one expanded occurrence of the
    // whole rule, and its id names the original local date.
    let occurrence = occurrence_of(id);
    let id = id.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = match occurrence {
            Some((series_id, date)) => {
                crate::api::move_series_occurrence(&series_id, date, When::At(instant)).await
            }
            None => {
                // Placed, a task taken off the planner is back on it.
                let body = crate::api::PatchTaskBody::new()
                    .when(When::At(instant))
                    .off_planner(false)
                    .updated_at(updated_at);
                crate::api::patch_task(&id, body).await
            }
        };
        match result {
            Ok(new_snapshot) => set_snapshot.set(new_snapshot),
            Err(e) => set_error.set(Some(format!("Move failed: {e}"))),
        }
    });
}

/// What sits under a point: the planner day column (from data-day) and the
/// pixel top of its hour rows, or None when the point is off the planner.
#[cfg(target_arch = "wasm32")]
fn drop_location(x: f64, y: f64) -> Option<(i64, f32)> {
    use wasm_bindgen::JsCast;
    let element = under(x, y)?;
    let day = element
        .closest(".day")
        .ok()
        .flatten()
        .and_then(|day| day.get_attribute("data-day"))?
        .parse()
        .ok()?;
    let hours = element
        .closest(".hours")
        .ok()
        .flatten()
        .and_then(|hours| hours.dyn_into::<web_sys::HtmlElement>().ok())?;
    Some((day, hours.get_bounding_client_rect().top() as f32))
}

/// The slot a point is over, day and snapped minute, or None when the point
/// is off the planner. What the held slip's outline and its own drop use.
#[cfg(target_arch = "wasm32")]
fn landing_at(x: f64, y: f64, settings: &Settings) -> Option<Slot> {
    let location = drop_location(x, y)?;
    let minutes = drop_minute(
        y as f32 - location.1,
        settings.start_hour,
        settings.end_hour,
        HOUR_PX,
    );
    Some(Slot {
        day: location.0,
        minutes,
    })
}

/// The hour under a point on the screen, as a day and a quarter hour: what a tap
/// while a task waits, or Rambo dropped on the planner, points at. None off the
/// planner or over a slip, which is a task of its own.
#[cfg(target_arch = "wasm32")]
pub(crate) fn hour_at(x: f64, y: f64, settings: &Settings) -> Option<Slot> {
    if slip_at(x, y).is_some() {
        return None;
    }
    landing_at(x, y, settings)
}

/// Off the browser there is no page to point at.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn hour_at(_x: f64, _y: f64, _settings: &Settings) -> Option<Slot> {
    None
}

/// The slip under a point on the screen: the task it is, its title, and whether it is
/// one day of a repeating series. What Rambo dropped on the planner asks about.
pub(crate) fn slip_at(x: f64, y: f64) -> Option<(TaskId, String, bool)> {
    let slip = under(x, y)?.closest(".placement[data-id]").ok().flatten()?;
    Some((
        TaskId(slip.get_attribute("data-id")?),
        slip.get_attribute("data-title").unwrap_or_default(),
        slip.has_attribute("data-repeats"),
    ))
}

/// The topmost element at a point, looking past Rambo: while he is carried he is
/// under the finger himself, and a hold or a drop means what lies beneath him.
fn under(x: f64, y: f64) -> Option<web_sys::Element> {
    use wasm_bindgen::JsCast as _;
    leptos::prelude::document()
        .elements_from_point(x as f32, y as f32)
        .iter()
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .find(|el| el.closest(".rambo").ok().flatten().is_none())
}

/// Makes task `id` a quarter hour longer or shorter (`placing::stretched`): its slip
/// grows or shrinks on the page at once, and the new length is saved. A length already
/// at its limit stays as it is.
pub(crate) fn stretch(
    id: &TaskId,
    stretch: crate::placing::Stretch,
    snapshot: ReadSignal<Snapshot>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
) {
    let Some((from, updated_at)) = snapshot.with_untracked(|snap| {
        snap.tasks
            .iter()
            .find(|task| task.id == *id)
            .map(|task| (task.duration_min, task.updated_at))
    }) else {
        return;
    };
    let to = crate::placing::stretched(from, stretch);
    if to == from {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    resize_slip(id, from, to);
    let id = id.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let body = crate::api::PatchTaskBody::new()
            .duration_min(to)
            .updated_at(updated_at);
        match crate::api::patch_task(&id, body).await {
            Ok(new_snapshot) => {
                // The saved plan draws the slip anew; it carries on from the height it
                // had reached instead of starting at its new one. The redraw runs as a
                // microtask, so the ease queued after it lands before the next paint:
                // a frame later, the new slip would show once at its planned height.
                #[cfg(target_arch = "wasm32")]
                let reached = slips_of(&id)
                    .first()
                    .map(|slip| slip.get_bounding_client_rect().height());
                set_snapshot.set(new_snapshot);
                #[cfg(target_arch = "wasm32")]
                if let Some(reached) = reached {
                    wasm_bindgen_futures::spawn_local(async move { ease_slip_from(&id, reached) });
                }
            }
            Err(e) => {
                // Refused, the slip goes back to the length it still has.
                #[cfg(target_arch = "wasm32")]
                unstretch_slip(&id, from, to);
                set_error.set(Some(format!("Resize failed: {e}")));
            }
        }
    });
}

/// Takes task `id` off the planner (`Task::off_planner`): it stays in the list, and the
/// scheduler leaves it alone until it is placed again. Its slips slide out of their
/// days at once, and the week is drawn without them once that has run and the change
/// is saved; refused, they slide back.
pub(crate) fn take_off(
    id: &TaskId,
    snapshot: ReadSignal<Snapshot>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
) {
    let Some(updated_at) = snapshot.with_untracked(|snap| {
        snap.tasks
            .iter()
            .find(|task| task.id == *id)
            .map(|task| task.updated_at)
    }) else {
        return;
    };
    #[cfg(target_arch = "wasm32")]
    slide_off(id, true);
    let began = crate::engine::now();
    let id = id.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let body = crate::api::PatchTaskBody::new()
            .off_planner(true)
            .updated_at(updated_at);
        let saved = crate::api::patch_task(&id, body).await;
        let left = f64::from(crate::tuning::SLIP_OFF_MS) - (crate::engine::now() - began);
        if left > 0.0 {
            gloo_timers::future::TimeoutFuture::new(left.ceil() as u32).await;
        }
        match saved {
            Ok(new_snapshot) => set_snapshot.set(new_snapshot),
            Err(e) => {
                #[cfg(target_arch = "wasm32")]
                slide_off(&id, false);
                set_error.set(Some(format!("Taking it off failed: {e}")));
            }
        }
    });
}

/// Slides task `id`'s slips out of their days (`off`), or back in.
#[cfg(target_arch = "wasm32")]
fn slide_off(id: &TaskId, off: bool) {
    for slip in slips_of(id) {
        let _ = slip.style().set_property(
            "--slip-off-ms",
            &format!("{}ms", crate::tuning::SLIP_OFF_MS),
        );
        let _ = slip.class_list().toggle_with_force("slip-off", off);
    }
}

/// Task `id`'s slips on the page.
#[cfg(target_arch = "wasm32")]
fn slips_of(id: &TaskId) -> Vec<web_sys::HtmlElement> {
    use wasm_bindgen::JsCast as _;
    let slips = leptos::prelude::document().get_elements_by_class_name("placement");
    (0..slips.length())
        .filter_map(|i| slips.item(i))
        .filter_map(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
        .filter(|slip| slip.get_attribute("data-id").as_deref() == Some(id.0.as_str()))
        .collect()
}

/// Grows or shrinks task `id`'s slip on the page from `from` to `to` minutes long, so
/// it moves the moment it is asked, before the change is saved.
#[cfg(target_arch = "wasm32")]
fn resize_slip(id: &TaskId, from: u16, to: u16) {
    for slip in slips_of(id) {
        let height = resized_px(slip.get_bounding_client_rect().height() as f32, from, to);
        let _ = slip.class_list().add_1("stretching");
        let _ = slip
            .style()
            .set_property("height", &format!("{height:.1}px"));
    }
}

/// Takes back a stretch whose save was refused: task `id`'s slips ease from the `to`
/// minutes they were stretched to back to `from`, then lose `.stretching`, so a later
/// redraw of them does not ease. It starts from the height they were stretched to, not
/// the one they have reached, since a quick refusal can come mid-ease.
#[cfg(target_arch = "wasm32")]
fn unstretch_slip(id: &TaskId, from: u16, to: u16) {
    use wasm_bindgen::JsCast as _;
    for slip in slips_of(id) {
        let style = slip.style();
        let stretched = style
            .get_property_value("height")
            .ok()
            .and_then(|height| height.trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or_else(|| slip.get_bounding_client_rect().height() as f32);
        let _ = style.set_property(
            "height",
            &format!("{:.1}px", resized_px(stretched, to, from)),
        );
        let eased = slip.clone();
        let done = wasm_bindgen::closure::Closure::once_into_js(move || {
            let _ = eased.class_list().remove_1("stretching");
        });
        let once = web_sys::AddEventListenerOptions::new();
        once.set_once(true);
        let _ = slip.add_event_listener_with_callback_and_add_event_listener_options(
            "transitionend",
            done.unchecked_ref(),
            &once,
        );
    }
}

/// Eases task `id`'s newly drawn slip from `reached` px to the height the plan gives it
/// (`.stretching`'s height transition, which only a stretched slip has).
#[cfg(target_arch = "wasm32")]
fn ease_slip_from(id: &TaskId, reached: f64) {
    for slip in slips_of(id) {
        let style = slip.style();
        let Ok(planned) = style.get_property_value("height") else {
            continue;
        };
        let _ = style.set_property("transition", "none");
        let _ = style.set_property("height", &format!("{reached:.2}px"));
        // Lay it out at the height reached before it eases on.
        let _ = slip.offset_height();
        let _ = style.remove_property("transition");
        let _ = slip.class_list().add_1("stretching");
        let _ = style.set_property("height", &planned);
    }
}

/// The paper a not-yet-lifted finger slides over: `main` along its length,
/// the planner's own scroll box down its hours, in the direction the finger
/// drags them, because a slip itself never scrolls (it is touch-action: none).
#[cfg(target_arch = "wasm32")]
fn pan_under(dx: f64, dy: f64, under: &web_sys::Element) {
    use wasm_bindgen::JsCast;
    if dx != 0.0
        && let Some(page) = under
            .closest("main")
            .ok()
            .flatten()
            .and_then(|page| page.dyn_into::<web_sys::HtmlElement>().ok())
    {
        page.set_scroll_left(page.scroll_left() - dx as i32);
    }
    if dy != 0.0
        && let Some(scroller) = under
            .closest(".week-scroll")
            .ok()
            .flatten()
            .and_then(|scroller| scroller.dyn_into::<web_sys::HtmlElement>().ok())
    {
        scroller.set_scroll_top(scroller.scroll_top() - dy as i32);
    }
}
/// What a placement block shows: the task's title, looked up in the snapshot. A
/// placement always ships with its task, but a snapshot mid-swap may not, so an
/// unknown id falls back to the id itself rather than an empty block.
fn placement_label(snapshot: &Snapshot, task_id: &TaskId) -> String {
    snapshot
        .tasks
        .iter()
        .find(|task| task.id == *task_id)
        .map(|task| task.title.clone())
        .unwrap_or_else(|| task_id.0.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_dragged_slip_stays_under_the_finger_where_it_was_taken() {
        let drag = |x, y, grab| super::Drag {
            id: todo_core::TaskId("t1".into()),
            title: "Lab report".into(),
            x,
            y,
            grab,
        };
        // Taken 12px in and 16px down, the ghost's corner keeps that offset wherever
        // the finger goes, including off the planner's left and top edges.
        for (x, y) in [(575.0, 174.0), (1745.0, 1000.0), (5.0, 3.0)] {
            let (left, top) = drag(x, y, (12.0, 16.0)).ghost_corner();
            assert_eq!((x - left, y - top), (12.0, 16.0));
        }
        assert_eq!(drag(40.0, 50.0, (0.0, 0.0)).ghost_corner(), (40.0, 50.0));
    }

    use super::*;
    use chrono::TimeZone;

    #[test]
    fn a_placement_block_shows_the_task_title_not_the_raw_id() {
        let snapshot = crate::api::fixture_snapshot();
        let placement = &snapshot.plan.placements[0];
        assert_eq!(placement.task.0, "history");
        assert_eq!(
            placement_label(&snapshot, &placement.task),
            "Finish history paper"
        );
    }

    #[test]
    fn an_unknown_task_shows_the_id_rather_than_an_empty_block() {
        let snapshot = crate::api::fixture_snapshot();
        assert_eq!(placement_label(&snapshot, &TaskId("gone".into())), "gone");
    }

    fn zone(name: &str, now: DateTime<Utc>) -> LocalContext {
        LocalContext {
            now,
            tz: name.parse().unwrap(),
        }
    }

    fn at_utc(year: u32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year as i32, month, day, hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn a_task_shows_its_clock_time_in_the_users_zone() {
        // 14:00 UTC on Monday 12 Oct is 10:00 in New York (EDT, UTC-4).
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        assert_eq!(
            local_time_label(&ny, at_utc(2026, 10, 12, 14, 0), clock::Clock::Twelve),
            "Mon 10 am"
        );
        assert_eq!(
            local_time_label(&ny, at_utc(2026, 10, 12, 14, 30), clock::Clock::Twelve),
            "Mon 10:30 am"
        );
        // The 24-hour clock keeps the machine hours.
        assert_eq!(
            local_time_label(&ny, at_utc(2026, 10, 12, 14, 0), clock::Clock::TwentyFour),
            "Mon 10:00"
        );
    }

    #[test]
    fn an_unknown_zone_name_falls_back_to_utc() {
        let context = planner_context("Not/AZone");
        assert_eq!(context.tz, chrono_tz::UTC);
    }
}

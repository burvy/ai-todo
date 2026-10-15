use chrono::{DateTime, Timelike, Utc};
use leptos::portal::Portal;
use leptos::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use todo_core::{Clock, SystemClock};
use todo_core::{LocalContext, Placement, Snapshot, TaskId};

use crate::store::Store;
use crate::tuning::{HOUR_PX, SHORT_SLIP_PX};
use crate::{clock, settings::Settings};
pub(crate) use rambl_web_core::planner::*;
use rambl_web_core::store::Command;
use rambl_web_core::week::{self as model, SlipDrag};

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
    /// Turn to this page: where a task asked to move waits.
    turn_here: std::rc::Rc<dyn Fn()>,
    /// The page shown: 0 the list, 1 this planner.
    pane: ReadSignal<u8>,
) -> impl IntoView {
    // A task Rambo was asked to move (`Event::MoveAsked`): it waits for a tap on an hour.
    // The page turns here when one is asked, and nothing waits once it turns back.
    let waiting = RwSignal::new(None::<TaskId>);
    let events = expect_context::<crate::events::Queue>();
    Effect::new(move |_| {
        events.track();
        if let Some(id) = events.take_all(rambl_web_core::events::move_asked).pop() {
            waiting.set(Some(TaskId(id)));
            turn_here();
        }
    });
    Effect::new(move |_| {
        if pane.get() == 0 {
            waiting.set(None);
        }
    });
    let grid_ref = NodeRef::<leptos::html::Div>::new();
    #[cfg(target_arch = "wasm32")]
    let store = expect_context::<Store>();
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
    let drag = RwSignal::new(None::<SlipDrag>);
    // Where it would land: day + minutes, drawn as a faint slot outline.
    let landing = RwSignal::new(None::<Slot>);
    // The ghost of a slip let go, on its way to where it goes.
    let let_go = RwSignal::new(None::<model::LetGoGhost>);
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
    // (flip_turner.rs turns back to the list); the class tells the browser to let it through.
    let (at_earliest, set_at_earliest) = signal(true);
    // The bookmark ribbon's head (ribbon.rs) sticks out of the day names at today's
    // column, or at the edge today went past, kept up with the paper as it scrolls; its
    // tail lies on today's hours, under the slips. A hop to another day eases: the head
    // is drawn at its spot plus a slack that glides away (`ribbon::hop_slack`), so it
    // still keeps up with the paper while it eases.
    let (ribbon_left, set_ribbon_left) = signal(0.0_f64);
    let (ribbon_top, set_ribbon_top) = signal(0.0_f64);
    let (ribbon_height, set_ribbon_height) = signal(0.0_f64);
    let ribbon_hung = StoredValue::new(None::<(Option<usize>, f64)>);
    let ribbon_slack = StoredValue::new(0.0_f64);
    let hang_ribbon = move || {
        #[cfg(target_arch = "wasm32")]
        if let Some((day, left, top, height)) =
            grid_ref.get_untracked().and_then(|grid| ribbon_spot(&grid))
        {
            let was = ribbon_hung.get_value().map(|(was, _)| was);
            ribbon_hung.set_value(Some((day, left)));
            if let Some(slack) =
                crate::ribbon::hop_slack(was, day, ribbon_left.get_untracked(), left)
            {
                crate::slide::slide(
                    "ribbon-head",
                    slack,
                    0.0,
                    crate::tuning::SLIDE_NEAR_PX,
                    move |slack| {
                        ribbon_slack.set_value(slack);
                        let spot = ribbon_hung.get_value().map_or(0.0, |(_, left)| left);
                        set_ribbon_left.set(spot + slack);
                    },
                    || {},
                );
            }
            set_ribbon_left.set(left + ribbon_slack.get_value());
            set_ribbon_top.set(top);
            set_ribbon_height.set(height);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (
            ribbon_hung,
            ribbon_slack,
            set_ribbon_left,
            set_ribbon_top,
            set_ribbon_height,
        );
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
        let Some(id) = waiting.get_untracked() else {
            return;
        };
        waiting.set(None);
        landing.set(None);
        #[cfg(target_arch = "wasm32")]
        place_at(&id, slot, snapshot, store);
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (id, slot);
    });
    // While a task waits, the hour under the pointer is where a tap puts it: outlined
    // under a mouse, placed on the tap. A held slip keeps its own outline.
    let waiting_slot = move |ev: &leptos::ev::PointerEvent| {
        if waiting.with_untracked(Option::is_none) || drag.with_untracked(Option::is_some) {
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
                <crate::gear_menu::GearMenu settings set_settings set_snapshot set_error />
            </div>
            <crate::PageFoot page=1 turn=turn_back />
            <crate::placing::PlacingNote snapshot settings to_place=waiting place />
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
                                class:placing=move || waiting.with(Option::is_some)
                                on:pointermove=move |ev| {
                                    if ev.pointer_type() == "mouse"
                                        && let Some(slot) = waiting_slot(&ev)
                                    {
                                        landing.set(Some(slot));
                                    }
                                }
                                on:pointerleave=move |_| {
                                    if waiting.with_untracked(Option::is_some)
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
                                                node_ref=crate::slide::arriving()
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
                                <PlacementBlock placement snapshot settings drag landing let_go mascot_state />
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
            // Held, it is on the finger; let go, it springs to where it goes.
            {move || {
                let (d, (left, top), slot) = match drag.get() {
                    Some(d) => {
                        let corner = d.ghost_corner();
                        (d, corner, landing.get())
                    }
                    None => {
                        let ghost = let_go.get()?;
                        (ghost.drag, ghost.at, ghost.to)
                    }
                };
                Some(view! {
                    <div
                        class="drag-ghost"
                        style=format!("left: {left:.1}px; top: {top:.1}px;")
                        aria-hidden="true"
                    >
                        <strong>{d.title}</strong>
                        // The time the slip would get, live as the finger moves it.
                        <span class="ghost-time">
                            {slot.map(|slot| {
                                let ctx = planner_context(&snapshot.get().tz);
                                let weekday = (ctx.today() + chrono::Duration::days(slot.day))
                                    .format("%a")
                                    .to_string();
                                clock::format_day_time(&weekday, slot.minutes, settings.get().clock)
                            })}
                        </span>
                    </div>
                })
            }}
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
/// `tuning::LONG_PRESS_MS` pre-fills the capture box with that slot and focuses it;
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
        crate::engine::after(f64::from(crate::tuning::LONG_PRESS_MS), move || {
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
        });
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
fn ribbon_spot(grid: &web_sys::HtmlDivElement) -> Option<(Option<usize>, f64, f64, f64)> {
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
    let (day, left) = crate::ribbon::hang_left(
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
        day,
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
    lift: Option<crate::engine::Guard>,
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
    drag: RwSignal<Option<SlipDrag>>,
    landing: RwSignal<Option<Slot>>,
    let_go: RwSignal<Option<model::LetGoGhost>>,
    mascot_state: ReadSignal<crate::mascot::State>,
) -> impl IntoView {
    #[cfg(target_arch = "wasm32")]
    let store = expect_context::<Store>();
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
        drag.set(Some(SlipDrag {
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
                let mut hold = crate::engine::Guard::default();
                hold.keep(crate::engine::after(
                    f64::from(crate::tuning::PRESS_HOLD_MS),
                    move || {
                        press.borrow_mut().take();
                        lift(x, y, grab);
                    },
                ));
                hold
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
                if !model::press_scrolls(p.slide) {
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
        move |event: leptos::ev::PointerEvent| {
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
            let ctx = planner_context(&snapshot.get_untracked().tz);
            let to = model::slip_moves_to(placement.start, &ctx, landing_slot);
            let home = own_corner(&event);
            let Some(slot) = to else {
                // Where it sat, or off the planner: the ghost goes back to its slip.
                let_ghost_go(let_go, d, None, home, move || {});
                return;
            };
            let target = slot_screen_corner(slot, &settings.get_untracked()).unwrap_or(home);
            // It is gone once it has landed and the plan shows the slip there, whichever
            // comes last; refused, it goes back to its slip.
            let waiting = std::rc::Rc::new(std::cell::Cell::new(2_u8));
            let finish = {
                let (waiting, d) = (waiting.clone(), d.clone());
                move || {
                    waiting.set(waiting.get().saturating_sub(1));
                    if waiting.get() == 0 {
                        clear_ghost(let_go, &d);
                    }
                }
            };
            let_ghost_go(let_go, d.clone(), Some(slot), target, finish.clone());
            let id = end_id.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if place(&id, slot, snapshot, store).await {
                    finish();
                } else {
                    let_ghost_go(let_go, d, None, home, move || {});
                }
            });
        }
    };

    #[cfg(target_arch = "wasm32")]
    let grip_cancel = {
        let press = press.clone();
        move |event: leptos::ev::PointerEvent| {
            press.borrow_mut().take();
            if let Some(d) = drag.get_untracked().filter(|d| d.id == cancel_id) {
                drag.set(None);
                landing.set(None);
                // The browser took the finger: the ghost goes back to its slip.
                let_ghost_go(let_go, d, None, own_corner(&event), move || {});
            }
        }
    };
    // On a native build the patch paths never run; keep the unused-shelf silent.
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (&drag, &landing, &let_go, &mascot_state);
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

/// Starts task `id` at `slot`, as a tapped hour says.
#[cfg(target_arch = "wasm32")]
fn place_at(id: &TaskId, slot: Slot, snapshot: ReadSignal<Snapshot>, store: Store) {
    let id = id.clone();
    wasm_bindgen_futures::spawn_local(async move {
        place(&id, slot, snapshot, store).await;
    });
}

/// Starts task `id` at `slot`, and answers whether the plan now shows it there.
#[cfg(target_arch = "wasm32")]
async fn place(id: &TaskId, slot: Slot, snapshot: ReadSignal<Snapshot>, store: Store) -> bool {
    let placed =
        snapshot.with_untracked(|snap| model::placed(snap, &planner_context(&snap.tz), id, slot));
    match placed {
        model::Placed::Send(command) => store.send(command).await.is_some(),
        model::Placed::SkippedTime => {
            store.set_error.set(Some(model::SKIPPED_TIME_NOTE.into()));
            false
        }
        model::Placed::Gone => false,
    }
}

/// Lets slip `drag`'s ghost go: it springs from where it is drawn to the corner `at`
/// (on its way to `to`, or back to its slip), then runs `done`; back at its slip, it
/// is gone. Each slip's ghost glides on its own key, and only the ghost of this very
/// let-go is drawn or cleared by it, so a slip lifted meanwhile is never touched.
#[cfg(target_arch = "wasm32")]
fn let_ghost_go(
    let_go: RwSignal<Option<model::LetGoGhost>>,
    drag: SlipDrag,
    to: Option<Slot>,
    at: (f64, f64),
    done: impl FnOnce() + 'static,
) {
    let from = let_go
        .with_untracked(|ghost| {
            ghost
                .as_ref()
                .filter(|ghost| ghost.drag == drag)
                .map(|ghost| ghost.at)
        })
        .unwrap_or_else(|| drag.ghost_corner());
    let key = format!("drag-ghost:{}", drag.id.0);
    let_go.set(Some(model::LetGoGhost {
        drag: drag.clone(),
        to,
        at: from,
    }));
    let drawn = drag.clone();
    crate::slide::slide_all(
        &key,
        [from.0, from.1],
        [at.0, at.1],
        crate::tuning::SLIDE_NEAR_PX,
        move |[x, y]| {
            let_go.update(|ghost| {
                if let Some(ghost) = ghost.as_mut().filter(|ghost| ghost.drag == drawn) {
                    ghost.at = (x, y);
                }
            });
        },
        move || {
            if to.is_none() {
                clear_ghost(let_go, &drag);
            }
            done();
        },
    );
}

/// Takes slip `drag`'s let-go ghost off the page, if it is still the one shown.
#[cfg(target_arch = "wasm32")]
fn clear_ghost(let_go: RwSignal<Option<model::LetGoGhost>>, drag: &SlipDrag) {
    if let_go.with_untracked(|ghost| ghost.as_ref().is_some_and(|ghost| ghost.drag == *drag)) {
        let_go.set(None);
    }
}

/// The top-left corner of the slip an event is on: where its ghost goes back to.
#[cfg(target_arch = "wasm32")]
fn own_corner(event: &leptos::ev::PointerEvent) -> (f64, f64) {
    use wasm_bindgen::JsCast as _;
    event
        .current_target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .map_or((0.0, 0.0), |slip| {
            let rect = slip.get_bounding_client_rect();
            (rect.left(), rect.top())
        })
}

/// Where a slip placed at `slot` would start on the screen, if its day is on the page.
#[cfg(target_arch = "wasm32")]
fn slot_screen_corner(slot: Slot, settings: &Settings) -> Option<(f64, f64)> {
    let hours = leptos::prelude::document()
        .query_selector(&format!(".day[data-day=\"{}\"] .hours", slot.day))
        .ok()??
        .get_bounding_client_rect();
    Some(model::slot_corner(
        (hours.left(), hours.top()),
        slot.minutes,
        settings.start_hour,
        settings.end_hour,
    ))
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
    store: Store,
) {
    let Some(stretched) = snapshot.with_untracked(|snap| model::stretch_of(snap, id, stretch))
    else {
        return;
    };
    #[cfg(target_arch = "wasm32")]
    resize_slip(id, stretched.from, stretched.to);
    let id = id.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let body = crate::api::PatchTaskBody::new()
            .duration_min(stretched.to)
            .updated_at(stretched.updated_at);
        if store.send(Command::Patch(id.clone(), body)).await.is_some() {
            // The saved plan draws the slip anew; it carries on from the height it
            // had reached instead of starting at its new one. The store has just shown
            // the plan, and the redraw runs as a microtask: reading the slip here still
            // sees the old one, and the ease queued after it lands before the next
            // paint (a frame later, the new slip would show once at its planned height).
            #[cfg(target_arch = "wasm32")]
            if let Some(reached) = slips_of(&id)
                .first()
                .map(|slip| slip.get_bounding_client_rect().height())
            {
                wasm_bindgen_futures::spawn_local(async move { ease_slip_from(&id, reached) });
            }
        } else {
            // Refused, the slip goes back to the length it still has.
            #[cfg(target_arch = "wasm32")]
            resize_slip(&id, stretched.to, stretched.from);
        }
    });
}

/// Takes task `id` off the planner (`Task::off_planner`): it stays in the list, and the
/// scheduler leaves it alone until it is placed again. Its slips slide out of their
/// days at once, and the week is drawn without them once that has run and the change
/// is saved; refused, they slide back.
pub(crate) fn take_off(id: &TaskId, snapshot: ReadSignal<Snapshot>, store: Store) {
    let Some(updated_at) = snapshot.with_untracked(|snap| model::edit_guard(snap, id)) else {
        return;
    };
    #[cfg(target_arch = "wasm32")]
    slide_off(id, true);
    let id = id.clone();
    wasm_bindgen_futures::spawn_local(async move {
        // The slips finish sliding out before the store shows the week without them.
        crate::engine::wait(f64::from(crate::tuning::SLIP_OFF_MS)).await;
        let body = crate::api::PatchTaskBody::new()
            .off_planner(true)
            .updated_at(updated_at);
        if store.send(Command::Patch(id.clone(), body)).await.is_none() {
            #[cfg(target_arch = "wasm32")]
            slide_off(&id, false);
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

/// Grows or shrinks task `id`'s slips on the page from `from` to `to` minutes long, so
/// they move the moment it is asked, before the change is saved. Each counts from the
/// height it is headed to, so a quick second stretch, or a refused one taken back
/// (`to` back to `from`), lands where it should even mid-glide.
#[cfg(target_arch = "wasm32")]
fn resize_slip(id: &TaskId, from: u16, to: u16) {
    for (i, slip) in slips_of(id).into_iter().enumerate() {
        let shown = slip.get_bounding_client_rect().height();
        let headed = slip
            .get_attribute("data-headed-px")
            .and_then(|px| px.parse::<f32>().ok())
            .unwrap_or(shown as f32);
        let to = f64::from(resized_px(headed, from, to));
        glide_height(slip, &stretch_key(id, i), shown, to);
    }
}

/// Glides task `id`'s newly drawn slips from `reached` px to the height the plan gives
/// them, so a saved stretch carries on from where the old slip had got to.
#[cfg(target_arch = "wasm32")]
fn ease_slip_from(id: &TaskId, reached: f64) {
    for (i, slip) in slips_of(id).into_iter().enumerate() {
        let planned = slip.get_bounding_client_rect().height();
        glide_height(slip, &stretch_key(id, i), reached, planned);
    }
}

/// The glide key of task `id`'s `i`th slip.
#[cfg(target_arch = "wasm32")]
fn stretch_key(id: &TaskId, i: usize) -> String {
    format!("stretch:{}:{i}", id.0)
}

/// Glides `slip`'s height from `from` to `to` px, and marks `to` on it as where it is
/// headed.
#[cfg(target_arch = "wasm32")]
fn glide_height(slip: web_sys::HtmlElement, key: &str, from: f64, to: f64) {
    let _ = slip.set_attribute("data-headed-px", &format!("{to:.2}"));
    let style = slip.style();
    crate::slide::slide(
        key,
        from,
        to,
        crate::tuning::SLIDE_NEAR_PX,
        move |height| {
            let _ = style.set_property("height", &format!("{height:.2}px"));
        },
        || {},
    );
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

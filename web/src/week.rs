use chrono::{DateTime, Timelike, Utc};
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use todo_core::When;
#[cfg(not(target_arch = "wasm32"))]
use todo_core::{Clock, SystemClock};
use todo_core::{LocalContext, Placement, Snapshot, TaskId};

use crate::{clock, settings::Settings};

/// A task being dragged by its grip: who it is, and where the finger is.
/// Where it may land is drawn from `landing`; the drop's `updated_at` guard
/// is looked up from the snapshot when the slip lands, so a still fresh
/// edit always wins.
#[derive(Clone, Debug)]
struct Drag {
    id: TaskId,
    title: String,
    x: f64,
    y: f64,
}

/// Where a task would start: a planner day (0 is today) and a minute of that day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    pub(crate) day: i64,
    pub(crate) minutes: u16,
}

/// How far a phone finger must hold still before its slip lifts onto it, so
/// a plain vertical drag still scrolls the paper (the mouse lifts at once).
#[cfg(target_arch = "wasm32")]
const PRESS_HOLD_MS: u32 = 250;

/// How far a not-yet-lifted finger may slide before we call it a scroll and
/// disarm the lift.
#[cfg(target_arch = "wasm32")]
const PRESS_SLOP_PX: f64 = 10.0;

/// The band near a scroll-box edge where a held slip makes the paper slide
/// along so slots beyond the edge can be reached without letting go.
#[cfg(any(target_arch = "wasm32", test))]
const EDGE_ZONE_PX: f32 = 48.0;

/// The loudest the paper slides per auto-scroll tick, right at an edge.
#[cfg(any(target_arch = "wasm32", test))]
const EDGE_MAX_PX: f32 = 6.0;

/// Pixels per auto-scroll tick while a held slip is `distance_to_edge` from
/// the edge: zero outside the band, fastest right at it.
#[cfg(any(target_arch = "wasm32", test))]
fn edge_nudge(distance_to_edge: f32) -> f32 {
    if distance_to_edge >= EDGE_ZONE_PX {
        0.0
    } else {
        ((EDGE_ZONE_PX - distance_to_edge) / EDGE_ZONE_PX) * EDGE_MAX_PX
    }
}

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
    let on_scroll = move |_: leptos::ev::Event| {
        #[cfg(target_arch = "wasm32")]
        if let Some(grid) = grid_ref.get() {
            set_at_earliest.set(grid.scroll_left() <= 1);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = set_at_earliest;
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
    view! {
        <section class="planner-page" aria-label="Planner">
            // The settings gear and the account link live on this page only, at its top right.
            <div class="planner-corner">
                <crate::account::HeaderLink />
                <crate::settings::GearMenu settings set_settings set_snapshot set_error />
            </div>
            <crate::PageFoot page=1 turn=turn_back />
            <crate::placing::PlacingNote snapshot settings move_request place />
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
                                {day_label(&planner_context(&snapshot.get().tz), i64::from(day))}
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
                                            <span>{clock::format_time(
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
            {move || drag.get().map(|d| view! {
                <div
                    class="drag-ghost"
                    style=format!("left: {:.1}px; top: {:.1}px;", d.x, d.y)
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

/// One hour of the week grid, in CSS pixels. The .hour rule in index.html has the same
/// height, so the current-time line can be placed from the clock.
const HOUR_PX: f32 = 64.0;

/// How far down the hour rows the current time sits. The rows start at start_hour.
fn now_line_top(now_minutes: u16, start_hour: u8, hour_px: f32) -> f32 {
    (f32::from(now_minutes) - f32::from(start_hour) * 60.0).max(0.0) / 60.0 * hour_px
}

/// Where the planner opens: about 08:00, or the current time if that is
/// already later — never in the middle of the night, never past the last
/// full hour we draw.
#[cfg(any(target_arch = "wasm32", test))]
fn planner_open_minutes(now_minutes: u16, start_hour: u8, end_hour: u8) -> u16 {
    const OPEN_AT: u16 = 8 * 60;
    let top = u16::from(end_hour).saturating_sub(1) * 60;
    OPEN_AT
        .max(now_minutes)
        .clamp(u16::from(start_hour) * 60, top)
}

/// The handwritten label on a day column: short weekday and date in the
/// planner's zone, counted from today, so day 0 is "today".
fn day_label(ctx: &LocalContext, day: i64) -> String {
    (ctx.today() + chrono::Duration::days(day))
        .format("%a %-d")
        .to_string()
}
/// A slip shorter than a full hour writes its words a size smaller (see
/// theme.css on `.placement.short`); every slip holds them inside its edges.
const SHORT_SLIP_PX: f32 = HOUR_PX;

/// What a slip's row in the week grid is keyed on: the task AND its slot. Keying
/// on the task alone would leave a slip that moves or is re-planned inside its
/// own day painted at the old spot with the old time until the page reloads.
fn placement_key(placement: &Placement) -> (String, DateTime<Utc>, DateTime<Utc>) {
    (placement.task.0.clone(), placement.start, placement.end)
}

/// Top and height in pixels of a slip spanning `start`..`end`, in the
/// planner's zone: a day-crossing task is clipped at the working day's end.
fn placement_box(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    ctx: &LocalContext,
    settings: &Settings,
) -> (f32, f32) {
    let local_start = ctx.local(start);
    let local_end = ctx.local(end);
    let start_min = local_start.hour() as u16 * 60 + local_start.minute() as u16;
    let end_min = if local_end.date() > local_start.date() {
        u16::from(settings.end_hour) * 60
    } else {
        local_end.hour() as u16 * 60 + local_end.minute() as u16
    };
    placement_style(
        start_min,
        end_min,
        settings.start_hour,
        HOUR_PX,
        settings.end_hour,
    )
}

/// Placement top/height in pixels, clipped to planner bounds.
/// start/end are minutes-since-midnight in planner zone.
fn placement_style(
    start_minutes: u16,
    end_minutes: u16,
    planner_start_hour: u8,
    hour_px: f32,
    planner_end_hour: u8,
) -> (f32, f32) {
    let start = f32::from(start_minutes) - f32::from(planner_start_hour) * 60.0;
    let end = f32::from(end_minutes) - f32::from(planner_start_hour) * 60.0;
    let top = start.max(0.0) / 60.0 * hour_px;
    let max_bottom = (i32::from(planner_end_hour) - i32::from(planner_start_hour)) as f32 * hour_px;
    let bottom = (end / 60.0 * hour_px).min(max_bottom);
    let height = (bottom - top).max(0.0);
    (top, height)
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

/// What an occurrence row's id is named from: `"{series_id}:{date}"`, the
/// original local date of the expansion. `None` for a plain task id, whose
/// move is a normal task PATCH.
#[cfg(any(target_arch = "wasm32", test))]
fn occurrence_of(id: &TaskId) -> Option<(String, chrono::NaiveDate)> {
    let (series_id, date) = id.0.rsplit_once(':')?;
    let date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some((series_id.to_owned(), date))
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

/// The day column a placement falls in: its date in the planner's zone, counted from
/// today. Day 0 is today.
fn day_index(start: DateTime<Utc>, ctx: &LocalContext) -> i64 {
    (ctx.local(start).date() - ctx.today()).num_days()
}

/// The placements that fall on planner day `day`, in their original order.
fn placements_on_day(placements: &[Placement], day: i64, ctx: &LocalContext) -> Vec<Placement> {
    placements
        .iter()
        .filter(|placement| day_index(placement.start, ctx) == day)
        .cloned()
        .collect()
}
/// The minute a drop lands on, from a pixel offset inside a day column.
/// Round to the quarter hour, and never land at the very end of the day:
/// a zero-length task at 24:00 cannot start.
#[cfg(any(target_arch = "wasm32", test))]
fn drop_minute(offset_px: f32, planner_start_hour: u8, planner_end_hour: u8, hour_px: f32) -> u16 {
    const SNAP: f32 = 15.0;
    let start_min = f32::from(planner_start_hour) * 60.0;
    let end_min = f32::from(planner_end_hour) * 60.0;
    let raw = start_min + offset_px.max(0.0) / hour_px * 60.0;
    let snapped = (raw / SNAP).round() * SNAP;
    snapped.clamp(start_min, (end_min - SNAP).max(start_min)) as u16
}

/// The instant a slot starts: planner day + minute of the day, read in the
/// planner's zone. None where the local time does not exist (DST jump).
pub(crate) fn drop_start(ctx: &LocalContext, day: i64, minutes: u16) -> Option<DateTime<Utc>> {
    use chrono::TimeZone as _;
    let date = ctx.today() + chrono::Duration::days(day);
    let naive = date.and_hms_opt(u32::from(minutes) / 60, u32::from(minutes) % 60, 0)?;
    ctx.tz
        .from_local_datetime(&naive)
        .earliest()
        .map(|l| l.with_timezone(&Utc))
}

/// Whether a new task could start at `slot`: the time exists that day, and no placed
/// task is under way then (one that ends at that minute is over).
pub(crate) fn free_at(placements: &[Placement], ctx: &LocalContext, slot: Slot) -> bool {
    drop_start(ctx, slot.day, slot.minutes).is_some_and(|instant| {
        !placements
            .iter()
            .any(|placement| (placement.start..placement.end).contains(&instant))
    })
}

/// What the capture box gets pre-filled with on a long-press of an empty hour:
/// the slot as a date phrase Jev already understands, plus a trailing space.
pub(crate) fn hour_prefill_text(ctx: &LocalContext, day: i64, minutes: u16) -> String {
    let date = ctx.today() + chrono::Duration::days(day);
    let day_phrase = match day {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        _ => date.format("%A").to_string().to_lowercase(),
    };
    format!(
        "{day_phrase} {:02}:{:02} ",
        u32::from(minutes) / 60,
        u32::from(minutes) % 60
    )
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
    let lift = move |x: f64, y: f64| {
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
            if event.pointer_type() == "mouse" {
                // A mouse lifts a slip the moment it takes hold: no hold to mistime.
                lift(x, y);
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
                    lift(x, y);
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
                let body = crate::api::PatchTaskBody::new()
                    .when(When::At(instant))
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
    let document = leptos::prelude::document();
    let element = document.element_from_point(x as f32, y as f32)?;
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
    let over_slip = leptos::prelude::document()
        .element_from_point(x as f32, y as f32)
        .is_some_and(|el| el.closest(".placement").ok().flatten().is_some());
    if over_slip {
        return None;
    }
    landing_at(x, y, settings)
}

/// Off the browser there is no page to point at.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn hour_at(_x: f64, _y: f64, _settings: &Settings) -> Option<Slot> {
    None
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

    #[test]
    fn the_line_sits_where_the_clock_is_in_its_hour() {
        // 09:53 with the grid starting at midnight: nine full hours and 53 minutes down.
        let top = now_line_top(9 * 60 + 53, 0, HOUR_PX);
        assert!(
            (top - (9.0 * HOUR_PX + 53.0 / 60.0 * HOUR_PX)).abs() < 0.01,
            "{top}"
        );
    }

    #[test]
    fn the_line_counts_from_the_first_row_not_from_midnight() {
        // Grid starts at 08:00; 09:00 is one hour down.
        assert_eq!(now_line_top(9 * 60, 8, HOUR_PX), HOUR_PX);
        assert_eq!(
            now_line_top(7 * 60, 8, HOUR_PX),
            0.0,
            "before the grid, the line sits at the top"
        );
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
    fn a_placement_lands_on_its_own_local_day() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        // 23:30 UTC on 12 Oct is 19:30 on 12 Oct in New York: today.
        assert_eq!(day_index(at_utc(2026, 10, 12, 23, 30), &ny), 0);
        let berlin = zone("Europe/Berlin", at_utc(2026, 10, 12, 12, 0));
        // The same instant is 01:30 on 13 Oct in Berlin: tomorrow.
        assert_eq!(day_index(at_utc(2026, 10, 12, 23, 30), &berlin), 1);
    }

    #[test]
    fn an_hour_is_free_unless_a_placed_task_is_under_way_then() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        // A seeded generator, so the run is the same every time.
        let mut seed = 7_u64;
        let mut next = |below: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % below
        };
        let slot = |day: i64, minutes: u16| Slot { day, minutes };
        for _ in 0..200 {
            let placements: Vec<Placement> = (0..next(5))
                .map(|n| {
                    let (day, minutes) = (next(4) as i64, (8 * 60 + next(36) * 15) as u16);
                    let start = drop_start(&ny, day, minutes).unwrap();
                    Placement {
                        task: todo_core::TaskId(format!("t{n}")),
                        start,
                        end: start + chrono::Duration::minutes(15 * (1 + next(8) as i64)),
                    }
                })
                .collect();
            for day in 0..4 {
                for minutes in (8 * 60..20 * 60).step_by(15) {
                    let instant = drop_start(&ny, day, minutes).unwrap();
                    let under_way = placements
                        .iter()
                        .any(|placement| placement.start <= instant && instant < placement.end);
                    assert_eq!(
                        free_at(&placements, &ny, slot(day, minutes)),
                        !under_way,
                        "day {day} at {minutes}: {placements:?}"
                    );
                }
            }
        }
        // With nothing placed every hour is free, but never a time the clock skips.
        assert!(free_at(&[], &ny, slot(0, 9 * 60)));
        let spring = zone("America/New_York", at_utc(2026, 3, 5, 12, 0));
        assert!(!free_at(&[], &spring, slot(3, 2 * 60 + 30)));
    }

    #[test]
    fn each_placement_appears_only_on_its_day() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        let at = |day: u32, hour: u32| Placement {
            task: todo_core::TaskId(format!("t{day}{hour}")),
            start: at_utc(2026, 10, day, hour, 0),
            end: at_utc(2026, 10, day, hour + 1, 0),
        };
        let placements = vec![at(12, 14), at(13, 9), at(14, 16), at(12, 18)];
        let ids = |day| -> Vec<String> {
            placements_on_day(&placements, day, &ny)
                .into_iter()
                .map(|placement| placement.task.0)
                .collect()
        };
        assert_eq!(ids(0), vec!["t1214", "t1218"]);
        assert_eq!(ids(1), vec!["t139"]);
        assert_eq!(ids(2), vec!["t1416"]);
        assert!(ids(3).is_empty());
    }

    #[test]
    fn a_day_is_counted_in_the_users_zone_not_utc() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        // 02:00 UTC on 13 Oct is still 22:00 on 12 Oct in New York.
        assert_eq!(day_index(at_utc(2026, 10, 13, 2, 0), &ny), 0);
    }

    #[test]
    fn a_placement_either_side_of_the_berlin_clock_change_keeps_its_day() {
        // Europe/Berlin goes from UTC+2 to UTC+1 at 01:00 UTC on 25 Oct 2026. A single
        // offset read on 19 Oct would put 22:30 UTC on 25 Oct on 26 Oct.
        let berlin = zone("Europe/Berlin", at_utc(2026, 10, 19, 12, 0));
        // 22:30 UTC on 25 Oct is 23:30 on 25 Oct in Berlin, after the change.
        assert_eq!(day_index(at_utc(2026, 10, 25, 22, 30), &berlin), 6);
        // 21:30 UTC on 24 Oct is 23:30 on 24 Oct in Berlin, before it.
        assert_eq!(day_index(at_utc(2026, 10, 24, 21, 30), &berlin), 5);
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

    #[test]
    fn the_planner_opens_at_eight_or_now_never_the_night() {
        // The brief: opens at about 08:00 — not 02:00.
        assert_eq!(planner_open_minutes(2 * 60, 0, 24), 8 * 60);
        // Later than eight: the clock wins.
        assert_eq!(planner_open_minutes(12 * 60 + 37, 0, 24), 12 * 60 + 37);
        // A working day that starts later clamps at its own first hour.
        assert_eq!(planner_open_minutes(2 * 60, 9, 18), 9 * 60);
        assert_eq!(planner_open_minutes(10 * 60, 9, 18), 10 * 60);
        // The last full hour is as far down as it opens.
        assert_eq!(planner_open_minutes(23 * 60 + 59, 0, 24), 23 * 60);
    }

    #[test]
    fn the_days_read_as_weekdays_in_the_planner_zone() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        // 2026-10-12 is a Monday; day 0 shows the date, later days the count.
        assert_eq!(day_label(&ny, 0), "Mon 12");
        assert_eq!(day_label(&ny, 1), "Tue 13");
        assert_eq!(day_label(&ny, 4), "Fri 16");
        // The label is drawn in the planner's zone, not UTC: 23:30 UTC is
        // still Sunday the 18th in New York.
        let late = zone("America/New_York", at_utc(2026, 10, 18, 23, 30));
        assert_eq!(day_label(&late, 0), "Sun 18");
    }

    #[test]
    fn a_drop_snaps_to_the_quarter_hour_and_never_lands_at_the_day_end() {
        // 1.5 px past the 08:00 row = dead on 08:00.
        assert_eq!(drop_minute(1.5, 8, 18, HOUR_PX), 8 * 60);
        // 16 px of 64 is a quarter hour.
        assert_eq!(drop_minute(16.0, 8, 18, HOUR_PX), 8 * 60 + 15);
        // Half of 64px is 30 minutes.
        assert_eq!(drop_minute(32.0, 8, 18, HOUR_PX), 8 * 60 + 30);
        assert_eq!(
            drop_minute(49.0, 8, 18, HOUR_PX),
            8 * 60 + 45,
            "49px is 45.9min → the nearest quarter hour"
        );
        // Far past the bottom: pinned just inside the planner's last slot.
        assert_eq!(drop_minute(10_000.0, 8, 18, HOUR_PX), 17 * 60 + 45);
        assert_eq!(drop_minute(-5.0, 8, 18, HOUR_PX), 8 * 60);
        // An end at midnight never snaps to 24:00 as a start.
        assert_eq!(drop_minute(10_000.0, 8, 24, HOUR_PX), 23 * 60 + 45);
    }

    #[test]
    fn the_paper_slides_faster_the_nearer_the_held_slip_gets_to_an_edge() {
        // Nothing at all outside the edge band.
        assert_eq!(edge_nudge(EDGE_ZONE_PX + 10.0), 0.0);
        assert_eq!(
            edge_nudge(EDGE_ZONE_PX),
            0.0,
            "the band's edge counts as out"
        );
        // Halfway into the band: half the page's fastest nudge.
        assert!((edge_nudge(EDGE_ZONE_PX / 2.0) - EDGE_MAX_PX / 2.0).abs() < 0.01,);
        // At the edge itself, the loudest nudge.
        assert!((edge_nudge(0.0) - EDGE_MAX_PX).abs() < 0.01);
    }

    #[test]
    fn a_drop_starts_at_the_wall_clock_time_in_the_planner_zone() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        // Day 0, 09:30 in New York = 13:30 UTC.
        assert_eq!(
            drop_start(&ny, 0, 9 * 60 + 30),
            Some(at_utc(2026, 10, 12, 13, 30))
        );
        // Day 2, 23:30 → still the same local day, two days ahead.
        assert_eq!(
            drop_start(&ny, 2, 23 * 60 + 30),
            Some(at_utc(2026, 10, 15, 3, 30))
        );
    }

    #[test]
    fn an_occurrence_names_its_series_and_original_date() {
        // A row the planner expanded from a weekly rule.
        assert_eq!(
            occurrence_of(&TaskId("standup:2026-10-13".into())),
            Some(("standup".to_owned(), "2026-10-13".parse().unwrap()))
        );
        // A series id may itself carry a colon: split at the LAST one.
        assert_eq!(
            occurrence_of(&TaskId("team:sync:2026-10-13".into())),
            Some(("team:sync".to_owned(), "2026-10-13".parse().unwrap()))
        );
        // Plain task ids are not occurrences.
        assert_eq!(occurrence_of(&TaskId("te86966".into())), None);
        // ... and neither is anything whose trailing part is not a date.
        assert_eq!(occurrence_of(&TaskId("standup:next-week".into())), None);
    }

    #[test]
    fn a_time_the_clock_skips_cannot_start() {
        // America/New_York jumps 02:00→03:00 on 2026-03-08; 02:30 does not exist.
        let ny = zone("America/New_York", at_utc(2026, 3, 5, 12, 0));
        // day 3 from 2026-03-05 = 2026-03-08 itself.
        assert_eq!(drop_start(&ny, 3, 2 * 60 + 30), None);
        // Same day, 03:30 exists after the jump.
        assert!(drop_start(&ny, 3, 3 * 60 + 30).is_some());
    }

    #[test]
    fn an_hour_long_press_prefills_a_date_phrase_jev_parses() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        let at = |day: i64, minutes: u16| hour_prefill_text(&ny, day, minutes);
        assert_eq!(at(0, 18 * 60), "today 18:00 ");
        assert_eq!(at(1, 9 * 60 + 30), "tomorrow 09:30 ");
        // 2026-10-16 is a Friday.
        assert_eq!(at(4, 10 * 60), "friday 10:00 ");
    }

    #[test]
    fn a_moved_or_replanned_placement_changes_its_key_so_the_block_is_redrawn() {
        // The same slip, moved half an hour later inside its own day.
        let first = Placement {
            task: TaskId("t".into()),
            start: at_utc(2026, 10, 12, 14, 0),
            end: at_utc(2026, 10, 12, 14, 30),
        };
        let moved = Placement {
            start: at_utc(2026, 10, 12, 14, 30),
            end: at_utc(2026, 10, 12, 15, 0),
            ..first.clone()
        };
        assert_ne!(placement_key(&first), placement_key(&moved));
        // An unchanged placement keeps its key, so the row is not rebuilt for nothing.
        assert_eq!(placement_key(&first), placement_key(&first.clone()));
    }

    #[test]
    fn a_thirty_minute_slip_is_short_an_hour_long_one_is_not() {
        let ny = zone("America/New_York", at_utc(2026, 10, 12, 12, 0));
        let settings = Settings::default();
        let slip = |start: DateTime<Utc>, end: DateTime<Utc>| Placement {
            task: TaskId("t".into()),
            start,
            end,
        };
        // 14:00 UTC is 10:00 in New York: a half hour, then a full hour.
        let half = slip(at_utc(2026, 10, 12, 14, 0), at_utc(2026, 10, 12, 14, 30));
        let whole = slip(at_utc(2026, 10, 12, 14, 0), at_utc(2026, 10, 12, 15, 0));
        assert_eq!(
            placement_box(half.start, half.end, &ny, &settings).1,
            HOUR_PX / 2.0
        );
        assert!(
            placement_box(half.start, half.end, &ny, &settings).1 < SHORT_SLIP_PX,
            "half an hour gets its words beside the slip"
        );
        assert!(
            placement_box(whole.start, whole.end, &ny, &settings).1 >= SHORT_SLIP_PX,
            "an hour holds its own words"
        );
    }

    #[test]
    fn placement_style_0930_start_8am_64px() {
        // 09:30 start => 90 min after 8:00 => 90/60*64 = 96; 45 min => 48
        let (top, height) = placement_style(9 * 60 + 30, 9 * 60 + 75, 8, HOUR_PX, 18);
        assert!((top - 96.0).abs() < 0.01, "top={}, expected 96", top);
        assert!(
            (height - 48.0).abs() < 0.01,
            "height={}, expected 48",
            height
        );
    }

    #[test]
    fn placement_style_clipped_at_end() {
        let (top, height) = placement_style(17 * 60, 19 * 60, 8, HOUR_PX, 18);
        assert!((top - 9.0 * HOUR_PX).abs() < 0.01);
        assert!((height - 1.0 * HOUR_PX).abs() < 0.01);
    }

    #[test]
    fn placement_style_midnight_crossing_clipped_to_end() {
        // Task starts at 23:30 and ends at 01:30 next day; planner ends 24:00 (18 not used here, planner 8-24)
        let (top, height) = placement_style(23 * 60 + 30, 25 * 60 + 30, 8, HOUR_PX, 24);
        assert!((top - 15.5 * HOUR_PX).abs() < 0.01); // 23:30 = 15h30 after 8:00
        assert!((height - 0.5 * HOUR_PX).abs() < 0.01); // 30 min until 24:00
    }

    #[test]
    fn planner_days_cross_month_and_year_end() {
        let ny = zone("America/New_York", at_utc(2026, 12, 31, 12, 0));
        assert_eq!(day_index(at_utc(2026, 12, 31, 23, 0), &ny), 0);
        assert_eq!(day_index(at_utc(2027, 1, 1, 1, 0), &ny), 0);
        assert_eq!(day_index(at_utc(2027, 1, 1, 5, 0), &ny), 1);
    }

    #[test]
    fn placement_at_exact_planner_start_hour() {
        let (top, height) = placement_style(8 * 60, 9 * 60, 8, HOUR_PX, 18);
        assert!((top - 0.0).abs() < 0.01, "start at 08:00 = 0, got {top}");
        assert!((height - HOUR_PX).abs() < 0.01, "1h = 64, got {height}");
    }

    #[test]
    fn one_minute_placement() {
        let (top, height) = placement_style(10 * 60, 10 * 60 + 1, 8, HOUR_PX, 18);
        assert!(
            (height - (1.0 / 60.0 * HOUR_PX)).abs() < 0.01,
            "1 min = 1.07, got {height}"
        );
        assert!((top - 2.0 * HOUR_PX).abs() < 0.01, "10:00 = 2h, got {top}");
    }

    #[test]
    fn six_hundred_minute_placement() {
        let (top, height) = placement_style(8 * 60, 18 * 60, 8, HOUR_PX, 18);
        assert!((top - 0.0).abs() < 0.01);
        assert!(
            (height - 10.0 * HOUR_PX).abs() < 0.01,
            "10h = 640, got {height}"
        );
    }
}

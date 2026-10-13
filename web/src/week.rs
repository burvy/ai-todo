use chrono::{DateTime, Timelike, Utc};
use gloo_timers::callback::Interval;
use leptos::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use todo_core::{Clock, SystemClock};
use todo_core::{LocalContext, Placement, Snapshot, TaskId};

use crate::{mascot, settings::Settings};

#[component]
pub fn WeekGrid(
    snapshot: ReadSignal<Snapshot>,
    settings: ReadSignal<Settings>,
    set_mascot_state: WriteSignal<mascot::State>,
) -> impl IntoView {
    let grid_ref = NodeRef::<leptos::html::Div>::new();
    let (now_minutes, set_now_minutes) = signal(current_minutes());
    set_interval(move || set_now_minutes.set(current_minutes()), 60_000).forget();
    // The planner opens on the working morning, not in the middle of the
    // night: about 08:00, or now if it is already later.
    Effect::new(move |_| {
        let s = settings.get();
        if let Some(grid) = grid_ref.get() {
            #[cfg(target_arch = "wasm32")]
            {
                let target = planner_open_minutes(current_minutes(), s.start_hour, s.end_hour);
                grid.set_scroll_top(now_line_top(target, s.start_hour, HOUR_PX) as i32);
            }
            #[cfg(not(target_arch = "wasm32"))]
            let _ = (grid, s);
        }
    });
    let days = move || (0..settings.get().days).collect::<Vec<_>>();
    let hours = move || (settings.get().start_hour..settings.get().end_hour).collect::<Vec<_>>();
    view! {
        <section aria-labelledby="week-heading">
            <h2 id="week-heading">"Planner"</h2>
            <div class="week-scroll" node_ref=grid_ref>
                <div class="week" style=move || format!("--days: {}", settings.get().days)>
                    <For each=days key=|day| *day let:day>
                        <div class="day">
                            <h3 class="day-name" class:today=day == 0>
                                {day_label(&planner_context(&snapshot.get().tz), i64::from(day))}
                            </h3>
                            <div class="hours">
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
                            <For each=move || {
                                    // Re-read every minute, so "today" moves on at midnight.
                                    let _ = now_minutes.get();
                                    let snapshot = snapshot.get();
                                    let ctx = planner_context(&snapshot.tz);
                                    placements_on_day(&snapshot.plan.placements, i64::from(day), &ctx)
                                }
                                key=|placement| placement.task.clone() let:placement>
                                <PlacementBlock placement snapshot settings set_mascot_state />
                            </For>
                                <For each=hours key=|hour| *hour let:hour>
                                    <div class="hour">
                                        {(day == 0).then(|| view! {
                                            <span>{format!("{hour:02}:00")}</span>
                                        })}
                                    </div>
                                </For>
                            </div>
                        </div>
                    </For>
                </div>
            </div>
        </section>
    }
}

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

/// A placement's start as the planner shows it: weekday and clock time in the zone.
pub(crate) fn local_time_label(ctx: &LocalContext, start: DateTime<Utc>) -> String {
    ctx.local(start).format("%a %H:%M").to_string()
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

fn current_minutes() -> u16 {
    #[cfg(target_arch = "wasm32")]
    {
        let now = js_sys::Date::new_0();
        return now.get_hours() as u16 * 60 + now.get_minutes() as u16;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0
    }
}

fn set_interval(callback: impl FnMut() + 'static, millis: u32) -> Interval {
    Interval::new(millis, callback)
}

#[component]
fn PlacementBlock(
    placement: Placement,
    snapshot: ReadSignal<Snapshot>,
    settings: ReadSignal<Settings>,
    set_mascot_state: WriteSignal<mascot::State>,
) -> impl IntoView {
    let task_id = placement.task.clone();
    let label_task_id = task_id.clone();
    let render_time = local_time_label(&planner_context(&snapshot.get().tz), placement.start);
    view! {
        <div class="placement" style=move || {
            let s = settings.get();
            let ctx = planner_context(&snapshot.get().tz);
            let local_start = ctx.local(placement.start);
            let local_end = ctx.local(placement.end);
            let start_min = local_start.hour() as u16 * 60 + local_start.minute() as u16;
            let end_min = if local_end.date() > local_start.date() {
                s.end_hour as u16 * 60
            } else {
                local_end.hour() as u16 * 60 + local_end.minute() as u16
            };
            let (top, height) = placement_style(start_min, end_min, s.start_hour, HOUR_PX, s.end_hour);
            format!("top: {}px; height: {}px;", top, height)
        }
            on:drop=move |_| set_mascot_state.update(|state| {
                if matches!(state, mascot::State::Dragging)
                    && let Some(task) = snapshot.get().tasks.iter().find(|task| task.id == task_id)
                {
                    *state = mascot::transition(state, mascot::Event::DropTask {
                        task_id: task.id.0.clone(), title: task.title.clone()
                    });
                }
            })
            on:dragover=|event| event.prevent_default()
        >
            {move || view! { <strong>{placement_label(&snapshot.get(), &label_task_id)}</strong> }}
            " · " {render_time}
        </div>
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
            local_time_label(&ny, at_utc(2026, 10, 12, 14, 0)),
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

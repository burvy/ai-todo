use chrono::{DateTime, NaiveDate, Utc};
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use todo_core::{Placement, Snapshot};

use crate::{api, mascot, settings::Settings};

#[component]
pub fn WeekGrid(
    snapshot: ReadSignal<Snapshot>,
    settings: ReadSignal<Settings>,
    set_mascot_state: WriteSignal<mascot::State>,
) -> impl IntoView {
    let grid_ref = NodeRef::<leptos::html::Div>::new();
    let (now_minutes, set_now_minutes) = signal(current_minutes());
    set_interval(move || set_now_minutes.set(current_minutes()), 60_000).forget();
    Effect::new(move |_| {
        let _ = settings.get();
        if let Some(_grid) = grid_ref.get() {
            #[cfg(target_arch = "wasm32")]
            _grid.set_scroll_top((js_sys::Date::new_0().get_hours() as f32 * HOUR_PX) as i32);
        }
    });
    // The planner's first day is today on this device. Placements are instants, so each
    // one is bucketed by its date in the browser's zone.
    let today = NaiveDate::parse_from_str(&api::today_date(), "%Y-%m-%d").ok();
    let offset = zone_offset_minutes();
    let days = move || (0..settings.get().days).collect::<Vec<_>>();
    let hours = move || (settings.get().start_hour..settings.get().end_hour).collect::<Vec<_>>();
    view! {
        <section aria-labelledby="week-heading">
            <h2 id="week-heading">"Planner"</h2>
            <div class="week-scroll" node_ref=grid_ref>
                <div class="week" style=move || format!("--days: {}", settings.get().days)>
                    <For each=days key=|day| *day let:day>
                        <div class="day">
                            <h3>{format!("Day {}", day + 1)}</h3>
                            <For each=move || today.map_or_else(Vec::new, |today| {
                                    placements_on_day(&snapshot.get().plan.placements, i64::from(day), offset, today)
                                })
                                key=|placement| placement.task.clone() let:placement>
                                <PlacementBlock placement snapshot set_mascot_state />
                            </For>
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
                                <For each=hours key=|hour| *hour let:hour>
                                    <div class="hour"><span>{format!("{hour:02}:00")}</span></div>
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

/// The day column a placement falls in: its date in the zone `offset_minutes` east of
/// UTC, counted from `today`. Day 0 is today.
fn day_index(start: DateTime<Utc>, offset_minutes: i32, today: NaiveDate) -> i64 {
    let local = start + chrono::Duration::minutes(i64::from(offset_minutes));
    (local.date_naive() - today).num_days()
}

/// The placements that fall on planner day `day`, in their original order.
fn placements_on_day(
    placements: &[Placement],
    day: i64,
    offset_minutes: i32,
    today: NaiveDate,
) -> Vec<Placement> {
    placements
        .iter()
        .filter(|placement| day_index(placement.start, offset_minutes, today) == day)
        .cloned()
        .collect()
}

/// The browser's zone offset, in minutes east of UTC.
fn zone_offset_minutes() -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        -(js_sys::Date::new_0().get_timezone_offset() as i32)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0
    }
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
    set_mascot_state: WriteSignal<mascot::State>,
) -> impl IntoView {
    let task_id = placement.task.clone();
    let render_task_id = task_id.clone();
    let render_time = placement.start.format("%a %H:%M").to_string();
    view! {
        <div class="placement"
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
        >{render_task_id.0} " " {render_time}</div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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

    #[test]
    fn a_placement_lands_on_its_own_local_day() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        // 23:30 UTC on 12 Oct is 19:30 on 12 Oct in New York (UTC-4): today.
        let evening = Utc.with_ymd_and_hms(2026, 10, 12, 23, 30, 0).unwrap();
        assert_eq!(day_index(evening, -240, today), 0);
        // The same instant is 00:30 on 13 Oct in Berlin (UTC+2 in October): tomorrow.
        assert_eq!(day_index(evening, 120, today), 1);
    }

    #[test]
    fn each_placement_appears_only_on_its_day() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let at = |day: u32, hour: u32| Placement {
            task: todo_core::TaskId(format!("t{day}{hour}")),
            start: Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 10, day, hour + 1, 0, 0).unwrap(),
        };
        let placements = vec![at(12, 14), at(13, 9), at(14, 16), at(12, 18)];
        let ids = |day| -> Vec<String> {
            placements_on_day(&placements, day, 0, today)
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
        let today = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        // 02:00 UTC on 13 Oct is still 22:00 on 12 Oct in New York.
        let late = Utc.with_ymd_and_hms(2026, 10, 13, 2, 0, 0).unwrap();
        assert_eq!(day_index(late, -240, today), 0);
    }
}

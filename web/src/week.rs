use gloo_timers::callback::Interval;
use leptos::prelude::*;
use todo_core::{Placement, Snapshot};

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
    Effect::new(move |_| {
        let _ = settings.get();
        if let Some(_grid) = grid_ref.get() {
            #[cfg(target_arch = "wasm32")]
            _grid.set_scroll_top((js_sys::Date::new_0().get_hours() as i32 * 64).max(0));
        }
    });
    let days = move || (0..settings.get().days).collect::<Vec<_>>();
    let hours = move || (settings.get().start_hour..settings.get().end_hour).collect::<Vec<_>>();
    view! {
        <section aria-labelledby="week-heading">
            <h2 id="week-heading">"Week"</h2>
            <div class="week-scroll" node_ref=grid_ref>
                <div class="week" style=move || format!("--days: {}", settings.get().days)>
                    <For each=days key=|day| *day let:day>
                        <div class="day">
                            <h3>{format!("Day {}", day + 1)}</h3>
                            {move || (day == 0).then(|| view! {
                                <div
                                    class="now-line"
                                    style=move || format!(
                                        "top: {}px",
                                        (((now_minutes.get() as i32
                                            - settings.get().start_hour as i32 * 60)
                                            .max(0) as f32 * 64.0 / 60.0) + 28.0)
                                    )
                                    aria-label="Current time"
                                ></div>
                            })}
                            <For each=move || snapshot.get().plan.placements.clone()
                                key=|placement| placement.task.clone() let:placement>
                                <PlacementBlock placement snapshot set_mascot_state />
                            </For>
                            <For each=hours key=|hour| *hour let:hour>
                                <div class="hour"><span>{format!("{hour:02}:00")}</span></div>
                            </For>
                        </div>
                    </For>
                </div>
            </div>
        </section>
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

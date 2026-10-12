use leptos::prelude::*;
use todo_core::Placement;
use todo_core::Snapshot;

pub mod api;
pub mod mascot;

#[component]
pub fn App() -> impl IntoView {
    let snapshot = api::use_snapshot();
    let (mascot_state, set_mascot_state) = signal(mascot::State::Home);
    view! {
        <header><h1>"Rambl"</h1><CaptureBox state=mascot_state set_state=set_mascot_state /></header>
        <main>
            <TaskList snapshot=snapshot set_mascot_state=set_mascot_state />
            <WeekGrid snapshot=snapshot set_mascot_state=set_mascot_state />
        </main>
        <Mascot state=mascot_state set_state=set_mascot_state />
    }
}

#[component]
fn CaptureBox(
    state: ReadSignal<mascot::State>,
    set_state: WriteSignal<mascot::State>,
) -> impl IntoView {
    view! {
        <form class="capture" on:submit=|ev| ev.prevent_default()>
            <input placeholder="Say what you need to do…" aria-label="Ramble" />
            <button class="mascot-home" type="button" aria-label="Talk with Rambl"
                on:pointerdown=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Talk))>
                <img src="assets/pen.svg" alt="Rambl" />
            </button>
            <button type="submit">"Add"</button>
        </form>
    }
}

#[component]
fn Mascot(
    state: ReadSignal<mascot::State>,
    set_state: WriteSignal<mascot::State>,
) -> impl IntoView {
    view! {
        <span class="mascot" draggable="true"
            on:dragstart=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Grab))
            on:dragend=move |_| {
                if matches!(state.get(), mascot::State::Dragging) {
                    set_state.set(mascot::transition(&state.get(), mascot::Event::DropEmpty));
                    set_state.set(mascot::transition(&state.get(), mascot::Event::Return));
                    set_state.set(mascot::transition(&state.get(), mascot::Event::Return));
                }
            }
            on:pointerdown=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Talk))>
            <img src="assets/pen.svg" alt="Rambl" />
        </span>
        <span class="mascot-status" aria-live="polite">
            {move || match state.get() {
                mascot::State::Asking { ref title, .. } => view! {
                    <span class="ask">
                        <span>{format!("Okay, what about {title}?")}</span>
                        <button type="button" on:click=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Choose))>"Edit"</button>
                        <button type="button" on:click=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Choose))>"Move"</button>
                        <button type="button" on:click=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Choose))>"Done"</button>
                        <button type="button" on:click=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Choose))>"Delete"</button>
                        <button type="button" on:click=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Return))>"Never mind"</button>
                    </span>
                }.into_any(),
                _ => view! { <span></span> }.into_any(),
            }}
        </span>
    }
}

#[component]
fn TaskList(
    snapshot: ReadSignal<Snapshot>,
    set_mascot_state: WriteSignal<mascot::State>,
) -> impl IntoView {
    view! {
        <section aria-labelledby="tasks-heading">
            <h2 id="tasks-heading">"Tasks"</h2>
            <ul>
                <For each=move || snapshot.get().tasks key=|task| task.id.clone() let:task>
                    {
                        let task_id = task.id.0.clone();
                        let task_title = task.title.clone();
                        let drop_task_id = task_id.clone();
                        let drop_task_title = task_title.clone();
                        let render_task_id = task_id.clone();
                        let render_task_title = task_title.clone();
                        view! {
                    <li
                        on:drop=move |_| set_mascot_state.update(|state| {
                            if matches!(state, mascot::State::Dragging) {
                                *state = mascot::transition(state, mascot::Event::DropTask {
                                    task_id: drop_task_id.clone(), title: drop_task_title.clone()
                                });
                            }
                        })
                        on:dragover=|ev| ev.prevent_default()
                        draggable="true"
                    ><strong>{render_task_title}</strong> " — " {task.duration_min} " min"
                        {move || task_time(&snapshot.get(), &render_task_id)}
                    </li>
                        }
                    }
                </For>
            </ul>
        </section>
    }
}

#[component]
fn WeekGrid(
    snapshot: ReadSignal<Snapshot>,
    set_mascot_state: WriteSignal<mascot::State>,
) -> impl IntoView {
    view! {
        <section aria-labelledby="week-heading">
            <h2 id="week-heading">"Week"</h2>
            <div class="week">
                <For each=move || week_days() key=|day| *day let:day>
                    <div class="day"><h3>{format!("Day {}", day + 1)}</h3>
                        <For each=move || snapshot.get().plan.placements.clone()
                            key=|placement| placement.task.clone() let:placement>
                            <div class="placement"
                                on:drop=move |_| set_mascot_state.update(|state| {
                                    if matches!(state, mascot::State::Dragging) {
                                        if let Some(task) = snapshot.get().tasks.iter().find(|task| task.id == placement.task) {
                                            *state = mascot::transition(state, mascot::Event::DropTask {
                                                task_id: task.id.0.clone(), title: task.title.clone()
                                            });
                                        }
                                    }
                                })
                                on:dragover=|ev| ev.prevent_default()
                            >{placement.task.0.clone()} " " {placement_time(&placement)}</div>
                        </For>
                        <div class="hour"></div><div class="hour"></div><div class="hour"></div>
                    </div>
                </For>
            </div>
        </section>
    }
}

fn task_time(snapshot: &Snapshot, id: &str) -> String {
    snapshot
        .plan
        .placements
        .iter()
        .find(|p| p.task.0 == id)
        .map(|p| p.start.format("%a %H:%M").to_string())
        .unwrap_or_default()
}

fn placement_time(placement: &Placement) -> String {
    placement.start.format("%a %H:%M").to_string()
}

fn week_days() -> Vec<u8> {
    (0..7).collect()
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    leptos::mount::mount_to_body(App);
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn list_and_week_use_the_same_task_time() {
        let snapshot: Snapshot =
            serde_json::from_str(include_str!("../fixtures/snapshot.json")).unwrap();
        for task in &snapshot.tasks {
            let list_time = task_time(&snapshot, &task.id.0);
            let week_time = snapshot
                .plan
                .placements
                .iter()
                .find(|p| p.task == task.id)
                .map(placement_time)
                .unwrap_or_default();
            assert_eq!(list_time, week_time);
        }
    }
}

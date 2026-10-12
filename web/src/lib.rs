use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use todo_core::Snapshot;
use wasm_bindgen::JsCast;

pub mod api;
pub mod mascot;
pub mod settings;
pub mod week;

#[component]
pub fn App() -> impl IntoView {
    let snapshot = api::use_snapshot();
    let (view_settings, set_view_settings) = signal(settings::load());
    let (mascot_state, set_mascot_state) = signal(mascot::State::Home);
    let (talking, set_talking) = signal(false);
    window_event_listener(leptos::ev::keydown, move |event| {
        if event.key() == " "
            && !matches!(event.target(), Some(target) if target.dyn_ref::<web_sys::HtmlInputElement>().is_some()
                || target.dyn_ref::<web_sys::HtmlTextAreaElement>().is_some())
        {
            set_talking.set(true);
        }
    });
    view! {
        <header><h1>"Rambl"</h1><CaptureBox state=mascot_state set_state=set_mascot_state set_talking /></header>
        <settings::SettingsPanel settings=view_settings set_settings=set_view_settings />
        <main>
            <TaskList snapshot=snapshot set_mascot_state=set_mascot_state />
            <week::WeekGrid snapshot=snapshot settings=view_settings set_mascot_state=set_mascot_state />
        </main>
        <Mascot state=mascot_state set_state=set_mascot_state talking set_talking />
    }
}

#[component]
fn CaptureBox(
    state: ReadSignal<mascot::State>,
    set_state: WriteSignal<mascot::State>,
    set_talking: WriteSignal<bool>,
) -> impl IntoView {
    view! {
        <form class="capture" on:submit=|ev| ev.prevent_default()>
            <input placeholder="Say what you need to do…" aria-label="Ramble" />
            <button class="mascot-home" type="button" aria-label="Talk with Rambl"
                on:pointerdown=move |_| {
                    set_state.set(mascot::transition(&state.get(), mascot::Event::Talk));
                    set_talking.set(true);
                }>
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
    talking: ReadSignal<bool>,
    set_talking: WriteSignal<bool>,
) -> impl IntoView {
    let (animation, set_animation) = signal("home");
    let start_return = move || {
        if mascot::prefers_reduced_motion() {
            set_state.set(mascot::State::Home);
            set_animation.set("home");
            return;
        }
        set_animation.set("falling");
        set_state.set(mascot::State::Falling);
        let set_state = set_state;
        let set_animation = set_animation;
        Timeout::new(450, move || {
            set_state.set(mascot::State::Returning);
            set_animation.set("returning");
            let set_state = set_state;
            let set_animation = set_animation;
            Timeout::new(450, move || {
                set_state.set(mascot::State::Home);
                set_animation.set("home");
            })
            .forget();
        })
        .forget();
    };
    view! {
        <span class=move || format!("mascot {}", animation.get()) draggable="true"
            on:dragstart=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Grab))
            on:dragend=move |_| {
                if matches!(state.get(), mascot::State::Dragging) {
                    start_return();
                }
            }
            on:pointerdown=move |_| {
                set_state.set(mascot::transition(&state.get(), mascot::Event::Talk));
                set_talking.set(true);
            }>
            <img src="assets/pen.svg" alt="Rambl" />
        </span>
        <span class="mascot-status" aria-live="polite">
            {move || match state.get() {
                mascot::State::Asking { ref title, .. } => view! {
                    <span class="ask">
                        <span>{format!("Hey! What do you want with \"{title}\"?")}</span>
                        // TODO: wire these to PUT /tasks/{id}, PUT /tasks/{id}/placement,
                        // POST /tasks/{id}/done, and DELETE /tasks/{id}.
                        <button type="button" on:click=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Choose))>"Edit"</button>
                        <button type="button" on:click=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Choose))>"Move"</button>
                        <button type="button" on:click=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Choose))>"Done"</button>
                        <button type="button" on:click=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Choose))>"Delete"</button>
                        <button type="button" on:click=move |_| set_state.set(mascot::transition(&state.get(), mascot::Event::Return))>"Never mind"</button>
                    </span>
                }.into_any(),
                _ => view! { <span></span> }.into_any(),
            }}
            {move || talking.get().then(|| view! {
                <span class="talking">"I'm listening."</span>
            })}
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

fn task_time(snapshot: &Snapshot, id: &str) -> String {
    snapshot
        .plan
        .placements
        .iter()
        .find(|p| p.task.0 == id)
        .map(|p| p.start.format("%a %H:%M").to_string())
        .unwrap_or_default()
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
                .map(|placement| placement.start.format("%a %H:%M").to_string())
                .unwrap_or_default();
            assert_eq!(list_time, week_time);
        }
    }
}

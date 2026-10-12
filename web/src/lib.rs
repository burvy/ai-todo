use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use todo_core::Snapshot;
use todo_core::{Label, Priority, Source, TaskDraft, TaskId, When};
use wasm_bindgen::JsCast;

pub mod api;
pub mod mascot;
pub mod settings;
pub mod week;

/// Load snapshot from API and return it, falling back to fixture on error
async fn load_snapshot_from_api() -> Snapshot {
    match api::fetch_snapshot(&api::today_date(), 7).await {
        Ok(s) => s,
        Err(_) => api::fixture_snapshot(),
    }
}

#[component]
pub fn App() -> impl IntoView {
    let (snapshot, set_snapshot) = signal(api::fixture_snapshot());
    let (view_settings, set_view_settings) = signal(settings::load());
    let (error, set_error) = signal::<Option<String>>(None);

    // Load snapshot from API on mount
    Effect::new(move |_| {
        wasm_bindgen_futures::spawn_local(async move {
            let new_snapshot = load_snapshot_from_api().await;
            set_snapshot.set(new_snapshot);
        });
    });

    let (mascot_state, set_mascot_state) = signal(mascot::State::Home);
    let (talking, set_talking) = signal(false);

    window_event_listener(leptos::ev::keydown, move |event| {
        if event.key() == " "
            && !matches!(event.target(), Some(target) if 
                target.dyn_ref::<web_sys::HtmlInputElement>().is_some()
                || target.dyn_ref::<web_sys::HtmlTextAreaElement>().is_some())
        {
            set_talking.set(true);
        }
    });

    view! {
        <header>
            <h1>"Rambl"</h1>
            <CaptureBox
                set_snapshot=set_snapshot
                state=mascot_state
                set_state=set_mascot_state
                set_talking
            />
        </header>
        {move || error.get().map(|e| view! {
            <div class="error" aria-live="assertive">{e}</div>
        })}
        <settings::SettingsPanel settings=view_settings set_settings=set_view_settings />
        <main>
            <ImportIcs set_snapshot set_error />
            <TaskList snapshot set_snapshot set_mascot_state />
            <week::WeekGrid snapshot=snapshot settings=view_settings set_mascot_state=set_mascot_state />
        </main>
        <Mascot
            state=mascot_state
            set_state=set_mascot_state
            talking
            set_talking
            snapshot
            set_snapshot
        />
    }
}

#[component]
fn CaptureBox(
    set_snapshot: WriteSignal<Snapshot>,
    state: ReadSignal<mascot::State>,
    set_state: WriteSignal<mascot::State>,
    set_talking: WriteSignal<bool>,
) -> impl IntoView {
    let (text, set_text) = signal(String::new());
    let (speech_supported, set_speech_supported) = signal(false);
    let (listening, set_listening) = signal(false);

    // Check if SpeechRecognition is supported
    Effect::new(move |_| {
        set_speech_supported.set(
            web_sys::window()
                .map(|w| {
                    w.get("SpeechRecognition").is_some()
                        || w.get("webkitSpeechRecognition").is_some()
                })
                .unwrap_or(false),
        );
    });

    let handle_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let text_val = text.get_untracked();
        if !text_val.is_empty() {
            wasm_bindgen_futures::spawn_local(async move {
                let draft = TaskDraft {
                    title: text_val,
                    duration_min: 30,
                    when: When::Anytime,
                    label: Label::Other,
                    priority: Priority::Medium,
                    source: Source::Text,
                };
                if let Ok(new_snapshot) = api::create_task(draft).await {
                    set_snapshot.set(new_snapshot);
                }
                set_text.set(String::new());
            });
        }
    };

    view! {
        <form class="capture" on:submit=handle_submit>
            <input
                prop:value=text
                on:input=move |ev| set_text.set(event_target_value(&ev))
                placeholder="Say what you need to do…"
                aria-label="Ramble"
            />
            {move || {
                if speech_supported.get() {
                    view! {
                        <button
                            type="button"
                            aria-label="Speak to capture"
                            class=move || if listening.get() { "mic listening" } else { "mic" }
                            on:click=move |_| {
                                set_listening.set(true);
                                set_talking.set(true);
                                // Speech recognition would be implemented here
                                // For now, just toggle the state
                            }
                        >"Mic"</button>
                    }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }
            }}
            <button
                class="mascot-home"
                type="button"
                aria-label="Talk with Rambl"
                on:pointerdown=move |_| {
                    set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Talk));
                    set_talking.set(true);
                }
            >
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
    snapshot: ReadSignal<Snapshot>,
    set_snapshot: WriteSignal<Snapshot>,
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
        <span
            class=move || format!("mascot {}", animation.get())
            draggable="true"
            on:dragstart=move |_| set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Grab))
            on:dragend=move |_| {
                if matches!(state.get_untracked(), mascot::State::Dragging) {
                    start_return();
                }
            }
            on:pointerdown=move |_| {
                set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Talk));
                set_talking.set(true);
            }
        >
            <img src="assets/pen.svg" alt="Rambl" />
        </span>
        <span class="mascot-status" aria-live="polite">
            {move || match state.get_untracked() {
                mascot::State::Asking { ref title, .. } => view! {
                    <span class="ask">
                        <span>{format!("Hey! What do you want with \"{title}\"?")}</span>
                        <button type="button" on:click=move |_| {
                            if let mascot::State::Asking { ref task_id, .. } = state.get_untracked() {
                                let task_id_str = task_id.clone();
                                let snapshot_val = snapshot.get_untracked();
                                let set_snapshot_clone = set_snapshot;
                                let set_state_clone = set_state;
                                wasm_bindgen_futures::spawn_local(async move {
                                    if let Some(task) = snapshot_val.tasks.iter().find(|t| t.id.0 == task_id_str) {
                                        let new_priority = match task.priority {
                                            todo_core::Priority::Low => todo_core::Priority::Medium,
                                            todo_core::Priority::Medium => todo_core::Priority::High,
                                            todo_core::Priority::High => todo_core::Priority::Low,
                                            todo_core::Priority::Critical => todo_core::Priority::High,
                                        };
                                        let body = api::PatchTaskBody::new().priority(new_priority);
                                        if let Ok(new_snap) = api::patch_task(&task.id, body).await {
                                            set_snapshot_clone.set(new_snap);
                                        }
                                    }
                                    set_state_clone.set(mascot::State::Home);
                                });
                            }
                        }>"Edit"</button>
                        <button type="button" on:click=move |_| {
                            if let mascot::State::Asking { .. } = state.get_untracked() {
                                set_state.set(mascot::State::Home);
                            }
                        }>"Move"</button>
                        <button type="button" on:click=move |_| {
                            if let mascot::State::Asking { ref task_id, .. } = state.get_untracked() {
                                let task_id = task_id.clone();
                                let snapshot_val = snapshot.get_untracked();
                                let set_snapshot_clone = set_snapshot;
                                let set_state_clone = set_state;
                                wasm_bindgen_futures::spawn_local(async move {
                                    if let Some(task) = snapshot_val.tasks.iter().find(|t| t.id.0 == task_id) {
                                        let body = api::PatchTaskBody::new().done(!task.done);
                                        if let Ok(new_snap) = api::patch_task(&task.id, body).await {
                                            set_snapshot_clone.set(new_snap);
                                        }
                                    }
                                    set_state_clone.set(mascot::State::Home);
                                });
                            }
                        }>"Done"</button>
                        <button type="button" on:click=move |_| {
                            if let mascot::State::Asking { ref task_id, .. } = state.get_untracked() {
                                let task_id = task_id.clone();
                                let set_snapshot_clone = set_snapshot;
                                let set_state_clone = set_state;
                                wasm_bindgen_futures::spawn_local(async move {
                                    if let Ok(new_snap) = api::delete_task(&TaskId(task_id)).await {
                                        set_snapshot_clone.set(new_snap);
                                    }
                                    set_state_clone.set(mascot::State::Home);
                                });
                            }
                        }>"Delete"</button>
                        <button type="button" on:click=move |_| set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Return))>"Never mind"</button>
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
fn ImportIcs(
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
) -> impl IntoView {
    let handle_import = move |ev: leptos::ev::Event| {
        let set_snapshot_clone = set_snapshot;
        let set_error_clone = set_error;
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(target) = ev.target()
                && let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>()
                && let Some(files) = input.files()
                && files.length() > 0
                && let Some(file) = files.get(0)
            {
                match api::import_ics(file).await {
                    Ok(response) => {
                        set_snapshot_clone.set(response.snapshot);
                        set_error_clone.set(None);
                    }
                    Err(e) => {
                        set_error_clone.set(Some(format!("Import failed: {}", e)));
                    }
                }
            }
        });
    };

    view! {
        <div class="import">
            <label>
                "Import .ics: "
                <input
                    type="file"
                    accept=".ics"
                    on:change=handle_import
                    aria-label="Import calendar file"
                />
            </label>
        </div>
    }
}

#[component]
fn TaskList(
    snapshot: ReadSignal<Snapshot>,
    set_snapshot: WriteSignal<Snapshot>,
    set_mascot_state: WriteSignal<mascot::State>,
) -> impl IntoView {
    let (dragged_task, set_dragged_task) = signal::<Option<TaskId>>(None);

    view! {
        <section aria-labelledby="tasks-heading">
            <h2 id="tasks-heading">"Tasks"</h2>
            <ul>
                <For each=move || snapshot.get().tasks key=|task| task.id.clone() let:task>
                    {
                        let task_id = task.id.clone();
                        let task_title = task.title.clone();
                        let drop_task_id = task_id.0.clone();
                        let drop_task_title = task_title.clone();
                        let render_task_id = task_id.0.clone();
                        let render_task_title = task_title.clone();

                        let toggle_done = {
                            let task_id_clone = task_id.clone();
                            move |_| {
                                let task_id_clone = task_id_clone.clone();
                                let set_snapshot_clone = set_snapshot;
                                wasm_bindgen_futures::spawn_local(async move {
                                    let body = api::PatchTaskBody::new().done(!task.done);
                                    if let Ok(new_snap) = api::patch_task(&task_id_clone, body).await {
                                        set_snapshot_clone.set(new_snap);
                                    }
                                });
                            }
                        };

                        let delete_task = {
                            let task_id_clone = task_id.clone();
                            move |ev: leptos::ev::MouseEvent| {
                                ev.prevent_default();
                                ev.stop_propagation();
                                let task_id_clone = task_id_clone.clone();
                                let set_snapshot_clone = set_snapshot;
                                wasm_bindgen_futures::spawn_local(async move {
                                    if let Ok(new_snap) = api::delete_task(&task_id_clone).await {
                                        set_snapshot_clone.set(new_snap);
                                    }
                                });
                            }
                        };

                        let handle_drag_start = {
                            let task_id_clone = task_id.clone();
                            move |_| {
                                set_dragged_task.set(Some(task_id_clone.clone()));
                            }
                        };

                        let handle_drag_end = move |_| {
                            set_dragged_task.set(None);
                        };

                        let handle_drop = {
                            let task_id_clone = task_id.clone();
                            let set_mascot_state_clone = set_mascot_state;
                            let set_snapshot_clone = set_snapshot;
                            let drop_task_id_clone = drop_task_id.clone();
                            let drop_task_title_clone = drop_task_title.clone();
                            move |ev: leptos::ev::DragEvent| {
                                ev.prevent_default();
                                // Handle mascot drop
                                set_mascot_state_clone.update(|state| {
                                    if matches!(state, mascot::State::Dragging) {
                                        *state = mascot::transition(state, mascot::Event::DropTask {
                                            task_id: drop_task_id_clone.clone(), title: drop_task_title_clone.clone()
                                        });
                                    }
                                });

                                // Handle task reorder drop
                                if let Some(dragged_id) = dragged_task.get()
                                    && dragged_id != task_id_clone
                                {
                                    let snapshot_tasks = snapshot.get().tasks;

                                    // Reorder: move dragged task to position of dropped task
                                    let mut new_order = Vec::new();
                                    let mut found_target = false;

                                    for t in &snapshot_tasks {
                                        if t.id == dragged_id {
                                            continue;
                                        }
                                        if t.id == task_id_clone && !found_target {
                                            new_order.push(dragged_id.clone());
                                            found_target = true;
                                        }
                                        new_order.push(t.id.clone());
                                    }
                                    if !found_target {
                                        new_order.push(dragged_id.clone());
                                    }

                                    wasm_bindgen_futures::spawn_local(async move {
                                        if let Ok(new_snap) = api::reorder_tasks(new_order).await {
                                            set_snapshot_clone.set(new_snap);
                                        }
                                    });
                                }
                            }
                        };

                        let handle_drag_over = |ev: leptos::ev::DragEvent| {
                            ev.prevent_default();
                        };

                        view! {
                    <li
                        on:drop=handle_drop
                        on:dragover=handle_drag_over
                        draggable="true"
                        on:dragstart=handle_drag_start
                        on:dragend=handle_drag_end
                    >
                        <input
                            type="checkbox"
                            checked=task.done
                            on:change=toggle_done
                            aria-label=format!("Mark {} as done", task_title)
                        />
                        <strong>{render_task_title}</strong> " — " {task.duration_min} " min"
                        {move || task_time(&snapshot.get(), &render_task_id)}
                        <button
                            class="delete"
                            type="button"
                            on:click=delete_task
                            aria-label=format!("Delete {}", task_title)
                        >"Delete"</button>
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

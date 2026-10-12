use chrono::{DateTime, Utc};
use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use todo_core::Snapshot;
use todo_core::{Label, Priority, Source, TaskDraft, TaskId, When};
use wasm_bindgen::JsCast;

pub mod account;
pub mod api;
pub mod brain;
pub mod mascot;
pub mod settings;
pub mod speech;
pub mod week;

/// Load snapshot from API and return it, falling back to fixture on error
async fn load_snapshot_from_api(days: u8) -> Snapshot {
    match api::fetch_snapshot(&api::today_date(), days).await {
        Ok(s) => s,
        Err(_) => api::fixture_snapshot(),
    }
}

/// The task `id` as the list holds it right now.
///
/// A row keeps the `Task` it was first drawn with, so an edit must read the task here,
/// at click time: its `updated_at` moves on with every accepted change, and sending the
/// old one is refused as stale.
fn held<'a>(snapshot: &'a Snapshot, id: &str) -> Option<&'a todo_core::Task> {
    snapshot.tasks.iter().find(|task| task.id.0 == id)
}

/// The body that flips `id` between done and not done, against the version held now.
fn toggle_done_body(snapshot: &Snapshot, id: &str) -> Option<api::PatchTaskBody> {
    let task = held(snapshot, id)?;
    Some(
        api::PatchTaskBody::new()
            .done(!task.done)
            .updated_at(task.updated_at),
    )
}

/// The body that moves `id` to the next priority, against the version held now.
fn next_priority_body(snapshot: &Snapshot, id: &str) -> Option<api::PatchTaskBody> {
    let task = held(snapshot, id)?;
    let next = match task.priority {
        todo_core::Priority::Low => todo_core::Priority::Medium,
        todo_core::Priority::Medium => todo_core::Priority::High,
        todo_core::Priority::High => todo_core::Priority::Low,
        todo_core::Priority::Critical => todo_core::Priority::High,
    };
    Some(
        api::PatchTaskBody::new()
            .priority(next)
            .updated_at(task.updated_at),
    )
}

/// What the page tells the user when an edit did not apply.
///
/// The wording follows why the edit failed, not whether the reload afterwards worked: a
/// refusal from the server is not "could not reach the server" just because the reload
/// that followed also failed.
fn refused_note(error: &api::ApiError, reloaded: bool) -> &'static str {
    match (error, reloaded) {
        (api::ApiError::Network(_), _) => {
            "Could not reach the server, so that change was not saved."
        }
        (
            api::ApiError::Server(todo_core::ApiError {
                code: todo_core::ErrorCode::StaleUpdate,
                ..
            }),
            true,
        ) => "That task changed elsewhere, so it was reloaded. Try again.",
        (
            api::ApiError::Server(todo_core::ApiError {
                code: todo_core::ErrorCode::StaleUpdate,
                ..
            }),
            false,
        ) => "That task changed elsewhere. Reload the page to see it.",
        (_, true) => "That change was not saved. The list was reloaded.",
        (_, false) => "That change was not saved.",
    }
}

/// Where an edit's result goes: the list it replaces, the note that explains a refusal,
/// and the view settings that say how many days the list covers.
#[derive(Clone, Copy)]
struct EditSink {
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
    view_settings: ReadSignal<settings::Settings>,
}

/// Sends an edit and shows what the server now holds. Returns whether the edit applied.
///
/// A refused edit (the task changed on another device, or the network failed) is not an
/// error to hide: the list is reloaded so the next click starts from the server's copy,
/// and a short note says why nothing happened.
async fn patch_and_show(id: &todo_core::TaskId, body: api::PatchTaskBody, sink: EditSink) -> bool {
    match api::patch_task(id, body).await {
        Ok(snapshot) => {
            // A note about an earlier refusal must not outlive the retry that worked.
            sink.set_error.set(None);
            sink.set_snapshot.set(snapshot);
            true
        }
        Err(error) => {
            let days = sink.view_settings.get_untracked().days;
            let reloaded = match api::fetch_snapshot(&api::today_date(), days).await {
                Ok(snapshot) => {
                    sink.set_snapshot.set(snapshot);
                    true
                }
                Err(_) => false,
            };
            sink.set_error
                .set(Some(refused_note(&error, reloaded).to_owned()));
            false
        }
    }
}

#[component]
pub fn App() -> impl IntoView {
    let (snapshot, set_snapshot) = signal(api::fixture_snapshot());
    let (view_settings, set_view_settings) = signal(settings::load());
    let (error, set_error) = signal::<Option<String>>(None);
    let (brain_speech, set_brain_speech) = signal::<Option<String>>(None);
    let (brain_pose, set_brain_pose) = signal("home".to_owned());
    let (brain_event, set_brain_event) = signal::<Option<rambl_brain::Event>>(None);

    // Load snapshot from API on mount
    Effect::new(move |_| {
        wasm_bindgen_futures::spawn_local(async move {
            let days = view_settings.get_untracked().days;
            let new_snapshot = load_snapshot_from_api(days).await;
            set_snapshot.set(new_snapshot);
        });
    });

    account::start();
    let main_ref = NodeRef::<leptos::html::Main>::new();
    let (pane, set_pane) = signal(0_u8);
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
            <account::HeaderLink />
            <CaptureBox
                set_snapshot=set_snapshot
                set_error=set_error
                state=mascot_state
                set_state=set_mascot_state
                set_talking
            />
        </header>
        {move || error.get().map(|e| view! {
            <div class="error" aria-live="polite">{e}</div>
        })}
        <settings::SettingsPanel settings=view_settings set_settings=set_view_settings />
        <brain::Controller snapshot mode=Signal::derive(move || view_settings.get().mode) event=brain_event set_event=set_brain_event set_speech=set_brain_speech set_pose=set_brain_pose />
        <nav class="pane-switch" aria-label="Show">
            <button
                type="button"
                class:active=move || pane.get() == 0
                attr:aria-current=move || if pane.get() == 0 { "true" } else { "false" }
                aria-pressed=move || (pane.get() == 0).to_string()
                on:click=move |_| show_pane(main_ref, 0, set_pane)
            >"To do"</button>
            <button
                type="button"
                class:active=move || pane.get() == 1
                attr:aria-current=move || if pane.get() == 1 { "true" } else { "false" }
                aria-pressed=move || (pane.get() == 1).to_string()
                on:click=move |_| show_pane(main_ref, 1, set_pane)
            >"Planner"</button>
        </nav>
        <main node_ref=main_ref on:scroll=move |_| track_pane(main_ref, set_pane)>
            <div class="tasks-pane">
                <ImportIcs set_snapshot set_error />
                <TaskList snapshot set_snapshot set_error view_settings set_mascot_state />
            </div>
            <week::WeekGrid snapshot=snapshot settings=view_settings set_mascot_state=set_mascot_state />
        </main>
        <Mascot
            state=mascot_state
            set_state=set_mascot_state
            talking
            set_talking
            snapshot
            set_snapshot
            set_error
            view_settings
            brain_speech
            brain_pose
            set_brain_event
        />
    }
}

/// The pane in view on a phone: 0 for tasks, 1 for the week. The two panes sit side by
/// side in a scroll-snap row, so the pane is the scroll offset in whole pane widths.
#[cfg(any(target_arch = "wasm32", test))]
fn pane_at(scroll_left: f64, pane_width: f64) -> u8 {
    if pane_width <= 0.0 {
        return 0;
    }
    (scroll_left / pane_width).round().clamp(0.0, 1.0) as u8
}

#[cfg(target_arch = "wasm32")]
fn show_pane(main: NodeRef<leptos::html::Main>, index: u8, set_pane: WriteSignal<u8>) {
    if let Some(element) = main.get() {
        // Scroll to the pane's own offset, not a multiple of the width: the grid gap
        // sits between panes, so index * client_width lands short of the planner.
        if let Some(pane) = element.children().item(u32::from(index)) {
            let left = pane.unchecked_into::<web_sys::HtmlElement>().offset_left();
            element.set_scroll_left(left);
        }
    }
    set_pane.set(index);
}

#[cfg(not(target_arch = "wasm32"))]
fn show_pane(_main: NodeRef<leptos::html::Main>, index: u8, set_pane: WriteSignal<u8>) {
    set_pane.set(index);
}

#[cfg(target_arch = "wasm32")]
fn track_pane(main: NodeRef<leptos::html::Main>, set_pane: WriteSignal<u8>) {
    if let Some(element) = main.get() {
        let index = pane_at(
            f64::from(element.scroll_left()),
            f64::from(element.client_width()),
        );
        set_pane.set(index);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn track_pane(_main: NodeRef<leptos::html::Main>, _set_pane: WriteSignal<u8>) {}

#[component]
fn CaptureBox(
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
    state: ReadSignal<mascot::State>,
    set_state: WriteSignal<mascot::State>,
    set_talking: WriteSignal<bool>,
) -> impl IntoView {
    let (text, set_text) = signal(String::new());
    let (listening, set_listening) = signal(false);
    let speech_session = std::rc::Rc::new(std::cell::RefCell::new(None::<speech::Session>));
    let speech_session_for_mic = speech_session.clone();

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
            {if speech::supported() {
                    let speech_session_for_mic = speech_session_for_mic.clone();
                    view! {
                        <button
                            type="button"
                            aria-label="Speak to capture"
                            class=move || if listening.get() { "mic listening" } else { "mic" }
                            on:click=move |_| {
                                if listening.get_untracked() {
                                    if let Some(session) = speech_session_for_mic.borrow().as_ref() {
                                        session.stop();
                                    }
                                    return;
                                }
                                set_error.set(None);
                                let text_for_speech = std::rc::Rc::new(std::cell::RefCell::new(
                                    text.get_untracked(),
                                ));
                                let result = speech::Session::start(
                                    text_for_speech,
                                    move |value| set_text.set(value),
                                    move |value| {
                                        set_listening.set(value);
                                        set_talking.set(value);
                                    },
                                    move |value| set_error.set(value),
                                );
                                match result {
                                    Ok(session) => {
                                        *speech_session_for_mic.borrow_mut() = Some(session);
                                        set_listening.set(true);
                                        set_talking.set(true);
                                    }
                                    Err(message) => set_error.set(Some(message)),
                                }
                            }
                        >"Mic"</button>
                    }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }}
            <button
                class="mascot-home"
                type="button"
                aria-label="Talk with Rambo"
                on:pointerdown=move |_| {
                    set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Talk));
                    set_talking.set(true);
                }
            >
                <img src="assets/pen.svg" alt="Rambo" />
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
    set_error: WriteSignal<Option<String>>,
    view_settings: ReadSignal<settings::Settings>,
    brain_speech: ReadSignal<Option<String>>,
    brain_pose: ReadSignal<String>,
    set_brain_event: WriteSignal<Option<rambl_brain::Event>>,
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
            class=move || format!("mascot {} {}", animation.get(), brain_pose.get())
            draggable="true"
            on:dragstart=move |_| {
                set_brain_event.set(Some(rambl_brain::Event::Dragged));
                set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Grab))
            }
            on:dragend=move |_| {
                if matches!(state.get_untracked(), mascot::State::Dragging) {
                    start_return();
                }
            }
            on:pointerdown=move |_| {
                set_brain_event.set(Some(rambl_brain::Event::Poked));
                set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Talk));
                set_talking.set(true);
            }
        >
            <img src="assets/pen.svg" alt="Rambo" />
        </span>
        <span class="mascot-status" aria-live="polite">
            {move || brain_speech.get().map(|line| view! { <span class="brain-line">{line}</span> })}
            {move || match state.get_untracked() {
                mascot::State::Asking { ref title, .. } => view! {
                    <span class="ask">
                        <span>{format!("Hey! What do you want with \"{title}\"?")}</span>
                        <button type="button" on:click=move |_| {
                            if let mascot::State::Asking { ref task_id, .. } = state.get_untracked() {
                                let task_id_str = task_id.clone();
                                let body = snapshot.with_untracked(|s| next_priority_body(s, &task_id_str));
                                let sink = EditSink { set_snapshot, set_error, view_settings };
                                let set_state_clone = set_state;
                                wasm_bindgen_futures::spawn_local(async move {
                                    if let Some(body) = body {
                                        patch_and_show(&todo_core::TaskId(task_id_str), body, sink).await;
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
                                let body = snapshot.with_untracked(|s| toggle_done_body(s, &task_id));
                                let sink = EditSink { set_snapshot, set_error, view_settings };
                                let set_state_clone = set_state;
                                wasm_bindgen_futures::spawn_local(async move {
                                    if let Some(body) = body {
                                        patch_and_show(&todo_core::TaskId(task_id), body, sink).await;
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

fn label_name(label: Label) -> &'static str {
    match label {
        Label::School => "School",
        Label::Work => "Work",
        Label::Errands => "Errands",
        Label::Health => "Health",
        Label::Personal => "Personal",
        Label::Other => "Other",
    }
}

fn load_label_filter() -> Option<Label> {
    #[cfg(target_arch = "wasm32")]
    {
        let value = leptos::prelude::window()
            .local_storage()
            .ok()
            .flatten()
            .and_then(|storage| storage.get_item("rambl-task-label-filter").ok().flatten());
        return value.and_then(|value| match value.as_str() {
            "School" => Some(Label::School),
            "Work" => Some(Label::Work),
            "Errands" => Some(Label::Errands),
            "Health" => Some(Label::Health),
            "Personal" => Some(Label::Personal),
            "Other" => Some(Label::Other),
            _ => None,
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

fn save_label_filter(label: Option<Label>) {
    #[cfg(target_arch = "wasm32")]
    if let Ok(Some(storage)) = leptos::prelude::window().local_storage() {
        let value = label.map(label_name);
        let _ = match value {
            Some(value) => storage.set_item("rambl-task-label-filter", value),
            None => storage.remove_item("rambl-task-label-filter"),
        };
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = label;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditWhen {
    Anytime,
    DueBy,
    At,
}

fn local_input_value(time: DateTime<Utc>) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let date = js_sys::Date::new(&(time.timestamp_millis() as f64).into());
        return format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}",
            date.get_full_year(),
            date.get_month() + 1,
            date.get_date(),
            date.get_hours(),
            date.get_minutes()
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        time.format("%Y-%m-%dT%H:%M").to_string()
    }
}

fn parse_local_input(value: &str) -> Option<DateTime<Utc>> {
    #[cfg(target_arch = "wasm32")]
    {
        let millis = js_sys::Date::parse(&format!("{value}:00"));
        return DateTime::from_timestamp_millis(millis as i64);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
            .ok()
            .map(|time| DateTime::<Utc>::from_naive_utc_and_offset(time, Utc))
    }
}

fn is_overdue_at(task: &todo_core::Task, now: DateTime<Utc>) -> bool {
    !task.done && matches!(task.when, When::DueBy(deadline) if deadline < now)
}

fn is_overdue(task: &todo_core::Task) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        return DateTime::from_timestamp_millis(js_sys::Date::now() as i64)
            .is_some_and(|now| is_overdue_at(task, now));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = is_overdue_at(task, task.created);
        false
    }
}

#[component]
fn TaskList(
    snapshot: ReadSignal<Snapshot>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
    view_settings: ReadSignal<settings::Settings>,
    set_mascot_state: WriteSignal<mascot::State>,
) -> impl IntoView {
    let (dragged_task, set_dragged_task) = signal::<Option<TaskId>>(None);
    let (selected_label, set_selected_label) = signal(load_label_filter());
    let (editing_task, set_editing_task) = signal::<Option<TaskId>>(None);
    let (edit_title, set_edit_title) = signal(String::new());
    let (edit_duration, set_edit_duration) = signal(String::new());
    let (edit_when, set_edit_when) = signal(EditWhen::Anytime);
    let (edit_datetime, set_edit_datetime) = signal(String::new());
    let labels = move || {
        let tasks = snapshot.get().tasks;
        [
            Label::School,
            Label::Work,
            Label::Errands,
            Label::Health,
            Label::Personal,
            Label::Other,
        ]
        .into_iter()
        .filter(|label| tasks.iter().any(|task| task.label == *label))
        .collect::<Vec<_>>()
    };
    let visible_tasks = move || {
        snapshot
            .get()
            .tasks
            .into_iter()
            .filter(|task| selected_label.get().is_none_or(|label| task.label == label))
            .collect::<Vec<_>>()
    };

    view! {
        <section aria-labelledby="tasks-heading">
            <h2 id="tasks-heading">"To do"</h2>
            <div class="label-filters" aria-label="Filter tasks by label">
                <button
                    type="button"
                    class:active=move || selected_label.get().is_none()
                    on:click=move |_| {
                        set_selected_label.set(None);
                        save_label_filter(None);
                    }
                >"All"</button>
                <For each=labels key=|label| label_name(*label) let:label>
                    <button
                        type="button"
                        class:active=move || selected_label.get() == Some(label)
                        on:click=move |_| {
                            set_selected_label.set(Some(label));
                            save_label_filter(Some(label));
                        }
                    >{label_name(label)}</button>
                </For>
            </div>
            <ul>
                <For each=visible_tasks key=|task| task.id.clone() let:task>
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
                                let sink = EditSink { set_snapshot, set_error, view_settings };
                                let Some(body) = snapshot.with_untracked(|s| toggle_done_body(s, &task_id_clone.0)) else {
                                    return;
                                };
                                wasm_bindgen_futures::spawn_local(async move {
                                    patch_and_show(&task_id_clone, body, sink).await;
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
                        <span
                            class="task-open"
                            on:click={
                                let task = task.clone();
                                move |_| {
                                    set_edit_title.set(task.title.clone());
                                    set_edit_duration.set(task.duration_min.to_string());
                                    match task.when {
                                        When::Anytime => {
                                            set_edit_when.set(EditWhen::Anytime);
                                            set_edit_datetime.set(String::new());
                                        }
                                        When::DueBy(time) => {
                                            set_edit_when.set(EditWhen::DueBy);
                                            set_edit_datetime.set(local_input_value(time));
                                        }
                                        When::At(time) => {
                                            set_edit_when.set(EditWhen::At);
                                            set_edit_datetime.set(local_input_value(time));
                                        }
                                    }
                                    set_editing_task.set(Some(task.id.clone()));
                                }
                            }
                        >
                            <strong>{render_task_title.clone()}</strong> " — " {task.duration_min} " min"
                            {is_overdue(&task).then(|| view! { <span class="overdue">"Overdue"</span> })}
                        </span>
                        {move || {
                            let time = task_time(&snapshot.get(), &render_task_id);
                            (!time.is_empty()).then(|| view! { <span class="task-time">" · " {time}</span> })
                        }}
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
        <dialog
            class="task-dialog"
            open=move || editing_task.get().is_some()
            on:cancel=move |_: leptos::ev::Event| set_editing_task.set(None)
        >
            <form method="dialog">
                <h3>"Edit task"</h3>
                {move || editing_task.get().and_then(|id| {
                    snapshot.get().tasks.into_iter().find(|task| task.id == id).map(|task| {
                        let imported = task.source == Source::Calendar;
                        let task_id = task.id.clone();
                        view! {
                            <p class="dialog-task-title">{task.title.clone()}</p>
                            {imported.then(|| view! {
                                <p class="calendar-notice">
                                    "This came from your calendar. Changes here won't sync back."
                                </p>
                            })}
                            <label>
                                "Title "
                                <input
                                    prop:value=edit_title
                                    on:input=move |event| set_edit_title.set(event_target_value(&event))
                                />
                            </label>
                            <fieldset>
                                <legend>"Length (minutes)"</legend>
                                <div class="duration-chips">
                                    {[15_u16, 30, 60, 120].into_iter().map(|minutes| view! {
                                        <button
                                            type="button"
                                            class:active=move || edit_duration.get() == minutes.to_string()
                                            on:click=move |_| set_edit_duration.set(minutes.to_string())
                                        >{minutes}</button>
                                    }).collect_view()}
                                    <input
                                        type="number"
                                        min="1"
                                        prop:value=edit_duration
                                        on:input=move |event| set_edit_duration.set(event_target_value(&event))
                                        aria-label="Custom length in minutes"
                                    />
                                </div>
                            </fieldset>
                            <fieldset>
                                <legend>"When"</legend>
                                <label><input type="radio" name="when" checked=move || edit_when.get() == EditWhen::Anytime on:change=move |_| set_edit_when.set(EditWhen::Anytime) /> "Anytime"</label>
                                <label><input type="radio" name="when" checked=move || edit_when.get() == EditWhen::DueBy on:change=move |_| set_edit_when.set(EditWhen::DueBy) /> "Due by"</label>
                                <label><input type="radio" name="when" checked=move || edit_when.get() == EditWhen::At on:change=move |_| set_edit_when.set(EditWhen::At) /> "Scheduled at"</label>
                                <input
                                    type="datetime-local"
                                    prop:value=edit_datetime
                                    on:input=move |event| set_edit_datetime.set(event_target_value(&event))
                                />
                            </fieldset>
                            <div class="dialog-actions">
                                <button
                                    type="button"
                                    on:click={
                                        let task_id = task_id.clone();
                                        move |_| {
                                            let task_id = task_id.clone();
                                            let Some(body) = snapshot.with_untracked(|s| toggle_done_body(s, &task_id.0)) else {
                                                return;
                                            };
                                            let sink = EditSink { set_snapshot, set_error, view_settings };
                                            wasm_bindgen_futures::spawn_local(async move {
                                                if patch_and_show(&task_id, body, sink).await {
                                                    set_editing_task.set(None);
                                                }
                                            });
                                        }
                                    }
                                >{if task.done { "Undone" } else { "Done" }}</button>
                                <button
                                    type="button"
                                    on:click={
                                        let task_id = task_id.clone();
                                        move |_| {
                                            let task_id = task_id.clone();
                                            wasm_bindgen_futures::spawn_local(async move {
                                                if let Ok(new_snap) = api::delete_task(&task_id).await {
                                                    set_snapshot.set(new_snap);
                                                    set_editing_task.set(None);
                                                }
                                            });
                                        }
                                    }
                                >"Delete"</button>
                                <button type="button" on:click=move |_| set_editing_task.set(None)>"Cancel"</button>
                                <button
                                    type="button"
                                    on:click={
                                        let task_id = task_id.clone();
                                        move |_| {
                                            let title = edit_title.get_untracked().trim().to_string();
                                            let duration = edit_duration.get_untracked().parse::<u16>().ok();
                                            let when = match edit_when.get_untracked() {
                                                EditWhen::Anytime => Some(When::Anytime),
                                                EditWhen::DueBy => parse_local_input(&edit_datetime.get_untracked()).map(When::DueBy),
                                                EditWhen::At => parse_local_input(&edit_datetime.get_untracked()).map(When::At),
                                            };
                                            if title.is_empty() || duration.is_none() || when.is_none() {
                                                return;
                                            }
                                            let task_id = task_id.clone();
                                            let Some(seen) = snapshot.with_untracked(|s| held(s, &task_id.0).map(|task| task.updated_at)) else {
                                                return;
                                            };
                                            let sink = EditSink { set_snapshot, set_error, view_settings };
                                            let body = api::PatchTaskBody::new()
                                                .title(title)
                                                .duration_min(duration.unwrap_or(1))
                                                .when(when.unwrap_or(When::Anytime))
                                                .updated_at(seen);
                                            wasm_bindgen_futures::spawn_local(async move {
                                                if patch_and_show(&task_id, body, sink).await {
                                                    set_editing_task.set(None);
                                                }
                                            });
                                        }
                                    }
                                >"Save"</button>
                            </div>
                        }
                    })
                })}
            </form>
        </dialog>
    }
}

fn task_time(snapshot: &Snapshot, id: &str) -> String {
    snapshot
        .plan
        .placements
        .iter()
        .find(|p| p.task.0 == id)
        .map(|p| week::local_time_label(&week::planner_context(&snapshot.tz), p.start))
        .unwrap_or_default()
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overdue_only_applies_to_incomplete_due_tasks() {
        let mut task = api::fixture_snapshot().tasks[0].clone();
        let now = task.created + chrono::Duration::hours(1);
        task.when = When::DueBy(now - chrono::Duration::minutes(1));
        assert!(is_overdue_at(&task, now));
        task.done = true;
        assert!(!is_overdue_at(&task, now));
        task.done = false;
        task.when = When::At(now - chrono::Duration::minutes(1));
        assert!(!is_overdue_at(&task, now));
    }

    /// A row keeps the task it was first drawn with, so the second click on the same
    /// task must still carry the server's newest version and the current done state.
    #[test]
    fn an_edit_is_built_from_the_task_as_the_list_holds_it_now() {
        let mut snapshot = api::fixture_snapshot();
        let id = snapshot.tasks[0].id.0.clone();
        let first = toggle_done_body(&snapshot, &id).expect("task is listed");
        assert_eq!(first.done, Some(true));
        assert_eq!(first.updated_at, Some(snapshot.tasks[0].updated_at));

        // The server accepted that edit: the list now holds a newer version, done.
        snapshot.tasks[0].done = true;
        snapshot.tasks[0].updated_at += chrono::Duration::minutes(5);
        let second = toggle_done_body(&snapshot, &id).expect("task is listed");
        assert_eq!(second.done, Some(false), "must flip the current state");
        assert_eq!(second.updated_at, Some(snapshot.tasks[0].updated_at));

        let bump = next_priority_body(&snapshot, &id).expect("task is listed");
        assert_eq!(bump.updated_at, Some(snapshot.tasks[0].updated_at));
    }

    #[test]
    fn a_refused_edit_says_why() {
        let stale = api::ApiError::Server(todo_core::ApiError {
            code: todo_core::ErrorCode::StaleUpdate,
            message: String::new(),
        });
        assert!(refused_note(&stale, true).contains("changed elsewhere"));
        assert!(refused_note(&stale, false).contains("Reload the page"));
        let offline = api::ApiError::Network("offline".into());
        for reloaded in [true, false] {
            assert!(refused_note(&offline, reloaded).contains("Could not reach the server"));
        }
        // A server refusal is never reported as an unreachable server, reload or not.
        let refused = api::ApiError::Server(todo_core::ApiError {
            code: todo_core::ErrorCode::BadRequest,
            message: String::new(),
        });
        for reloaded in [true, false] {
            assert!(!refused_note(&refused, reloaded).contains("reach"));
            assert!(refused_note(&refused, reloaded).contains("not saved"));
        }
    }

    #[test]
    fn no_edit_is_built_for_a_task_that_is_gone() {
        let snapshot = api::fixture_snapshot();
        assert!(toggle_done_body(&snapshot, "nope").is_none());
        assert!(next_priority_body(&snapshot, "nope").is_none());
    }

    #[test]
    fn parses_local_datetime_input() {
        assert_eq!(
            parse_local_input("2026-10-12T14:30")
                .expect("valid datetime")
                .format("%Y-%m-%dT%H:%M")
                .to_string(),
            "2026-10-12T14:30"
        );
    }
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

#[cfg(test)]
mod pane_tests {
    use super::pane_at;

    #[test]
    fn a_phone_swipe_maps_to_the_nearest_pane() {
        assert_eq!(pane_at(0.0, 400.0), 0);
        assert_eq!(pane_at(180.0, 400.0), 0);
        assert_eq!(pane_at(220.0, 400.0), 1);
        assert_eq!(pane_at(400.0, 400.0), 1);
        assert_eq!(pane_at(900.0, 400.0), 1, "never past the last pane");
        assert_eq!(pane_at(100.0, 0.0), 0, "no width yet, so the first pane");
    }
}

//! The task list: its rows, the edit dialog, and deleting with a scribble.

mod fields;
#[cfg(target_arch = "wasm32")]
mod scribble;
#[cfg(target_arch = "wasm32")]
pub(crate) mod write;

use chrono::{DateTime, NaiveDate, Utc, Weekday};
use leptos::prelude::*;
use todo_core::{Label, Snapshot, Source, TaskId, When};

use crate::clock::Clock;
use crate::edits::{EditSink, confirmed, held, patch_and_show, series_note, toggle_done_body};
#[cfg(target_arch = "wasm32")]
use crate::pen;
use crate::{api, clock, mascot, row, settings, week};
use fields::{
    EditWhen, WEEKDAY_CHOICES, due_display, local_input_value, parse_hhmm, series_from_dialog,
    series_patch_from_dialog, toggled_day, typed_when,
};

/// The order the server wants after moving `dragged` to `slot` beside `target`,
/// or `None` when the drop is not a reorder at all.
///
/// The server renumbers stored tasks only (`ids must list every task exactly once`),
/// and a snapshot lists series occurrences too — rows with `{series}:{date}` ids
/// that are expanded on the fly and share the series' row. A slip dragged from or
/// onto one is not a reorder, so the handler skips the call instead of failing 400.
fn reorder_ids(
    snapshot: &Snapshot,
    dragged: &TaskId,
    target: &TaskId,
    slot: row::Slot,
) -> Option<Vec<TaskId>> {
    if dragged == target {
        return None;
    }
    if snapshot
        .tasks
        .iter()
        .any(|task| task.source == Source::Series && (task.id == *dragged || task.id == *target))
    {
        return None;
    }
    let mut new_order: Vec<TaskId> = snapshot
        .tasks
        .iter()
        .filter(|task| task.source != Source::Series && task.id != *dragged)
        .map(|task| task.id.clone())
        .collect();
    // A drop that missed every row lands the slip at the end rather than
    // nowhere, which is where a finger that let go over empty paper expects it.
    let at = match (new_order.iter().position(|id| id == target), slot) {
        (Some(at), row::Slot::Before) => at,
        (Some(at), row::Slot::After) => at + 1,
        (None, _) => new_order.len(),
    };
    new_order.insert(at, dragged.clone());
    Some(new_order)
}

/// What the task list keys a row on: the whole task as the row draws it.
///
/// `<For>` keeps a row for as long as its key holds, so a key of the id alone kept the
/// row's first-drawn task: an edited title still showed the old one, and the dialog it
/// opened saved those old fields back over the edit. Any change to the task now draws a
/// fresh row.
#[derive(Clone)]
struct RowKey(todo_core::Task);

impl PartialEq for RowKey {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

// A task holds no floats, so its equality is total.
impl Eq for RowKey {}

impl std::hash::Hash for RowKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Equal tasks share an id and a version, so this agrees with `eq`.
        self.0.id.hash(state);
        self.0.updated_at.hash(state);
    }
}

/// The two halves of an occurrence row's id, which the server mints as
/// `"{series_id}:{date}"` (docs/ARCHITECTURE.md 7.1). Ordinary task ids carry
/// no date, so they give `None`; a moved occurrence keeps its original date in
/// the id, which is exactly the day an exception names.
fn occurrence_parts(id: &str) -> Option<(&str, NaiveDate)> {
    let (series_id, date) = id.split_once(':')?;
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some((series_id, date))
}

/// The series a row belongs to, when the row is an occurrence of one.
/// What the edit sheet shows when it opens on the row `id`, or `None` when the
/// list no longer holds it.
#[derive(Clone, Debug, PartialEq)]
struct EditForm {
    title: String,
    duration: String,
    when: EditWhen,
    datetime: String,
    weekdays: Vec<Weekday>,
    until: String,
    time: String,
}

fn edit_form(snapshot: &Snapshot, id: &str) -> Option<EditForm> {
    let task = held(snapshot, id)?;
    let (when, datetime) = match task.when {
        When::Anytime => (EditWhen::Anytime, String::new()),
        When::DueBy(time) => (EditWhen::DueBy, due_display(task, time)),
        When::At(time) => (EditWhen::At, local_input_value(time)),
    };
    // A series row edits the series: its fields come from the rule, not from the
    // one occurrence row opened.
    let series = series_of(snapshot, id);
    Some(EditForm {
        title: task.title.clone(),
        duration: task.duration_min.to_string(),
        when,
        datetime,
        weekdays: series.map(|s| s.rule.weekdays.clone()).unwrap_or_default(),
        until: series
            .and_then(|s| s.rule.until)
            .map(|day| day.to_string())
            .unwrap_or_default(),
        time: series
            .and_then(|s| s.time_of_day)
            .map(|time| time.format("%H:%M").to_string())
            .unwrap_or_default(),
    })
}

fn series_of<'a>(snapshot: &'a Snapshot, id: &str) -> Option<&'a todo_core::Series> {
    let (series_id, _) = occurrence_parts(id)?;
    snapshot
        .series
        .iter()
        .find(|series| series.id.0 == series_id)
}

/// What ticking a row writes. An ordinary task flips its `done` flag against
/// the version the list holds; an occurrence writes — or, un-ticked, clears —
/// an exception on its own date, because its flag is per date, never per task.
#[derive(Debug)]
pub(crate) enum TickAction {
    Task(api::PatchTaskBody),
    MarkDone { series: String, date: NaiveDate },
    ClearDone { series: String, date: NaiveDate },
}

pub(crate) fn tick_action(snapshot: &Snapshot, id: &str) -> Option<TickAction> {
    if let Some((series_id, date)) = occurrence_parts(id) {
        let done = held(snapshot, id)?.done;
        let series = series_id.to_owned();
        return Some(if done {
            TickAction::ClearDone { series, date }
        } else {
            TickAction::MarkDone { series, date }
        });
    }
    toggle_done_body(snapshot, id).map(TickAction::Task)
}

/// Writes a tick. A task edit is refused with a reload-and-retry note
/// (`patch_and_show`); a series write has nothing to reload, so its refusals speak
/// for themselves.
pub(crate) async fn apply_tick(id: &TaskId, action: TickAction, sink: EditSink) {
    let written = match action {
        TickAction::Task(body) => {
            patch_and_show(id, body, sink).await;
            return;
        }
        TickAction::MarkDone { series, date } => {
            api::mark_occurrence(&series, &date.to_string(), "done").await
        }
        TickAction::ClearDone { series, date } => {
            api::clear_occurrence(&series, &date.to_string()).await
        }
    };
    match written {
        Ok(snapshot) => {
            sink.set_error.set(None);
            sink.set_snapshot.set(snapshot);
        }
        Err(e) => sink.set_error.set(Some(series_note(&e))),
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

/// Hook exposed for Rambo's scribble-out animation before removal. Named
/// after the task, because the row carries its own title.
#[cfg(target_arch = "wasm32")]
pub async fn on_delete_requested(task_id: todo_core::TaskId) {
    scribble::scribble_out(&task_id.0).await;
}

/// Off the browser there is nothing to scribble on, so the row just goes.
#[cfg(not(target_arch = "wasm32"))]
pub async fn on_delete_requested(_task_id: todo_core::TaskId) {}

/// Delete a task the user asked to delete, with Rambo crossing its title out
/// first. Every user-initiated delete goes through here, so the scribble is
/// the rule rather than a promise in a comment: the `api::delete_task` calls
/// that bypass it are the two that must not scribble, and each says why.
///
/// An occurrence of a repeat is not a stored task: deleting its row skips that one
/// date, the way the series endpoint describes (docs/ARCHITECTURE.md 7.1).
pub async fn delete_with_scribble(
    task_id: &todo_core::TaskId,
) -> Result<todo_core::Snapshot, api::ApiError> {
    on_delete_requested(task_id.clone()).await;
    match occurrence_parts(&task_id.0) {
        Some((series, date)) => api::mark_occurrence(series, &date.to_string(), "skipped").await,
        None => api::delete_task(task_id).await,
    }
}

#[component]
pub(crate) fn TaskList(
    snapshot: ReadSignal<Snapshot>,
    set_snapshot: WriteSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
    view_settings: ReadSignal<settings::Settings>,
    mascot_state: ReadSignal<mascot::State>,
    set_mascot_state: WriteSignal<mascot::State>,
    pen_queue: RwSignal<Vec<TaskId>>,
    pen_pos: RwSignal<(f64, f64)>,
    /// A row to open the edit sheet on, set by Rambo's "Edit".
    edit_request: RwSignal<Option<TaskId>>,
) -> impl IntoView {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = pen_pos;
    let (selected_label, set_selected_label) = signal(load_label_filter());
    let (editing_task, set_editing_task) = signal::<Option<TaskId>>(None);
    let (edit_title, set_edit_title) = signal(String::new());
    let (edit_duration, set_edit_duration) = signal(String::new());
    let (edit_when, set_edit_when) = signal(EditWhen::Anytime);
    let (edit_datetime, set_edit_datetime) = signal(String::new());
    let (edit_time_error, set_edit_time_error) = signal::<Option<&'static str>>(None);
    // The repeats block (docs/ARCHITECTURE.md 7.1): which weekdays a repeat
    // runs on, and the optional end date and time for a series.
    let edit_weekdays = RwSignal::new(Vec::<Weekday>::new());
    let edit_until = RwSignal::new(String::new());
    let edit_time = RwSignal::new(String::new());
    // Open the edit sheet on a row: from a click on it, or from Rambo's "Edit".
    let open_editor = move |id: &TaskId| {
        let Some(form) = snapshot.with_untracked(|s| edit_form(s, &id.0)) else {
            return;
        };
        set_edit_title.set(form.title);
        set_edit_duration.set(form.duration);
        set_edit_when.set(form.when);
        set_edit_datetime.set(form.datetime);
        edit_weekdays.set(form.weekdays);
        edit_until.set(form.until);
        edit_time.set(form.time);
        set_editing_task.set(Some(id.clone()));
        set_edit_time_error.set(None);
    };
    Effect::new(move |_| {
        if let Some(id) = edit_request.get() {
            edit_request.set(None);
            open_editor(&id);
        }
    });
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

    // Process pen queue: only write rows for tasks the client just created,
    // not every task on reload.
    #[cfg(target_arch = "wasm32")]
    {
        Effect::new(move |_| {
            let mode = view_settings.get().mode;
            if mascot::prefers_reduced_motion() || mode == settings::RamblMode::Lobotomized {
                return;
            }
            let queue = pen_queue.get();
            if queue.is_empty() {
                return;
            }
            let current_state = mascot_state.get();
            // Start writing if Rambo is at Home or stuck at WritingCaret (after Add).
            if matches!(
                current_state,
                mascot::State::Home | mascot::State::WritingCaret
            ) {
                let set_state = set_mascot_state;
                let pen_pos = pen_pos;
                wasm_bindgen_futures::spawn_local(async move {
                    for task_id in queue {
                        // On the first row, Rambo may be at WritingCaret (after Add) or Home.
                        // On subsequent rows, he should be back at Home from the fly-home above.
                        let state = mascot_state.get_untracked();
                        let is_ready =
                            matches!(state, mascot::State::Home | mascot::State::WritingCaret);
                        if !is_ready {
                            break;
                        }
                        let row = web_sys::window()
                            .and_then(|w| w.document())
                            .and_then(|doc| {
                                doc.query_selector(&format!("li[data-id=\"{}\"]", task_id.0))
                                    .ok()
                                    .flatten()
                            });
                        // A row off the screen (the other pane on a phone, or
                        // scrolled away) is left as it is; he stays home.
                        if !row.as_ref().is_some_and(write::in_view) {
                            set_state.set(mascot::transition(&state, mascot::Event::SkipRow));
                            continue;
                        }
                        set_state.set(mascot::transition(&state, mascot::Event::WriteRow));
                        // Clip the title and park the nib at the line's start.
                        // The fonts settle first: parking on fallback metrics
                        // would put the pen somewhere the line does not start.
                        if let Some(row) = row.as_ref() {
                            row.class_list().add_1("pen-pending").ok();
                        }
                        write::wait_for_fonts().await;
                        if let Some(row) = row.as_ref() {
                            write::park_nib(row, pen_pos);
                        }
                        gloo_timers::future::TimeoutFuture::new(pen::FLY_MS).await;
                        set_state.set(mascot::transition(
                            &mascot_state.get_untracked(),
                            mascot::Event::WriteRow,
                        ));
                        if let Some(row) = row.as_ref() {
                            write::write_row_title(row, pen_pos).await;
                        }
                        // Fly home after this row
                        set_state.set(mascot::transition(
                            &mascot::State::WritingRow,
                            mascot::Event::FlyHome,
                        ));
                        gloo_timers::future::TimeoutFuture::new(pen::FLY_MS).await;
                        set_state.set(mascot::transition(
                            &mascot_state.get_untracked(),
                            mascot::Event::FlyHome,
                        ));
                    }
                    pen_queue.set(Vec::new());
                });
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (pen_queue, mascot_state, set_mascot_state, view_settings);

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
            <ul class="task-list">
                <For each=visible_tasks key=|task| RowKey(task.clone()) let:task>
                    {
                        let task_id = task.id.clone();
                        let task_title = task.title.clone();
                        let task_done_label = match occurrence_parts(&task.id.0) {
                            Some((_, date)) => format!("Mark {task_title} on {date} as done"),
                            None => format!("Mark {task_title} as done"),
                        };
                        // An occurrence row carries the small hand-drawn repeat mark, and
                        // loses the one-row delete: its × would refuse the id, and a whole
                        // series is only ever deleted from the dialog, with a confirm.
                        let is_occurrence = task.source == Source::Series;
                        let repeat_mark = is_occurrence.then(|| view! {
                            <span class="repeat-mark" aria-hidden="true">
                                <svg viewBox="0 0 20 20" width="13" height="13">
                                    <path
                                        d="M10.1 3.3 C 5.6 2.8, 2.9 6.2, 3.5 10 C 4.1 14.1, 8.3 16.5, 12 15.3 C 15.7 14.1, 17.4 10.4, 15.9 7.2 C 14.7 4.6, 11.8 3.5, 9.4 4.5"
                                        fill="none" stroke="currentColor" stroke-width="1.7"
                                        stroke-linecap="round" stroke-linejoin="round" />
                                    <path
                                        d="M16.1 7.0 L 17.5 3.2 L 13.4 3.5"
                                        fill="none" stroke="currentColor" stroke-width="1.7"
                                        stroke-linecap="round" stroke-linejoin="round" />
                                </svg>
                            </span>
                        });
                        let drop_task_id = task_id.0.clone();
                        let render_task_id = task_id.0.clone();
                        let render_task_id_attr = render_task_id.clone();
                        let render_task_title = task_title.clone();

                        let toggle_done = {
                            let task_id_clone = task_id.clone();
                            move |_| {
                                let task_id_clone = task_id_clone.clone();
                                let sink = EditSink { set_snapshot, set_error };
                                let Some(action) =
                                    snapshot.with_untracked(|s| tick_action(s, &task_id_clone.0))
                                else {
                                    return;
                                };
                                wasm_bindgen_futures::spawn_local(async move {
                                    apply_tick(&task_id_clone, action, sink).await;
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
                                    // Rambo crosses the title out first; the task
                                    // only leaves once the ink has dried.
                                    if let Ok(new_snap) = delete_with_scribble(&task_id_clone).await {
                                        set_snapshot_clone.set(new_snap);
                                    }
                                });
                            }
                        };

                        // One finger's whole time on this row, shared by the four
                        // handlers below. `row::dom` decides what the press has
                        // become; this only tells it what to do about it.
                        let presses: row::dom::Presses =
                            std::rc::Rc::new(std::cell::RefCell::new(None));

                        let handle_pointer_down = {
                            let presses = presses.clone();
                            move |ev: leptos::ev::PointerEvent| {
                                row::dom::down(&presses, &ev);
                            }
                        };

                        let handle_pointer_move = {
                            let presses = presses.clone();
                            move |ev: leptos::ev::PointerEvent| {
                                // Only once the row is lifted: a press that is
                                // still a tap must not stop the click, and one
                                // that has become a scroll must not stop the
                                // scroll.
                                let lifted = matches!(
                                    row::dom::gesture(&presses),
                                    Some(row::Gesture::Lifted)
                                );
                                if lifted {
                                    ev.prevent_default();
                                }
                                row::dom::drag(&presses, &ev);
                            }
                        };

                        let handle_pointer_up = {
                            let presses = presses.clone();
                            let set_snapshot_clone = set_snapshot;
                            let drop_task_id_clone = drop_task_id.clone();
                            move |ev: leptos::ev::PointerEvent| {
                                let Some((outcome, row_id, x, y)) = row::dom::up(&presses, &ev)
                                else {
                                    return;
                                };
                                let dragged = TaskId(row_id);
                                match outcome {
                                    // A tap belongs to the controls, which answer
                                    // it themselves; a scroll and a backed-out
                                    // swipe leave the list exactly as it was.
                                    row::Release::Tap | row::Release::Scroll | row::Release::SnapBack => {}
                                    row::Release::Delete => {
                                        let task_id = TaskId(drop_task_id_clone.clone());
                                        let set_snapshot = set_snapshot_clone;
                                        wasm_bindgen_futures::spawn_local(async move {
                                            // A swipe is a row delete like the mark:
                                            // Rambo crosses the title out first.
                                            if let Ok(new_snap) = delete_with_scribble(&task_id).await {
                                                set_snapshot.set(new_snap);
                                            }
                                        });
                                    }
                                    row::Release::Reorder => {
                                        // A drop that lands on a series occurrence
                                        // is not a reorder at all, and the server
                                        // would refuse it: reorder_ids says no
                                        // instead, so nothing is sent.
                                        let new_order = row::dom::drop_target(x, y)
                                            .and_then(|(target, slot)| {
                                                reorder_ids(
                                                    &snapshot.get_untracked(),
                                                    &dragged,
                                                    &TaskId(target),
                                                    slot,
                                                )
                                            });
                                        let Some(new_order) = new_order else {
                                            return;
                                        };
                                        let set_snapshot = set_snapshot_clone;
                                        wasm_bindgen_futures::spawn_local(async move {
                                            if let Ok(new_snap) = api::reorder_tasks(new_order).await {
                                                set_snapshot.set(new_snap);
                                            }
                                        });
                                    }
                                }
                            }
                        };
                        let handle_pointer_cancel = {
                            let presses = presses.clone();
                            move |_| row::dom::cancel(&presses)
                        };
                        view! {
                    <li
                        class="task-row"
                        data-id={render_task_id_attr}
                        data-title={render_task_title.clone()}
                        on:pointerdown=handle_pointer_down
                        on:pointermove=handle_pointer_move
                        on:pointerup=handle_pointer_up
                        on:pointercancel=handle_pointer_cancel
                        // The row answers its own touch gestures and stops them here, so nothing
                        // above the row sees the same touch. This does NOT stop the page
                        // flip: that listens on pointer events, which are a separate
                        // dispatch. What keeps swipe right (delete) and swipe left (turn
                        // the page) apart is the direction rule in `row::advance`: right
                        // far enough is this row's to delete, anything left is a scroll
                        // and belongs to the panes.
                        on:touchstart=move |ev: leptos::ev::TouchEvent| ev.stop_propagation()
                        on:touchend=move |ev: leptos::ev::TouchEvent| ev.stop_propagation()
                        // Once the row is lifted the finger belongs to the row, not the
                        // page: without this the list scrolls out from under the drag.
                        on:touchmove=move |ev: leptos::ev::TouchEvent| {
                            if matches!(
                                row::dom::gesture(&presses),
                                Some(row::Gesture::Lifted)
                            ) {
                                ev.prevent_default();
                            }
                        }
                    >
                        <input
                            type="checkbox"
                            // The browser ticks the box before the server answers; a
                            // refused tick must untick it again from what the list holds.
                            prop:checked={
                                let id = task.id.0.clone();
                                move || snapshot.with(|s| held(s, &id).is_some_and(|task| task.done))
                            }
                            on:change=toggle_done
                            aria-label=task_done_label
                        />
                        <span
                            class="task-open"
                            on:click={
                                let id = task.id.clone();
                                move |_| open_editor(&id)
                            }
                        >
                            <span class="pen-title-wrap">
                                <strong class="pen-title">{render_task_title.clone()}</strong>
                                {repeat_mark}
                            </span> " — " {task.duration_min} " min"
                            {move || {
                                let time = task_time(&snapshot.get(), &render_task_id, view_settings.get().clock);
                                (!time.is_empty()).then(|| view! { <span class="task-time">" · " {time}</span> })
                            }}
                            {is_overdue(&task).then(|| view! { <span class="overdue">"Overdue"</span> })}
                        </span>
                        <span class="pen-underline"><img src="assets/pen-underline.svg" alt="" /></span>
                        {(!is_occurrence).then(|| view! {
                            <button
                                class="delete-mark"
                                type="button"
                                on:click=delete_task
                                aria-label=format!("Delete {}", task_title)
                            >"×"</button>
                        })}
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
                <h3>{move || match editing_task.get() {
                    Some(id) if occurrence_parts(&id.0).is_some() => "Edit series",
                    _ => "Edit task",
                }}</h3>
                {move || editing_task.get().and_then(|id| {
                    snapshot.get().tasks.into_iter().find(|task| task.id == id).map(|task| {
                        let imported = task.source == Source::Calendar;
                        let task_id = task.id.clone();
                        let task_done = task.done;
                        // An occurrence row edits the series it belongs to; `occ` also
                        // names the exact date, which is what Done / Not today write to.
                        let recurring = task.source == Source::Series;
                        let occ = occurrence_parts(&task_id.0)
                            .map(|(series_id, date)| (series_id.to_owned(), date));
                        // The zone a deadline in this dialog is shown and parsed in,
                        // rendered next to the date field so the user knows the wall
                        // clock means that zone (docs/ARCHITECTURE.md 4.1).
                        let due_zone = task.due_tz.clone();
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
                                <legend>{if recurring { "Repeats weekly" } else { "Make it repeat weekly" }}</legend>
                                <div class="day-picker">
                                    {WEEKDAY_CHOICES.iter().map(|(day, name)| view! {
                                        <button
                                            type="button"
                                            class:active=move || edit_weekdays.get().contains(day)
                                            on:click=move |_| {
                                                edit_weekdays
                                                    .set(toggled_day(edit_weekdays.get_untracked(), *day));
                                            }
                                        >{*name}</button>
                                    }).collect_view()}
                                </div>
                                <label>
                                    "Until (optional) "
                                    <input
                                        type="date"
                                        prop:value=edit_until
                                        on:input=move |event| edit_until.set(event_target_value(&event))
                                    />
                                </label>
                                <label>
                                    "At (optional) "
                                    <input
                                        type="time"
                                        prop:value=edit_time
                                        on:input=move |event| edit_time.set(event_target_value(&event))
                                    />
                                    // The native field's rendering follows the browser's
                                    // locale, not the paper's clock, so the chosen time is
                                    // also written out in it beside the field.
                                    <span class="dialog-time-written">
                                        {move || {
                                            let clock = view_settings.get().clock;
                                            parse_hhmm(&edit_time.get()).map(|value| {
                                                let (hour, minute) = value.split_once(':').unwrap_or(("0", "0"));
                                                clock::format_time(
                                                    hour.parse::<u16>().unwrap_or(0) * 60
                                                        + minute.parse::<u16>().unwrap_or(0),
                                                    clock,
                                                )
                                            })
                                        }}
                                    </span>
                                </label>
                                {(!recurring).then(|| view! {
                                    <p class="dialog-hint">
                                        "Pick days and save: this task becomes a weekly series, and its one-off row goes away."
                                    </p>
                                })}
                            </fieldset>
                            {(!recurring).then(|| view! {
                            <fieldset>
                                <legend>"When"</legend>
                                <label><input type="radio" name="when" checked=move || edit_when.get() == EditWhen::Anytime on:change=move |_| set_edit_when.set(EditWhen::Anytime) /> "Anytime"</label>
                                <label><input type="radio" name="when" checked=move || edit_when.get() == EditWhen::DueBy on:change=move |_| set_edit_when.set(EditWhen::DueBy) /> "Due by"</label>
                                <label><input type="radio" name="when" checked=move || edit_when.get() == EditWhen::At on:change=move |_| set_edit_when.set(EditWhen::At) /> "Scheduled at"</label>
                                <input
                                    type="datetime-local"
                                    prop:value=edit_datetime
                                    on:input=move |event| {
                                        set_edit_datetime.set(event_target_value(&event));
                                        set_edit_time_error.set(None);
                                    }
                                />
                                {move || match due_zone.as_deref() {
                                    Some(zone) if edit_when.get() == EditWhen::DueBy => Some(view! {
                                        <span class="due-zone">"(" {zone.to_owned()} ")"</span>
                                    }),
                                    _ => None,
                                }}
                                {move || edit_time_error.get().map(|message| view! {
                                    <p class="time-error" role="alert">{message}</p>
                                })}
                            </fieldset>
                            })}
                            <div class="dialog-actions">
                                {if recurring {
                                    view! {
                                        <button
                                            type="button"
                                            on:click={
                                                let occ = occ.clone();
                                                move |_| {
                                                    let Some((series, date)) = occ.clone() else {
                                                        return;
                                                    };
                                                    let set_snapshot = set_snapshot;
                                                    let set_error = set_error;
                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        // Done and Undone are the same button on
                                                        // this date: marked writes the exception,
                                                        // un-marked clears it.
                                                        let result = if task_done {
                                                            api::clear_occurrence(&series, &date.to_string()).await
                                                        } else {
                                                            api::mark_occurrence(&series, &date.to_string(), "done").await
                                                        };
                                                        match result {
                                                            Ok(snapshot) => {
                                                                set_snapshot.set(snapshot);
                                                                set_editing_task.set(None);
                                                            }
                                                            Err(e) => set_error.set(Some(series_note(&e))),
                                                        }
                                                    });
                                                }
                                            }
                                        >{if task_done { "Undone" } else { "Done" }}</button>
                                        <button
                                            type="button"
                                            on:click={
                                                let occ = occ.clone();
                                                move |_| {
                                                    let Some((series, date)) = occ.clone() else {
                                                        return;
                                                    };
                                                    let set_snapshot = set_snapshot;
                                                    let set_error = set_error;
                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        match api::mark_occurrence(&series, &date.to_string(), "skipped").await {
                                                            Ok(snapshot) => {
                                                                set_snapshot.set(snapshot);
                                                                set_editing_task.set(None);
                                                            }
                                                            Err(e) => set_error.set(Some(series_note(&e))),
                                                        }
                                                    });
                                                }
                                            }
                                        >"Not today"</button>
                                        <button
                                            type="button"
                                            on:click={
                                                let occ = occ.clone();
                                                move |_| {
                                                    let Some((series, _)) = occ.clone() else {
                                                        return;
                                                    };
                                                    if !confirmed("Delete this series and every day still to come?") {
                                                        return;
                                                    }
                                                    let set_snapshot = set_snapshot;
                                                    let set_error = set_error;
                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        match api::delete_series(&series).await {
                                                            Ok(snapshot) => {
                                                                set_snapshot.set(snapshot);
                                                                set_editing_task.set(None);
                                                            }
                                                            Err(e) => set_error.set(Some(series_note(&e))),
                                                        }
                                                    });
                                                }
                                            }
                                        >"Delete series"</button>
                                        <button type="button" on:click=move |_| set_editing_task.set(None)>"Cancel"</button>
                                        <button
                                            type="button"
                                            on:click={
                                                let occ = occ.clone();
                                                move |_| {
                                                    let Some((series_id, _)) = occ.clone() else {
                                                        return;
                                                    };
                                                    let Some(patch) = series_patch_from_dialog(
                                                        &edit_title.get_untracked(),
                                                        edit_duration.get_untracked().parse::<u16>().ok(),
                                                        &edit_weekdays.get_untracked(),
                                                        &edit_until.get_untracked(),
                                                        &edit_time.get_untracked(),
                                                    ) else {
                                                        return;
                                                    };
                                                    let set_snapshot = set_snapshot;
                                                    let set_error = set_error;
                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        match api::patch_series(&series_id, patch).await {
                                                            Ok(snapshot) => {
                                                                set_snapshot.set(snapshot);
                                                                set_editing_task.set(None);
                                                            }
                                                            Err(e) => set_error.set(Some(series_note(&e))),
                                                        }
                                                    });
                                                }
                                            }
                                        >"Save"</button>
                                    }
                                    .into_any()
                                } else {
                                    view! {
                                <button
                                    type="button"
                                    on:click={
                                        let task_id = task_id.clone();
                                        move |_| {
                                            let task_id = task_id.clone();
                                            let Some(body) = snapshot.with_untracked(|s| toggle_done_body(s, &task_id.0)) else {
                                                return;
                                            };
                                            let sink = EditSink { set_snapshot, set_error };
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
                                                // The row is behind the dialog, but it
                                                // is the user's row: it gets crossed
                                                // out like any other delete.
                                                if let Ok(new_snap) = delete_with_scribble(&task_id).await {
                                                    set_snapshot.set(new_snap);
                                                    set_editing_task.set(None);
                                                }
                                            });
                                        }
                                    }
                                >"×"</button>
                                <button type="button" on:click=move |_| set_editing_task.set(None)>"Cancel"</button>
                                <button
                                    type="button"
                                    on:click={
                                        let task_id = task_id.clone();
                                        move |_| {
                                            let task_id = task_id.clone();
                                            let title = edit_title.get_untracked().trim().to_string();
                                            let duration = edit_duration.get_untracked().parse::<u16>().ok();
                                            // Days picked: the task becomes a weekly series
                                            // (docs/ARCHITECTURE.md 7.1), and the When field does
                                            // not apply — a rule carries its own weekly time.
                                            let days = edit_weekdays.get_untracked();
                                            if !days.is_empty() {
                                                let Some(body) = series_from_dialog(
                                                    &title,
                                                    duration,
                                                    &days,
                                                    &edit_until.get_untracked(),
                                                    &edit_time.get_untracked(),
                                                ) else {
                                                    return;
                                                };
                                                let set_snapshot = set_snapshot;
                                                let set_error = set_error;
                                                wasm_bindgen_futures::spawn_local(async move {
                                                    match api::create_series(body).await {
                                                        Ok(snapshot) => {
                                                            set_snapshot.set(snapshot);
                                                            // The series takes over: drop the
                                                            // one-off row it grew out of. Not
                                                            // crossed out - the user never
                                                            // deleted it, it was absorbed.
                                                            if let Ok(snapshot) = api::delete_task(&task_id).await {
                                                                set_snapshot.set(snapshot);
                                                            } else {
                                                                set_error.set(Some(
                                                                    "The weekly series was made, but the old row stayed — delete it if you don't want it twice."
                                                                        .to_owned(),
                                                                ));
                                                            }
                                                            set_editing_task.set(None);
                                                        }
                                                        Err(e) => set_error.set(Some(series_note(&e))),
                                                    }
                                                });
                                                return;
                                            }
                                            let deadline = snapshot
                                                .with_untracked(|s| held(s, &task_id.0).cloned());
                                            let when = match typed_when(
                                                edit_when.get_untracked(),
                                                &edit_datetime.get_untracked(),
                                                deadline.as_ref(),
                                            ) {
                                                Ok(when) => when,
                                                Err(message) => {
                                                    // A clock time that names no instant
                                                    // cannot be saved: say so here and
                                                    // leave the dialog open to fix it.
                                                    set_edit_time_error.set(Some(message));
                                                    return;
                                                }
                                            };
                                            if title.is_empty() || duration.is_none() {
                                                return;
                                            }
                                            let Some(seen) = snapshot.with_untracked(|s| held(s, &task_id.0).map(|task| task.updated_at)) else {
                                                return;
                                            };
                                            let sink = EditSink { set_snapshot, set_error };
                                            let body = api::PatchTaskBody::new()
                                                .title(title)
                                                .duration_min(duration.unwrap_or(1))
                                                .when(when)
                                                .updated_at(seen);
                                            wasm_bindgen_futures::spawn_local(async move {
                                                if patch_and_show(&task_id, body, sink).await {
                                                    set_editing_task.set(None);
                                                }
                                            });
                                        }
                                    }
                                >"Save"</button>
                                    }
                                    .into_any()
                                }}
                            </div>
                        }
                    })
                })}
            </form>
        </dialog>
    }
}

fn task_time(snapshot: &Snapshot, id: &str, clock: Clock) -> String {
    snapshot
        .plan
        .placements
        .iter()
        .find(|p| p.task.0 == id)
        .map(|p| week::local_time_label(&week::planner_context(&snapshot.tz), p.start, clock))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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

    /// Reorder sends the server exactly the stored tasks, so the occurrence rows
    /// a snapshot carries are kept out of the list — and a drop that starts or
    /// lands on one is not a reorder at all.
    #[test]
    fn a_reorder_lists_stored_tasks_once_and_never_an_occurrence() {
        use row::Slot;
        let mut snapshot = api::fixture_snapshot();
        // An expanded day of a repeating series sits between the stored tasks.
        let mut occurrence = snapshot.tasks[0].clone();
        occurrence.id = TaskId("standup:2026-10-14".into());
        occurrence.source = Source::Series;
        snapshot.tasks.insert(1, occurrence);

        let first = snapshot.tasks[0].id.clone();
        let third = snapshot.tasks[2].id.clone();
        let order = reorder_ids(&snapshot, &third, &first, Slot::Before)
            .expect("stored slip onto stored slip");
        assert_eq!(
            order.first(),
            Some(&third),
            "the slip moves before its target"
        );
        assert_eq!(order.len(), 2, "only stored tasks, exactly once");
        assert!(!order.iter().any(|id| id.0.contains(':')));

        let occ = snapshot.tasks[1].id.clone();
        assert_eq!(
            reorder_ids(&snapshot, &occ, &first, Slot::Before),
            None,
            "dragged occurrence"
        );
        assert_eq!(
            reorder_ids(&snapshot, &first, &occ, Slot::Before),
            None,
            "dropped on occurrence"
        );
    }

    #[test]
    fn a_drop_below_its_target_moves_the_slip_down() {
        use row::Slot;
        let snapshot = api::fixture_snapshot();
        let first = snapshot.tasks[0].id.clone();
        let second = snapshot.tasks[1].id.clone();

        let below = reorder_ids(&snapshot, &first, &second, Slot::After)
            .expect("dropped on the lower half");
        assert_eq!(
            below.first(),
            Some(&second),
            "the slip sits after its target"
        );
        assert_eq!(below.get(1), Some(&first));

        let above = reorder_ids(&snapshot, &second, &first, Slot::Before)
            .expect("dropped on the upper half");
        assert_eq!(
            above.first(),
            Some(&second),
            "the slip sits before its target"
        );
    }

    #[test]
    fn a_slip_dropped_on_itself_keeps_its_place() {
        use row::Slot;
        let snapshot = api::fixture_snapshot();
        let first = snapshot.tasks[0].id.clone();
        assert_eq!(
            reorder_ids(&snapshot, &first, &first, Slot::Before),
            None,
            "a row dropped on itself is not a move, and re-sending the order would fight the user"
        );
    }

    #[test]
    fn a_slip_dropped_over_empty_paper_goes_to_the_end() {
        use row::Slot;
        let snapshot = api::fixture_snapshot();
        let first = snapshot.tasks[0].id.clone();
        // The finger let go where no row is: there is no target to land beside.
        let empty = TaskId("no-row-here".into());

        let order =
            reorder_ids(&snapshot, &first, &empty, Slot::Before).expect("a target the list lacks");
        assert_eq!(
            order.last(),
            Some(&first),
            "landing nowhere puts the slip at the end, where the finger left off"
        );
        let stored = snapshot
            .tasks
            .iter()
            .filter(|task| task.source != Source::Series)
            .count();
        assert_eq!(order.len(), stored, "still every stored task exactly once");
    }

    /// A row is drawn again whenever anything it shows changes, and only then.
    #[test]
    fn a_row_is_keyed_on_the_task_as_it_is_now() {
        use std::hash::{BuildHasher, RandomState};
        let task = api::fixture_snapshot().tasks[0].clone();
        let hasher = RandomState::new();
        let same = RowKey(task.clone());
        assert!(RowKey(task.clone()) == same);
        assert_eq!(
            hasher.hash_one(RowKey(task.clone())),
            hasher.hash_one(&same)
        );

        let mut renamed = task.clone();
        renamed.title = "A new title".into();
        assert!(RowKey(renamed) != same, "an edited title draws a new row");

        let mut ticked = task.clone();
        ticked.done = !task.done;
        assert!(RowKey(ticked) != same, "a tick draws a new row");

        let mut moved = task;
        moved.when = When::At(moved.created);
        moved.updated_at += chrono::Duration::seconds(1);
        assert!(RowKey(moved) != same, "a new time draws a new row");
    }

    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn list_and_week_use_the_same_task_time() {
        let snapshot: Snapshot =
            serde_json::from_str(include_str!("../../fixtures/snapshot.json")).unwrap();
        let clocks = [Clock::Twelve, Clock::TwentyFour];
        for clock in clocks {
            for task in &snapshot.tasks {
                let list_time = task_time(&snapshot, &task.id.0, clock);
                let week_time = snapshot
                    .plan
                    .placements
                    .iter()
                    .find(|p| p.task == task.id)
                    .map(|placement| {
                        week::local_time_label(
                            &week::planner_context(&snapshot.tz),
                            placement.start,
                            clock,
                        )
                    })
                    .unwrap_or_default();
                assert_eq!(list_time, week_time);
            }
        }
    }

    /// A snapshot holding one series ("Gym", Mon/Thu 07:00) and the expanded row
    /// for its Thursday, the way the server mints them (docs/ARCHITECTURE.md 7.1).
    /// The fixture already carries that series and row; this replaces the row so
    /// the `done` state is the test's to choose.
    fn snapshot_with_series(done: bool) -> Snapshot {
        let mut snapshot = api::fixture_snapshot();
        snapshot.tasks.retain(|task| task.id.0 != "sgym:2026-10-15");
        let created = Utc.with_ymd_and_hms(2026, 10, 12, 6, 0, 0).unwrap();
        snapshot.series.push(todo_core::Series {
            id: todo_core::SeriesId("sgym".into()),
            title: "Gym".into(),
            duration_min: 30,
            rule: todo_core::Recurrence::new([Weekday::Mon, Weekday::Thu], None)
                .expect("two days name a rule"),
            start_day: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            time_of_day: chrono::NaiveTime::from_hms_opt(7, 0, 0),
            zone: "UTC".into(),
            external_uid: None,
            created,
            order: -10,
        });
        snapshot.tasks.push(todo_core::Task {
            id: TaskId("sgym:2026-10-15".into()),
            title: "Gym".into(),
            duration_min: 30,
            when: When::At(Utc.with_ymd_and_hms(2026, 10, 15, 7, 0, 0).unwrap()),
            done,
            order: -10,
            label: todo_core::Label::Other,
            priority: todo_core::Priority::Medium,
            source: todo_core::Source::Series,
            created,
            updated_at: created,
            due_tz: None,
            external_uid: None,
        });
        snapshot
    }

    #[test]
    fn the_edit_sheet_opens_on_the_row_it_was_asked_for() {
        // Rambo's "Edit" and a click on the row both open the sheet through
        // `edit_form`: it must show that row, and a series row shows its rule.
        let snapshot = snapshot_with_series(false);
        let form = edit_form(&snapshot, "sgym:2026-10-15").expect("the row is listed");
        assert_eq!(form.title, "Gym");
        assert_eq!(form.duration, "30");
        assert_eq!(form.when, EditWhen::At);
        assert_eq!(form.weekdays, vec![Weekday::Mon, Weekday::Thu]);
        assert_eq!(form.time, "07:00");
        assert_eq!(form.until, "");

        let plain = snapshot
            .tasks
            .iter()
            .find(|task| occurrence_parts(&task.id.0).is_none())
            .expect("the fixture holds a plain task");
        let form = edit_form(&snapshot, &plain.id.0).expect("the row is listed");
        assert_eq!(form.title, plain.title);
        assert!(form.weekdays.is_empty() && form.time.is_empty());

        assert!(
            edit_form(&snapshot, "gone").is_none(),
            "no sheet for a task that is gone"
        );
    }

    #[test]
    fn an_occurrence_id_names_its_series_and_date() {
        let (series, date) = occurrence_parts("sgym:2026-10-15").expect("a valid occurrence id");
        assert_eq!(series, "sgym");
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 10, 15).unwrap());
        // Ordinary task ids carry no date part.
        assert!(occurrence_parts("t1234").is_none());
        assert!(occurrence_parts("sgym:not-a-date").is_none());
    }

    #[test]
    fn ticking_an_occurrence_marks_that_date_not_the_task() {
        match tick_action(&snapshot_with_series(false), "sgym:2026-10-15") {
            Some(TickAction::MarkDone { series, date }) => {
                assert_eq!(series, "sgym");
                assert_eq!(date.to_string(), "2026-10-15");
            }
            other => panic!("ticking an open occurrence marks its date, got {other:?}"),
        }
        // The same row, already done: un-ticking clears the exception, which
        // puts the day back to the rule.
        assert!(matches!(
            tick_action(&snapshot_with_series(true), "sgym:2026-10-15"),
            Some(TickAction::ClearDone { .. })
        ));
        // An ordinary task still flips its own flag, and an unknown id does nothing.
        let fixture = api::fixture_snapshot();
        let id = fixture.tasks[0].id.0.clone();
        assert!(matches!(
            tick_action(&fixture, &id),
            Some(TickAction::Task(_))
        ));
        assert!(tick_action(&fixture, "gone").is_none());
    }
}

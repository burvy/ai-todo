//! The task list: its rows, the edit dialog, and deleting with a scribble.

mod fields;
#[cfg(target_arch = "wasm32")]
mod scribble;
#[cfg(target_arch = "wasm32")]
pub(crate) mod write;

use chrono::{NaiveDate, Weekday};
use leptos::prelude::*;
use todo_core::{Snapshot, Source, TaskId, When};

use crate::clock::Clock;
use crate::edits::{confirmed, held, patch_and_show, series_note, toggle_done_body};
use crate::pickers::{PaperDate, PaperDateTime, PaperRadios, PaperTime};
use crate::store::Store;
use crate::{api, mascot, row, scrap, settings, smudge, tuning, week};
use fields::{
    EditWhen, WEEKDAY_CHOICES, due_display, local_input_value, series_from_dialog,
    series_patch_from_dialog, toggled_day, typed_when,
};
use rambl_web_core::store::{Command, Mark};
pub(crate) use rambl_web_core::task_list::occurrence_parts;
use rambl_web_core::task_list::{
    EditForm, MISSING_TITLE_NOTE, Tapped, blank_form, blank_task, is_draft, is_overdue_at,
    keep_typed, new_task_draft, reorder_ids, series_of, tapped,
};
#[cfg(target_arch = "wasm32")]
use rambl_web_core::task_list::{
    PenStep, flies_home_after_row, let_go, pen_free, pen_step, pen_writes,
};

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
pub(crate) async fn apply_tick(id: &TaskId, action: TickAction, sink: Store) {
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

#[cfg(target_arch = "wasm32")]
fn is_overdue(task: &todo_core::Task) -> bool {
    chrono::DateTime::from_timestamp_millis(js_sys::Date::now() as i64)
        .is_some_and(|now| is_overdue_at(task, now))
}

#[cfg(not(target_arch = "wasm32"))]
fn is_overdue(task: &todo_core::Task) -> bool {
    let _ = is_overdue_at(task, task.created);
    false
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
/// first. Every user-initiated delete goes through here or, when a swipe has
/// already drawn the ink, through [`delete_scribbled`], so the scribble is the
/// rule rather than a promise in a comment: the `api::delete_task` calls that
/// bypass both are the two that must not scribble, and each says why.
///
/// An occurrence of a repeat is not a stored task: deleting its row skips that one
/// date, the way the series endpoint describes (docs/ARCHITECTURE.md 7.1). Either
/// way the row's scrap is laid, to tape it back.
pub async fn delete_with_scribble(
    task_id: &todo_core::TaskId,
    snapshot: ReadSignal<Snapshot>,
    scraps: RwSignal<Vec<scrap::Scrap>>,
    store: crate::store::Store,
) -> bool {
    on_delete_requested(task_id.clone()).await;
    delete_scribbled(task_id, snapshot, scraps, store).await
}

/// Delete a task whose row is already scribbled out (the swipe drew the ink
/// itself), and lay its scrap.
pub async fn delete_scribbled(
    task_id: &todo_core::TaskId,
    snapshot: ReadSignal<Snapshot>,
    scraps: RwSignal<Vec<scrap::Scrap>>,
    store: crate::store::Store,
) -> bool {
    let command = match occurrence_parts(&task_id.0) {
        Some((series, date)) => Command::MarkOccurrence {
            series: series.to_owned(),
            date: date.to_string(),
            mark: Mark::Skipped,
        },
        None => Command::DeleteTask(task_id.clone()),
    };
    let deleted = async { store.send(command).await.is_some() };
    scrap::delete_and_lay(task_id, snapshot, scraps, deleted).await
}

/// The `li` of row `id` on the page, if it is shown.
fn row_element(id: &TaskId) -> Option<web_sys::Element> {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|doc| {
            doc.query_selector(&format!("li[data-id=\"{}\"]", id.0))
                .ok()
                .flatten()
        })
}

/// Unclips the titles of rows the pen will not write, so none is left clipped.
#[cfg(target_arch = "wasm32")]
fn unclip_rows(ids: &[TaskId]) {
    for row in ids.iter().filter_map(row_element) {
        row.class_list().remove_1("pen-pending").ok();
    }
}

/// Whether row `id` is wet: Rambo wrote it less than `INK_WET_MS` ago.
fn is_wet(written: StoredValue<Vec<(TaskId, f64)>>, id: &TaskId) -> bool {
    let now = crate::engine::now();
    written.with_value(|rows| {
        rows.iter()
            .any(|(row, at)| row == id && smudge::is_wet(now - at))
    })
}

/// Whether a point on the screen is on a row's title: what lies under the point, which
/// `elementFromPoint` finds whatever holds the pointer. The click itself cannot say,
/// because the row captures the pointer on press and the click then lands on the row.
fn on_title(x: f64, y: f64) -> bool {
    leptos::prelude::document()
        .element_from_point(x as f32, y as f32)
        .and_then(|el| el.closest(".task-open").ok().flatten())
        .is_some()
}

/// A tap at `tap` (on the screen) on row `id` while its ink is wet smears it there.
fn smudge_if_wet(
    written: StoredValue<Vec<(TaskId, f64)>>,
    smudges: RwSignal<Vec<smudge::Smudge>>,
    id: &TaskId,
    tap: (f64, f64),
) {
    if !is_wet(written, id) {
        return;
    }
    let Some(row) = row_element(id) else {
        return;
    };
    let rect = row.get_bounding_client_rect();
    let at = (tap.0 - rect.left(), tap.1 - rect.top());
    let seed = crate::engine::now().to_bits() as u32 ^ smudges.with_untracked(Vec::len) as u32;
    smudges.update(|all| all.push(smudge::smear(id.clone(), at, seed)));
}

#[component]
pub(crate) fn TaskList(
    snapshot: ReadSignal<Snapshot>,
    set_error: WriteSignal<Option<String>>,
    view_settings: ReadSignal<settings::Settings>,
    mascot_state: ReadSignal<mascot::State>,
    pen_queue: RwSignal<Vec<TaskId>>,
    pen_pos: RwSignal<(f64, f64)>,
    /// Set by the + beside the capture box: the sheet opens on a new task.
    new_task: RwSignal<bool>,
    /// The task whose edit sheet is open; Rambo listens for its change while it is.
    open_sheet: RwSignal<Option<TaskId>>,
    /// The scrap a swiped-out row leaves in the corner, to tape it back.
    scraps: RwSignal<Vec<scrap::Scrap>>,
) -> impl IntoView {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = pen_pos;
    let store = expect_context::<crate::store::Store>();
    let events = expect_context::<crate::events::Queue>();
    let (editing_task, set_editing_task) = (open_sheet.read_only(), open_sheet.write_only());
    // Rows Rambo has written and when (`engine::now`): wet for a while, and a tap then
    // smudges the ink instead of opening the row (`smudge.rs`).
    let written = StoredValue::new(Vec::<(TaskId, f64)>::new());
    let smudges = RwSignal::new(Vec::<smudge::Smudge>::new());
    let (edit_title, set_edit_title) = signal(String::new());
    let (edit_duration, set_edit_duration) = signal(String::new());
    let (edit_when, set_edit_when) = signal(EditWhen::Anytime);
    let (edit_datetime, set_edit_datetime) = signal(String::new());
    let (edit_time_error, set_edit_time_error) = signal::<Option<&'static str>>(None);
    // Save was pressed with no title: say so rather than do nothing.
    let (title_missing, set_title_missing) = signal(false);
    // The repeats block (docs/ARCHITECTURE.md 7.1): which weekdays a repeat
    // runs on, and the optional end date and time for a series.
    let edit_weekdays = RwSignal::new(Vec::<Weekday>::new());
    let edit_until = RwSignal::new(String::new());
    let edit_time = RwSignal::new(String::new());
    // What the paper pickers in the sheet need: the paper's clock, and today for
    // the day grid to open on.
    let picker_clock = Signal::derive(move || view_settings.get().clock);
    let picker_today = Signal::derive(|| {
        NaiveDate::parse_from_str(&api::today_date(), "%Y-%m-%d").unwrap_or_default()
    });
    // The sheet is a real modal: `show_modal` traps focus inside it, makes the page
    // behind it inert, lifts it into the top layer (above Rambo) and lets Escape
    // close it (the `cancel` event). It follows `editing_task`.
    let sheet = NodeRef::<leptos::html::Dialog>::new();
    // The task the + sheet is making; it lives here until Save and is never sent before.
    let draft = RwSignal::new(None::<todo_core::Task>);
    Effect::new(move |_| {
        let wanted = editing_task.get().is_some();
        if let Some(dialog) = sheet.get() {
            match (wanted, dialog.open()) {
                (true, false) => {
                    let _ = dialog.show_modal();
                }
                (false, true) => dialog.close(),
                _ => {}
            }
        }
    });
    // Open the edit sheet on a row: from a click on it, or from Rambo's "Edit".
    // What the sheet was last filled with: a refill after Rambo's heard change keeps
    // any field the user has typed in since.
    let shown = StoredValue::new(None::<EditForm>);
    let open_editor = move |id: &TaskId| {
        let Some(fresh) = snapshot.with_untracked(|s| edit_form(s, &id.0)) else {
            return;
        };
        let refill = editing_task.get_untracked().as_ref() == Some(id);
        let form = match shown.get_value() {
            Some(before) if refill => {
                let typed = EditForm {
                    title: edit_title.get_untracked(),
                    duration: edit_duration.get_untracked(),
                    when: edit_when.get_untracked(),
                    datetime: edit_datetime.get_untracked(),
                    weekdays: edit_weekdays.get_untracked(),
                    until: edit_until.get_untracked(),
                    time: edit_time.get_untracked(),
                };
                keep_typed(&before, typed, fresh.clone())
            }
            _ => fresh.clone(),
        };
        shown.set_value(Some(fresh));
        set_edit_title.set(form.title);
        set_edit_duration.set(form.duration);
        set_edit_when.set(form.when);
        set_edit_datetime.set(form.datetime);
        edit_weekdays.set(form.weekdays);
        edit_until.set(form.until);
        edit_time.set(form.time);
        set_editing_task.set(Some(id.clone()));
        set_edit_time_error.set(None);
        set_title_missing.set(false);
    };
    Effect::new(move |_| {
        events.track();
        if let Some(id) = events.take_all(rambl_web_core::events::edit_asked).pop() {
            open_editor(&TaskId(id));
        }
    });
    // The +: the same sheet, on a blank form.
    Effect::new(move |_| {
        if !new_task.get() {
            return;
        }
        new_task.set(false);
        let form = blank_form();
        let task = blank_task();
        shown.set_value(None);
        set_edit_title.set(form.title);
        set_edit_duration.set(form.duration);
        set_edit_when.set(form.when);
        set_edit_datetime.set(form.datetime);
        edit_weekdays.set(form.weekdays);
        edit_until.set(form.until);
        edit_time.set(form.time);
        set_edit_time_error.set(None);
        set_title_missing.set(false);
        set_editing_task.set(Some(task.id.clone()));
        draft.set(Some(task));
    });
    // Closed, the sheet keeps nothing of a task that was never made.
    Effect::new(move |_| {
        if editing_task.get().is_none() {
            draft.set(None);
        }
    });
    let visible_tasks = move || {
        snapshot
            .get()
            .tasks
            .into_iter()
            // A done task has torn off the list and hangs at the pin.
            .filter(|task| !task.done)
            .collect::<Vec<_>>()
    };

    // When a row goes (done, deleted), the rows below it slide up to close the gap
    // instead of jumping: each row's rest place is read after the list has changed,
    // and compared with where it rested before.
    let rested = StoredValue::new(Vec::<(String, i32)>::new());
    Effect::new(move |_| {
        snapshot.track();
        crate::engine::next_frame(move || {
            let now = scrap::row_tops();
            let slide = rested.with_value(|before| scrap::rows_to_slide(before, &now));
            rested.set_value(now);
            scrap::slide_rows(&slide);
            snapshot.with_untracked(|s| scrap::fly_rows_back(events, s));
        });
    });

    // Process pen queue: only write rows for tasks the client just created,
    // not every task on reload.
    #[cfg(target_arch = "wasm32")]
    {
        // One writer at a time: a row he is waiting to finish must not have a
        // second writer start on the queue each time his state changes.
        let writing = StoredValue::new(false);
        Effect::new(move |_| {
            if !pen_writes(view_settings.get().mode, mascot::prefers_reduced_motion()) {
                // Nothing is written: drop the queue, so his lines are not held for it.
                if pen_queue.with(|queue| !queue.is_empty()) {
                    pen_queue.with(|queue| unclip_rows(&let_go(None, queue, true)));
                    pen_queue.set(Vec::new());
                }
                return;
            }
            if pen_queue.with(Vec::is_empty) || writing.get_value() {
                return;
            }
            if pen_free(&mascot_state.get()) {
                let rambo_world = expect_context::<crate::rambo_world::RamboWorld>();
                let pen_pos = pen_pos;
                writing.set_value(true);
                wasm_bindgen_futures::spawn_local(async move {
                    // Rows captured while he writes join the queue and are
                    // written in turn.
                    let next = move || {
                        pen_queue
                            .try_update(|queue| (!queue.is_empty()).then(|| queue.remove(0)))
                            .flatten()
                    };
                    while let Some(task_id) = next() {
                        let row = row_element(&task_id);
                        let in_view = row.as_ref().is_some_and(write::in_view);
                        match pen_step(&mascot_state.get_untracked(), in_view) {
                            PenStep::Stop => {
                                pen_queue.with_untracked(|queue| {
                                    unclip_rows(&let_go(Some(&task_id), queue, true));
                                });
                                pen_queue.set(Vec::new());
                                break;
                            }
                            PenStep::Skip => {
                                unclip_rows(&let_go(Some(&task_id), &[], false));
                                rambo_world.tell(mascot::Event::SkipRow);
                                continue;
                            }
                            PenStep::Write => {}
                        }
                        rambo_world.tell(mascot::Event::WriteRow);
                        // Clip the title and park the nib at the line's start.
                        // The fonts settle first: parking on fallback metrics
                        // would put the pen somewhere the line does not start.
                        if let Some(row) = row.as_ref() {
                            row.class_list().add_1("pen-pending").ok();
                        }
                        write::wait_for_fonts().await;
                        // He flies to the row where it is on each frame, not where
                        // it was when he set off.
                        let following = row.clone().map(|row| write::follow_row(row, pen_pos));
                        crate::engine::wait(f64::from(crate::tuning::FLY_MS)).await;
                        if let Some(following) = following {
                            following.stop();
                        }
                        rambo_world.tell(mascot::Event::WriteRow);
                        if let Some(row) = row.as_ref() {
                            write::write_row_title(row, pen_pos, mascot_state, rambo_world).await;
                            // The ink he has just laid is wet, and shines until it dries.
                            let now = crate::engine::now();
                            written.update_value(|rows| {
                                rows.retain(|(_, at)| smudge::is_wet(now - at));
                                rows.push((task_id.clone(), now));
                            });
                            if let Some(style) = write::inline_style(row) {
                                style
                                    .set_property(
                                        "--ink-wet-ms",
                                        &format!("{}ms", tuning::INK_WET_MS),
                                    )
                                    .ok();
                            }
                            row.class_list().add_1("ink-wet").ok();
                        }
                        if flies_home_after_row(&mascot_state.get_untracked()) {
                            rambo_world.tell(mascot::Event::FlyHome);
                            crate::engine::wait(f64::from(crate::tuning::FLY_MS)).await;
                            rambo_world.tell(mascot::Event::FlyHome);
                        }
                    }
                    writing.set_value(false);
                });
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (pen_queue, mascot_state, view_settings);

    view! {
        <section aria-label="Tasks">
            <ul class="task-list">
                <For each=visible_tasks key=|task| RowKey(task.clone()) let:task>
                    {
                        let task_id = task.id.clone();
                        // Queued before the list shows (see `Store::send_with`): the row
                        // is clipped from its first frame, so its title never shows whole.
                        let queued = pen_queue.with_untracked(|queue| queue.contains(&task_id));
                        let task_title = task.title.clone();
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

                        let delete_task = {
                            let task_id_clone = task_id.clone();
                            move |ev: leptos::ev::MouseEvent| {
                                ev.prevent_default();
                                ev.stop_propagation();
                                let task_id_clone = task_id_clone.clone();
                                wasm_bindgen_futures::spawn_local(async move {
                                    // Rambo crosses the title out first; the task
                                    // only leaves once the ink has dried.
                                    delete_with_scribble(&task_id_clone, snapshot, scraps, store).await;
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
                                // Dazed, Rambo takes every hold on the page, so the
                                // row does not lift for one.
                                let lifts = mascot::drawn_over(
                                    &mascot_state.get_untracked(),
                                    &crate::gesture::Gesture::Hold,
                                )
                                .is_none();
                                row::dom::down(&presses, &ev, lifts);
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
                            let drop_task_id_clone = drop_task_id.clone();
                            move |ev: leptos::ev::PointerEvent| {
                                let Some((outcome, row_id, x, y)) = row::dom::up(&presses, &ev)
                                else {
                                    return;
                                };
                                let dragged = TaskId(row_id);
                                match outcome {
                                    // A tap on a control never reaches here (the
                                    // row does not take that press); a scroll and a
                                    // backed-out swipe leave the list as it was.
                                    row::Release::Tap => {
                                        let wet = is_wet(written, &dragged);
                                        match tapped(wet, on_title(x, y)) {
                                            Tapped::Smudge => {
                                                smudge_if_wet(written, smudges, &dragged, (x, y))
                                            }
                                            Tapped::Open => open_editor(&dragged),
                                            Tapped::Nothing => {}
                                        }
                                    }
                                    row::Release::Scroll | row::Release::SnapBack => {}
                                    row::Release::Done => {
                                        let task_id = TaskId(drop_task_id_clone.clone());
                                        #[cfg(target_arch = "wasm32")]
                                        crate::sound::play(crate::sound::Cue::Scribble);
                                        wasm_bindgen_futures::spawn_local(async move {
                                            // The swipe drew the ink; it finishes across the row,
                                            // and the row, done, tears off and flies to the pin.
                                            crate::engine::wait(f64::from(tuning::SCRIBBLE_FINISH_MS)).await;
                                            scrap::note_departure(events, &task_id.0);
                                            if let Some(action) = snapshot.with_untracked(|s| tick_action(s, &task_id.0)) {
                                                apply_tick(&task_id, action, store).await;
                                            }
                                            snapshot.with_untracked(|s| scrap::settle_departure(events, s, &task_id.0));
                                            // Refused: the ink comes off and the row stays.
                                            if snapshot.with_untracked(|s| scrap::tick_refused(s, &task_id.0)) {
                                                row::dom::clear_ink(&task_id.0);
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
                                        store.dispatch(Command::Reorder(new_order));
                                    }
                                }
                            }
                        };
                        let handle_pointer_cancel = {
                            let presses = presses.clone();
                            move |_: leptos::ev::PointerEvent| row::dom::cancel(&presses)
                        };
                        let handle_pointer_lost = handle_pointer_cancel.clone();
                        view! {
                    <li
                        class="task-row"
                        class:pen-pending=queued
                        data-id={render_task_id_attr}
                        data-title={render_task_title.clone()}
                        on:pointerdown=handle_pointer_down
                        on:pointermove=handle_pointer_move
                        on:pointerup=handle_pointer_up
                        on:pointercancel=handle_pointer_cancel
                        // The page turn takes the pointer from the row; the row gets no pointerup then.
                        on:lostpointercapture=handle_pointer_lost
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
                        // Reached by Tab, Enter or Space opens the row as a tap on it
                        // does; a key leaves no fingerprint, so wet ink does not stop it.
                        <span
                            class="task-open"
                            tabindex="0"
                            role="button"
                            on:keydown={
                                let id = task.id.clone();
                                move |ev: leptos::ev::KeyboardEvent| {
                                    if matches!(ev.key().as_str(), "Enter" | " ") {
                                        ev.prevent_default();
                                        open_editor(&id);
                                    }
                                }
                            }
                        >
                            <span class="pen-title-wrap">
                                <strong class="pen-title">{render_task_title.clone()}</strong>
                                {repeat_mark}
                            </span>
                            <span class="task-meta">
                                " — " {task.duration_min} " min"
                                {move || {
                                    let time = task_time(&snapshot.get(), &render_task_id, view_settings.get().clock);
                                    (!time.is_empty()).then(|| view! { <span class="task-time">" · " {time}</span> })
                                }}
                            </span>
                            {is_overdue(&task).then(|| view! { <span class="overdue">"Overdue"</span> })}
                        </span>
                        {(!is_occurrence).then(|| view! {
                            <button
                                class="delete-mark"
                                type="button"
                                on:click=delete_task
                                aria-label=format!("Delete {}", task_title)
                            >"×"</button>
                        })}
                        <smudge::Smudges task=task.id.clone() smudges />
                    </li>
                        }

                    }
                </For>
            </ul>
        </section>
        <dialog
            class="task-dialog"
            node_ref=sheet
            on:cancel=move |_: leptos::ev::Event| set_editing_task.set(None)
            on:close=move |_: leptos::ev::Event| set_editing_task.set(None)
        >
            <form method="dialog">
                <h3>{move || match editing_task.get() {
                    Some(id) if is_draft(&id) => "New task",
                    Some(id) if occurrence_parts(&id.0).is_some() => "Edit series",
                    _ => "Edit task",
                }}</h3>
                {move || editing_task.get().and_then(|id| {
                    snapshot
                        .get()
                        .tasks
                        .into_iter()
                        .find(|task| task.id == id)
                        .or_else(|| draft.get().filter(|task| task.id == id))
                        .map(|task| {
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
                                    on:input=move |event| {
                                        set_edit_title.set(event_target_value(&event));
                                        set_title_missing.set(false);
                                    }
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
                                <div class="dialog-row">
                                    "Until (optional) "
                                    <PaperDate
                                        label="Until"
                                        value=edit_until
                                        set=move |day: String| edit_until.set(day)
                                        start=picker_today
                                    />
                                </div>
                                <div class="dialog-row">
                                    "At (optional) "
                                    <PaperTime
                                        label="At"
                                        value=edit_time
                                        set=move |time: String| edit_time.set(time)
                                        clock=picker_clock
                                    />
                                </div>
                                {(!recurring).then(|| view! {
                                    <p class="dialog-hint">
                                        "Pick days and save: this task becomes a weekly series, and its one-off row goes away."
                                    </p>
                                })}
                            </fieldset>
                            {(!recurring).then(|| view! {
                            <fieldset>
                                <legend>"When"</legend>
                                <PaperRadios
                                    label="When"
                                    options=vec![
                                        (EditWhen::Anytime, "Anytime"),
                                        (EditWhen::DueBy, "Due by"),
                                        (EditWhen::At, "Scheduled at"),
                                    ]
                                    value=edit_when
                                    set=move |kind: EditWhen| set_edit_when.set(kind)
                                />
                                <PaperDateTime
                                    value=edit_datetime
                                    set=move |value: String| {
                                        set_edit_datetime.set(value);
                                        set_edit_time_error.set(None);
                                    }
                                    clock=picker_clock
                                    start=picker_today
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
                            {move || title_missing.get().then(|| view! {
                                <p class="time-error" role="alert">{MISSING_TITLE_NOTE}</p>
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
                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        // Done and Undone are the same button on
                                                        // this date: marked writes the exception,
                                                        // un-marked clears it.
                                                        let date = date.to_string();
                                                        let command = if task_done {
                                                            Command::ClearOccurrence { series, date }
                                                        } else {
                                                            Command::MarkOccurrence { series, date, mark: Mark::Done }
                                                        };
                                                        if store.send(command).await.is_some() {
                                                            set_editing_task.set(None);
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
                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        let command = Command::MarkOccurrence {
                                                            series,
                                                            date: date.to_string(),
                                                            mark: Mark::Skipped,
                                                        };
                                                        if store.send(command).await.is_some() {
                                                            set_editing_task.set(None);
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
                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        if store.send(Command::DeleteSeries(series)).await.is_some() {
                                                            set_editing_task.set(None);
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
                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        let command = Command::PatchSeries { series: series_id, patch };
                                                        if store.send(command).await.is_some() {
                                                            set_editing_task.set(None);
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
                                    hidden=is_draft(&task_id)
                                    on:click={
                                        let task_id = task_id.clone();
                                        move |_| {
                                            let task_id = task_id.clone();
                                            let Some(body) = snapshot.with_untracked(|s| toggle_done_body(s, &task_id.0)) else {
                                                return;
                                            };
                                            let sink = store;
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
                                    hidden=is_draft(&task_id)
                                    on:click={
                                        let task_id = task_id.clone();
                                        move |_| {
                                            let task_id = task_id.clone();
                                            wasm_bindgen_futures::spawn_local(async move {
                                                // The row is behind the dialog, but it
                                                // is the user's row: it gets crossed
                                                // out like any other delete.
                                                if delete_with_scribble(&task_id, snapshot, scraps, store).await {
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
                                            if title.is_empty() {
                                                set_title_missing.set(true);
                                                return;
                                            }
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
                                                wasm_bindgen_futures::spawn_local(async move {
                                                    if store.send(Command::CreateSeries(body)).await.is_none() {
                                                        return;
                                                    }
                                                    // The series takes over: drop the one-off row it
                                                    // grew out of. Not crossed out - the user never
                                                    // deleted it, it was absorbed.
                                                    if !is_draft(&task_id)
                                                        && store.send(Command::DeleteTask(task_id)).await.is_none()
                                                    {
                                                        set_error.set(Some(
                                                            "The weekly series was made, but the old row stayed — delete it if you don't want it twice."
                                                                .to_owned(),
                                                        ));
                                                    }
                                                    set_editing_task.set(None);
                                                });
                                                return;
                                            }
                                            if is_draft(&task_id) {
                                                let when = match typed_when(
                                                    edit_when.get_untracked(),
                                                    &edit_datetime.get_untracked(),
                                                    Some(&blank_task()),
                                                ) {
                                                    Ok(when) => when,
                                                    Err(message) => {
                                                        set_edit_time_error.set(Some(message));
                                                        return;
                                                    }
                                                };
                                                let Some(made) = new_task_draft(&title, duration, when) else {
                                                    return;
                                                };
                                                wasm_bindgen_futures::spawn_local(async move {
                                                    if store.send(Command::CreateTask(made)).await.is_some() {
                                                        set_editing_task.set(None);
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
                                            let sink = store;
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
    /// The list's ruled lines are one grid on the paper, and rows are laid on it: the grid
    /// is drawn on the list itself at the pitch, and a row's height and its text's line
    /// height are the pitch, so a wrapped title takes whole rules.
    #[test]
    fn the_rows_are_laid_on_one_grid_of_rules() {
        let css = include_str!("../../theme.css");
        let rule = |selector: &str| {
            css.split(selector)
                .nth(1)
                .and_then(|rest| rest.split('}').next())
                .unwrap_or_else(|| panic!("theme.css styles {selector}"))
                .to_owned()
        };
        let list = rule(
            "
.task-list {",
        );
        let row = rule(".task-list li {");
        let text = rule(".task-open {");
        assert!(list.contains("repeating-linear-gradient"), "{list}");
        assert!(list.matches("var(--line-pitch)").count() >= 2, "{list}");
        assert!(list.contains("var(--paper-line)"), "{list}");
        assert!(row.contains("min-height: var(--line-pitch)"), "{row}");
        assert!(row.contains("padding: 0"), "{row}");
        assert!(
            !row.contains("border-bottom"),
            "the grid is the rules: {row}"
        );
        assert!(text.contains("line-height: var(--line-pitch)"), "{text}");
        assert!(text.contains("padding: 0"), "{text}");
    }

    /// A horizontal swipe on the open edit sheet is the page's to ignore: without this the
    /// browser takes it as its history "back" and leaves the app for a blank page.
    #[test]
    fn a_stray_swipe_never_becomes_the_browsers_back() {
        // The file is CRLF in a Windows checkout and LF in Git.
        let css = include_str!("../../theme.css").replace("\r\n", "\n");
        let rule = css
            .split("html,\nbody {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("theme.css has an html, body rule");
        assert!(rule.contains("overscroll-behavior-x: none"), "{rule}");
    }

    /// FRONTEND-PLAN 1.8 item 7: the edit sheet is a real modal (focus trapped, page
    /// inert, Escape closes), not a dialog merely given the `open` attribute.
    #[test]
    fn the_edit_sheet_opens_as_a_modal_not_with_the_open_attribute() {
        let source = include_str!("mod.rs");
        let sheet = &source[source.find("<dialog").expect("the sheet")..];
        let tag = &sheet[..sheet.find('>').expect("end of the tag")];
        assert!(
            !tag.contains("open="),
            "the sheet must not be opened by attribute: {tag}"
        );
        assert!(source.contains("dialog.show_modal()"));
    }

    use super::*;
    use chrono::{TimeZone, Utc};

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
            off_planner: false,
        });
        snapshot
    }

    #[test]
    fn a_refill_keeps_what_the_user_typed() {
        let shown = edit_form(&snapshot_with_series(false), "sgym:2026-10-15").expect("listed");
        let mut typed = shown.clone();
        typed.title = "Gym with Sam".into();
        let mut fresh = shown.clone();
        fresh.title = "Gym (server)".into();
        fresh.datetime = "2026-10-16T15:00".into();

        let refilled = keep_typed(&shown, typed, fresh);
        assert_eq!(refilled.title, "Gym with Sam", "typed meanwhile: kept");
        assert_eq!(
            refilled.datetime, "2026-10-16T15:00",
            "untouched: the server's new value"
        );
        assert_eq!(refilled.duration, shown.duration);
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

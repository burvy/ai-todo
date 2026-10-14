use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Timelike, Utc, Weekday};
use gloo_timers::callback::Timeout;
#[cfg(target_arch = "wasm32")]
use js_sys::Array;
use leptos::prelude::*;
use todo_core::Snapshot;
use todo_core::{Label, Priority, Source, TaskDraft, TaskId, When};
use wasm_bindgen::JsCast;

pub mod account;
pub mod api;
pub mod brain;
pub mod mascot;
pub mod pen;
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

/// The order the server wants after moving `dragged` before `target`, or `None`
/// when the drop is not a reorder at all.
///
/// The server renumbers stored tasks only (`ids must list every task exactly once`),
/// and a snapshot lists series occurrences too — rows with `{series}:{date}` ids
/// that are expanded on the fly and share the series' row. A slip dragged from or
/// onto one is not a reorder, so the handler skips the call instead of failing 400.
fn reorder_ids(snapshot: &Snapshot, dragged: &TaskId, target: &TaskId) -> Option<Vec<TaskId>> {
    if snapshot
        .tasks
        .iter()
        .any(|task| task.source == Source::Series && (task.id == *dragged || task.id == *target))
    {
        return None;
    }
    let mut new_order = Vec::new();
    let mut found_target = false;
    for task in &snapshot.tasks {
        if task.source == Source::Series {
            continue;
        }
        if task.id == *dragged {
            continue;
        }
        if task.id == *target && !found_target {
            new_order.push(dragged.clone());
            found_target = true;
        }
        new_order.push(task.id.clone());
    }
    if !found_target {
        new_order.push(dragged.clone());
    }
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

/// The body that flips `id` between done and not done, against the version held now.
pub(crate) fn toggle_done_body(snapshot: &Snapshot, id: &str) -> Option<api::PatchTaskBody> {
    let task = held(snapshot, id)?;
    Some(
        api::PatchTaskBody::new()
            .done(!task.done)
            .updated_at(task.updated_at),
    )
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
enum TickAction {
    Task(api::PatchTaskBody),
    MarkDone { series: String, date: NaiveDate },
    ClearDone { series: String, date: NaiveDate },
}

fn tick_action(snapshot: &Snapshot, id: &str) -> Option<TickAction> {
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

/// The dialog's day buttons, Monday first, each weekday with its short name.
const WEEKDAY_CHOICES: [(Weekday, &str); 7] = [
    (Weekday::Mon, "Mon"),
    (Weekday::Tue, "Tue"),
    (Weekday::Wed, "Wed"),
    (Weekday::Thu, "Thu"),
    (Weekday::Fri, "Fri"),
    (Weekday::Sat, "Sat"),
    (Weekday::Sun, "Sun"),
];

/// The days a series repeats on, Monday-first without duplicates — what the
/// wire wants, whatever order the buttons were clicked in.
fn sorted_weekdays(days: &[Weekday]) -> Vec<Weekday> {
    let mut days: Vec<Weekday> = days.to_vec();
    days.sort_by_key(|day| day.num_days_from_monday());
    days.dedup();
    days
}

/// Picking a picked day unpicks it.
fn toggled_day(days: Vec<Weekday>, day: Weekday) -> Vec<Weekday> {
    let mut days = days;
    if days.contains(&day) {
        days.retain(|picked| *picked != day);
    } else {
        days.push(day);
    }
    days
}

/// `HH:MM` from the dialog's time field, or none when left empty; an
/// unparsable value is treated as empty, and the field never offers anything
/// else its type cannot produce.
fn parse_hhmm(value: &str) -> Option<String> {
    chrono::NaiveTime::parse_from_str(value, "%H:%M")
        .ok()
        .map(|time| time.format("%H:%M").to_string())
}

/// `YYYY-MM-DD` from the dialog's end-date field, or none when left empty.
fn parse_ymd(value: &str) -> Option<String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .map(|date| date.to_string())
}

/// The series the dialog's fields describe, or `None` when they name no rule
/// yet (no title, no length, no day picked): the caller leaves the dialog
/// open, the way it already does for an ordinary edit that is not filled in.
fn series_from_dialog(
    title: &str,
    duration: Option<u16>,
    days: &[Weekday],
    until: &str,
    time: &str,
) -> Option<api::SeriesBody> {
    let title = title.trim();
    if title.is_empty() || duration.is_none() || days.is_empty() {
        return None;
    }
    Some(api::SeriesBody {
        title: title.to_owned(),
        duration_min: duration.unwrap_or(1),
        weekdays: sorted_weekdays(days),
        time_of_day: parse_hhmm(time),
        until: parse_ymd(until),
    })
}

/// The series edit the dialog's fields describe. An absent field keeps what
/// the series already has: the server cannot clear one (`null` reads as
/// absent), so an emptied end date or time changes nothing.
fn series_patch_from_dialog(
    title: &str,
    duration: Option<u16>,
    days: &[Weekday],
    until: &str,
    time: &str,
) -> Option<api::SeriesPatchBody> {
    let title = title.trim();
    if title.is_empty() || duration.is_none() || days.is_empty() {
        return None;
    }
    Some(api::SeriesPatchBody {
        title: Some(title.to_owned()),
        duration_min: duration,
        weekdays: Some(sorted_weekdays(days)),
        time_of_day: parse_hhmm(time),
        until: parse_ymd(until),
    })
}

/// The note when a series write is refused: the server's own words say best
/// why (a day the rule never repeats, a series gone, the network down). A
/// series has no `updated_at` to go stale, so unlike a task edit there is
/// nothing to reload first.
fn series_note(error: &api::ApiError) -> String {
    match error {
        api::ApiError::Server(e) => e.message.clone(),
        other => other.to_string(),
    }
}

/// The browser's own confirm dialog — for the one destructive button, so a
/// whole series (every day still to come) is never deleted by a slip. The
/// native test build has no window and never confirms.
fn confirmed(message: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .is_some_and(|window| window.confirm_with_message(message).is_ok_and(|ok| ok))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = message;
        false
    }
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
pub(crate) struct EditSink {
    pub(crate) set_snapshot: WriteSignal<Snapshot>,
    pub(crate) set_error: WriteSignal<Option<String>>,
    pub(crate) view_settings: ReadSignal<settings::Settings>,
}

/// Sends an edit and shows what the server now holds. Returns whether the edit applied.
///
/// A refused edit (the task changed on another device, or the network failed) is not an
/// error to hide: the list is reloaded so the next click starts from the server's copy,
/// and a short note says why nothing happened.
pub(crate) async fn patch_and_show(
    id: &todo_core::TaskId,
    body: api::PatchTaskBody,
    sink: EditSink,
) -> bool {
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
    // The page turn is the browser's own scroll: `main` is a scroll-snap row, so a
    // real touch never reaches a pointer handler — the browser scrolls and
    // cancels the pointer. The curl therefore follows the scroll, and stops
    // when the scroll settles. Some(true) = turning forward (list → planner).
    let page = RwSignal::new(0_u8);
    let turning = RwSignal::new(None::<bool>);
    let settling = std::rc::Rc::new(std::cell::RefCell::new(None::<Timeout>));
    #[cfg(target_arch = "wasm32")]
    let curl = move |_: leptos::ev::Event| {
        let Some(element) = main_ref.get() else {
            return;
        };
        set_pane.set(nearest_pane(&element));
        turning.set(curl_for(page.get_untracked(), nearest_pane(&element)));
        if let Some(timer) = settling.borrow_mut().take() {
            timer.cancel();
        }
        let (main_ref, page, turning) = (main_ref, page, turning);
        settling
            .borrow_mut()
            .replace(Timeout::new(SETTLE_MS, move || {
                let Some(element) = main_ref.get() else {
                    return;
                };
                page.set(nearest_pane(&element));
                turning.set(None);
            }));
    };
    #[cfg(not(target_arch = "wasm32"))]
    let curl = |_: leptos::ev::Event| {};
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (page, turning, settling);
    let (mascot_state, set_mascot_state) = signal(mascot::State::Home);
    let (talking, set_talking) = signal(false);
    let pen_pos = RwSignal::new((0.0_f64, 0.0));
    // Transcript buffer: Rambo fills it while listening, CaptureBox reads it
    let capture_text = RwSignal::new(String::new());
    // Pen queue: task IDs the client just created, so the pen writes only those rows
    let pen_queue = RwSignal::new(Vec::<TaskId>::new());
    // Capture form ref so Rambo can requestSubmit on release
    let form_ref = NodeRef::<leptos::html::Form>::new();

    window_event_listener(leptos::ev::keydown, move |event| {
        if event.key() == " "
            && !matches!(
                event.target(),
                Some(target) if target.dyn_ref::<web_sys::HtmlInputElement>().is_some()
                    || target.dyn_ref::<web_sys::HtmlTextAreaElement>().is_some()
            )
        {
            set_talking.set(true);
        }
    });

    view! {
        <header>
            <h1>"Rambl"</h1>
            <account::HeaderLink />
            <settings::GearMenu
                settings=view_settings
                set_settings=set_view_settings
                set_snapshot=set_snapshot
                set_error=set_error
            />
            <CaptureBox
                form_ref
                snapshot=snapshot
                set_snapshot=set_snapshot
                state=mascot_state
                set_state=set_mascot_state
                view_settings
                pen_pos
                capture_text
                pen_queue
                talking
                set_talking
                set_error
                brain_speech
                brain_pose
                set_brain_event
            />
        </header>
        {move || error.get().map(|e| view! {
            <div class="error" aria-live="polite">{e}</div>
        })}
        <brain::Controller snapshot mode=Signal::derive(move || view_settings.get().mode) event=brain_event set_event=set_brain_event set_speech=set_brain_speech set_pose=set_brain_pose />
        <nav class="pane-switch" aria-label="Show">
            <button
                type="button"
                class:active=move || pane.get() == 0
                aria-pressed=move || (pane.get() == 0).to_string()
                on:click=move |_| show_pane(main_ref, 0, set_pane)
            >"To do"</button>
            <button
                type="button"
                class:active=move || pane.get() == 1
                aria-pressed=move || (pane.get() == 1).to_string()
                on:click=move |_| show_pane(main_ref, 1, set_pane)
            >"Planner"</button>
        </nav>
        <main
                node_ref=main_ref
                on:scroll=curl
                class=move || match turning.get() {
                    Some(true) => "turning-forward",
                    Some(false) => "turning-back",
                    None => "",
                }
            >
            <div class="tasks-pane">
                <TaskList snapshot set_snapshot set_error view_settings mascot_state set_mascot_state pen_queue pen_pos />
            </div>
            <week::WeekGrid
                snapshot=snapshot
                settings=view_settings
                set_snapshot=set_snapshot
                set_error=set_error
                capture_text
                set_mascot_state=set_mascot_state
            />
        </main>
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

/// The page `main` is scrolled to, nearest pane first.
#[cfg(target_arch = "wasm32")]
fn nearest_pane(element: &web_sys::HtmlElement) -> u8 {
    pane_at(
        f64::from(element.scroll_left()),
        f64::from(element.client_width()),
    )
}

/// Milliseconds of quiet after the last scroll event before the turn is called
/// over. `scrollend` is not everywhere yet, and a fixed guess is worse than the
/// scroll going quiet: while the pages are still moving, scroll events keep
/// arriving and the curl keeps its lean.
#[cfg(target_arch = "wasm32")]
const SETTLE_MS: u32 = 220;

/// Which way the curl leans while the pages are turning, and nothing at all
/// when the scroll has not left the page it settled on. `from` is the page the
/// last turn ended on, `nearest` the page the scroll is closest to now.
#[cfg(any(target_arch = "wasm32", test))]
fn curl_for(from: u8, nearest: u8) -> Option<bool> {
    (from != nearest).then_some(nearest == 1)
}

/// Rambo's whisper: short handwritten prompts the empty capture line cycles
/// through, one every few seconds, as if he's quietly suggesting what to type.
const WHISPER_PROMPTS: [&str; 6] = [
    "Ramble on…",
    "What's on today?",
    "Tell me everything.",
    "Anything due soon?",
    "One line at a time.",
    "It can wait till later.",
];
const WHISPER_ROTATE_MS: u32 = 6000;
const WHISPER_FADE_MS: u32 = 450;

fn next_whisper(current: usize, len: usize) -> usize {
    if len == 0 { 0 } else { (current + 1) % len }
}

/// Rotation is motion; a user who asked the system for reduced motion gets
/// neither the fade nor the rotation, just one quiet line.
fn whisper_rotates(reduced_motion: bool) -> bool {
    !reduced_motion
}

fn prefers_reduced_motion() -> bool {
    web_sys::window()
        .and_then(|window| {
            window
                .match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
        })
        .map(|media| media.matches())
        .unwrap_or(false)
}

/// Keep the whisper turning: wait, dip the placeholder colour to transparent
/// (the `.whisper-fade` class), swap the text at the bottom of the dip and
/// let the colour return. Re-arms forever; on a reduced-motion system the
/// tick is a no-op, so the line stays put and the CSS transition is off.
fn schedule_whisper(index: RwSignal<usize>, fading: RwSignal<bool>) {
    Timeout::new(WHISPER_ROTATE_MS, move || {
        if whisper_rotates(prefers_reduced_motion()) {
            fading.set(true);
            Timeout::new(WHISPER_FADE_MS, move || {
                index.update(|current| *current = next_whisper(*current, WHISPER_PROMPTS.len()));
                fading.set(false);
            })
            .forget();
        }
        schedule_whisper(index, fading);
    })
    .forget();
}

/// What a submit adds: Rambo's transcript when he just heard one, else the box.
///
/// A release submits the form in the same tick that hands over the transcript, before
/// the box has caught up with it, so the box alone would send the text from before.
/// Typing clears the transcript, so an edited box always wins. Blank is nothing to add.
fn capture_title(transcript: &str, typed: &str) -> Option<String> {
    let source = if transcript.trim().is_empty() {
        typed
    } else {
        transcript
    };
    let title = source.trim();
    (!title.is_empty()).then(|| title.to_owned())
}

/// The note when a capture was not added, naming it so it is never silently lost.
fn capture_note(title: &str, error: &api::ApiError) -> String {
    match error {
        api::ApiError::Network(_) => {
            format!("Could not reach the server, so \"{title}\" was not added.")
        }
        api::ApiError::Server(refusal) => {
            format!("\"{title}\" was not added: {}", refusal.message)
        }
        api::ApiError::Decode(_) => {
            format!("\"{title}\" may not have been added. Reload to check.")
        }
    }
}

#[component]
fn CaptureBox(
    form_ref: NodeRef<leptos::html::Form>,
    snapshot: ReadSignal<Snapshot>,
    set_snapshot: WriteSignal<Snapshot>,
    state: ReadSignal<mascot::State>,
    set_state: WriteSignal<mascot::State>,
    view_settings: ReadSignal<settings::Settings>,
    pen_pos: RwSignal<(f64, f64)>,
    capture_text: RwSignal<String>,
    pen_queue: RwSignal<Vec<TaskId>>,
    talking: ReadSignal<bool>,
    set_talking: WriteSignal<bool>,
    set_error: WriteSignal<Option<String>>,
    brain_speech: ReadSignal<Option<String>>,
    brain_pose: ReadSignal<String>,
    set_brain_event: WriteSignal<Option<rambl_brain::Event>>,
) -> impl IntoView {
    let (text, set_text) = signal(String::new());
    // Rambo peeks over at you while you type. He lives in the capture form, so
    // the peek signal belongs with the input that drives it.
    let (typing, set_typing) = signal(false);

    // Watch for speech transcript from Rambo: when capture_text changes, fill the input
    Effect::new(move |_| {
        let transcript = capture_text.get();
        if !transcript.is_empty() {
            set_text.set(transcript);
        }
    });
    let input_ref = NodeRef::<leptos::html::Input>::new();

    // Rambo's whisper: the placeholder is the rotating prompt. Read the index
    // before the node ref so a ref that loads later can't drop a swap.
    let whisper = RwSignal::new(0);
    let whisper_fading = RwSignal::new(false);
    Effect::new(move |_| {
        let index = whisper.get();
        let Some(input) = input_ref.get() else {
            return;
        };
        input.set_placeholder(WHISPER_PROMPTS[index]);
    });
    Effect::new(move |_| {
        let fading = whisper_fading.get();
        let Some(input) = input_ref.get() else {
            return;
        };
        let _ = input.class_list().toggle_with_force("whisper-fade", fading);
    });
    schedule_whisper(whisper, whisper_fading);

    // Caret follow: on input, position pen at caret; after 2s of inactivity, fly home
    let linger_timer = std::rc::Rc::new(std::cell::RefCell::new(None::<Timeout>));
    // Peeking: Rambo looks up while you type and stops when you stop.
    let peek_timer = std::rc::Rc::new(std::cell::RefCell::new(None::<Timeout>));
    let restart_linger = {
        let linger_timer = linger_timer.clone();
        move || {
            if let Some(timer) = linger_timer.borrow_mut().take() {
                timer.cancel();
            }
            let timer = Timeout::new(pen::LINGER_MS, move || {
                set_state.update(|s| {
                    *s = mascot::transition(s, mascot::Event::FlyHome);
                });
                let set_state = set_state;
                let timer = Timeout::new(pen::FLY_MS, move || {
                    // Use transition() so a grab during flight is not overridden.
                    set_state.update(|s| {
                        *s = mascot::transition(s, mascot::Event::FlyHome);
                    });
                });
                timer.forget();
            });
            linger_timer.borrow_mut().replace(timer);
        }
    };

    let measure_caret = move || {
        #[cfg(target_arch = "wasm32")]
        {
            let Some(input_el) = input_ref.get() else {
                return;
            };
            let el: web_sys::Element = input_el.clone().into();
            let rect = el.get_bounding_client_rect();
            let sel = input_el.selection_start().ok().flatten().unwrap_or(0);
            let caret_x = rect.left() + f64::from(sel) * 8.0;
            let caret_y = rect.top() + rect.height() / 2.0;
            let center = pen::writing_pen_center(caret_x, caret_y);
            // Convert to offset from Rambo's home position
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                if let Some(rambo) = doc.query_selector(".rambo").ok().flatten() {
                    let rrect = rambo.get_bounding_client_rect();
                    let hx = rrect.left() + rrect.width() / 2.0;
                    let hy = rrect.top() + rrect.height() / 2.0;
                    pen_pos.set((center.0 - hx, center.1 - hy));
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (input_ref, pen_pos);
        }
    };

    let handle_input = move |ev: leptos::ev::Event| {
        // Keep the typed text first; the pen choreography below may bail out early.
        set_text.set(event_target_value(&ev));
        // What is typed now is what a submit sends, not an older transcript.
        capture_text.set(String::new());
        // He looks up at you as you type, and goes back to what he was doing
        // PEEK_MS after the last keystroke.
        set_typing.set(true);
        if let Some(timer) = peek_timer.borrow_mut().take() {
            timer.cancel();
        }
        peek_timer
            .borrow_mut()
            .replace(Timeout::new(mascot::PEEK_MS, move || {
                set_typing.set(false);
            }));
        let st = state.get_untracked();
        if mascot::pen_busy(&st) {
            return;
        }
        if matches!(st, mascot::State::Dragging) {
            return;
        }
        if view_settings.get_untracked().mode == settings::RamblMode::Lobotomized {
            return;
        }
        set_state.set(mascot::transition(&st, mascot::Event::WriteCaret));
        measure_caret();
        restart_linger();
    };

    // One capture in flight at a time: a second Enter while the first is still on its
    // way would add the same task twice.
    let sending = RwSignal::new(false);
    let handle_submit = {
        let linger_timer = linger_timer.clone();
        move |ev: leptos::ev::SubmitEvent| {
            ev.prevent_default();
            if sending.get_untracked() {
                return;
            }
            let Some(title) = capture_title(&capture_text.get_untracked(), &text.get_untracked())
            else {
                return;
            };
            if let Some(timer) = linger_timer.borrow_mut().take() {
                timer.cancel();
            }
            // Record current task IDs so we can find the new one after capture
            let old_ids: std::collections::HashSet<_> =
                snapshot.with_untracked(|s| s.tasks.iter().map(|t| t.id.clone()).collect());
            // The box empties now, so whatever is typed while the task is on its way
            // is the next task, not wiped when this one lands.
            sending.set(true);
            set_text.set(String::new());
            capture_text.set(String::new());
            wasm_bindgen_futures::spawn_local(async move {
                let draft = TaskDraft {
                    title: title.clone(),
                    duration_min: 30,
                    when: When::Anytime,
                    label: Label::Other,
                    priority: Priority::Medium,
                    source: Source::Text,
                };
                match api::create_task(draft).await {
                    Ok(new_snapshot) => {
                        // Find the new task ID (the one not in old_ids)
                        let new_id: Option<TaskId> = new_snapshot
                            .tasks
                            .iter()
                            .find(|t| !old_ids.contains(&t.id))
                            .map(|t| t.id.clone());
                        if let Some(id) = new_id {
                            let mut queue = pen_queue.get_untracked();
                            queue.push(id);
                            pen_queue.set(queue);
                        }
                        set_snapshot.set(new_snapshot);
                    }
                    Err(error) => {
                        // Never lose what was said: it goes back in the box when the
                        // box is still empty, and the note names it either way.
                        if text.get_untracked().trim().is_empty() {
                            set_text.set(title.clone());
                        }
                        set_error.set(Some(capture_note(&title, &error)));
                    }
                }
                sending.set(false);
            });
        }
    };

    view! {
        <form node_ref=form_ref class="capture" on:submit=handle_submit>
            <input
                node_ref=input_ref
                prop:value=text
                on:input=handle_input
                placeholder=WHISPER_PROMPTS[0]
                aria-label="Ramble"
            />
            // The nest: clicking it submits the form (replaces the Add button)
            <button
                type="submit"
                class="nest"
                aria-label="Submit task"
            >
                // Rambo sits here as a sibling element; clicking the nest area around him submits
            </button>
            <Rambo
                state
                set_state
                talking
                set_talking
                snapshot
                set_snapshot
                set_error
                view_settings
                brain_speech
                brain_pose
                set_brain_event
                pen_pos
                capture_text
                form_ref
                pen=Signal::derive(move || view_settings.get().pen)
                typing
            />
        </form>
    }
}

/// The single Rambo element: fixed at bottom-right, draggable, writes, skitters.
///
/// This replaces the old three-element setup (mascot-home button, away .mascot span,
/// and PenRig overlay). One `<button>`, one state machine, one CSS block.
#[component]
fn Rambo(
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
    pen_pos: RwSignal<(f64, f64)>,
    /// Transcript buffer: filled while listening, sent as capture on release
    capture_text: RwSignal<String>,
    /// Capture form: Rambo calls requestSubmit on release
    form_ref: NodeRef<leptos::html::Form>,
    /// The tool he holds: it decides which drawing of each pose he wears
    pen: Signal<settings::Pen>,
    /// True while you are typing, so he peeks over at the box
    typing: ReadSignal<bool>,
) -> impl IntoView {
    let start_return_with = move |hard_drop: bool| {
        if mascot::prefers_reduced_motion() {
            set_state.set(mascot::State::Home);
            return;
        }
        if hard_drop {
            // Splat → Recover → Skitter → Home
            set_state.set(mascot::transition(
                &state.get_untracked(),
                mascot::Event::DropHard,
            ));
            let set_state = set_state;
            Timeout::new(mascot::SPLAT_MS, move || {
                set_state.set(mascot::State::Recovering);
                let set_state = set_state;
                Timeout::new(mascot::RECOVER_MS, move || {
                    set_state.set(mascot::State::Skittering);
                    let set_state = set_state;
                    Timeout::new(mascot::SKITTER_MS, move || {
                        set_state.set(mascot::State::Home);
                    })
                    .forget();
                })
                .forget();
            })
            .forget();
        } else {
            // Recover → Skitter → Home
            set_state.set(mascot::transition(
                &state.get_untracked(),
                mascot::Event::DropEmpty,
            ));
            let set_state = set_state;
            Timeout::new(mascot::RECOVER_MS, move || {
                set_state.set(mascot::State::Skittering);
                let set_state = set_state;
                Timeout::new(mascot::SKITTER_MS, move || {
                    set_state.set(mascot::State::Home);
                })
                .forget();
            })
            .forget();
        }
    };
    let start_return = move || {
        start_return_with(false);
    };

    let rambo_ref = NodeRef::<leptos::html::Button>::new();

    // Track whether pointer moved (to distinguish Release from Drop)
    let moved = std::rc::Rc::new(std::cell::RefCell::new(false));
    let moved_down = moved.clone();
    let moved_move = moved.clone();
    let moved_for_up = moved.clone();

    // Grab state: track pointer position, pen offset, and timestamp at grab so drag
    // follows the pointer exactly and we can compute drop velocity for hard-drop detection.
    // (gx, gy, ox, oy, timestamp_ms)
    let grab_state = std::rc::Rc::new(std::cell::RefCell::new(None::<(f64, f64, f64, f64, u64)>));
    let grab_state_down = grab_state.clone();
    let grab_state_move = grab_state.clone();
    let grab_state_up = grab_state.clone();

    // Speech session for grab-to-listen: transcript goes into capture_text so
    // the capture box can submit it when the user releases Rambo.
    let speech_session = std::rc::Rc::new(std::cell::RefCell::new(None::<speech::Session>));
    let transcript = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let start_speech = {
        let ss = speech_session.clone();
        let transcript = transcript.clone();
        let capture = capture_text;
        move || {
            if !speech::supported() {
                // One calm note, and Rambo keeps the pose the grab already gave
                // him; the transcript simply stays whatever was typed.
                set_error.set(Some(speech::UNAVAILABLE_NOTE.to_owned()));
                return;
            }
            set_error.set(None);
            transcript.borrow_mut().clear();
            let result = speech::Session::start(
                transcript.clone(),
                move |text| capture.set(text),
                move |value| set_talking.set(value),
                move |value| set_error.set(value),
            );
            match result {
                Ok(session) => {
                    *ss.borrow_mut() = Some(session);
                    set_talking.set(true);
                }
                Err(message) => set_error.set(Some(message)),
            }
        }
    };
    let stop_speech_and_send = {
        let ss = speech_session.clone();
        let capture = capture_text;
        let form = form_ref;
        move || {
            if let Some(session) = ss.borrow().as_ref() {
                session.stop();
            }
            set_talking.set(false);
            // Send the transcript as a capture and submit the form
            let text = transcript.borrow().clone();
            if !text.is_empty() {
                capture.set(text.clone());
                // Submit the capture form so the transcript creates a task
                if let Some(form_el) = form.get() {
                    let _ = form_el.request_submit();
                }
            }
        }
    };

    // Clone closures for multiple event handlers
    let start_speech_down = start_speech.clone();
    let start_speech_key = start_speech.clone();
    let stop_speech_up = stop_speech_and_send.clone();
    let stop_speech_cancel = stop_speech_and_send.clone();
    let stop_speech_key = stop_speech_and_send.clone();

    view! {
        <button
            node_ref=rambo_ref
            class=move || mascot::rambo_class(&state.get(), &brain_pose.get())
            type="button"
            tabindex="0"
            aria-label="Grab Rambo to listen"
            // Which pen he writes with: the gear menu sets it, remembered per device.
            data-pen=move || view_settings.get().pen.slug()
            style:--pen-x=move || format!("{:.1}px", pen_pos.get().0)
            style:--pen-y=move || format!("{:.1}px", pen_pos.get().1)
            on:pointerdown=move |ev| {
                ev.prevent_default();
                if mascot::pen_busy(&state.get_untracked()) {
                    return;
                }
                set_brain_event.set(Some(rambl_brain::Event::Poked));
                *moved_down.borrow_mut() = false;
                // Save grab state: pointer position, pen offset, and timestamp at grab
                let gx = ev.client_x() as f64;
                let gy = ev.client_y() as f64;
                let (ox, oy) = pen_pos.get_untracked();
                let t = js_sys::Date::now() as u64;
                *grab_state_down.borrow_mut() = Some((gx, gy, ox, oy, t));
                set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Grab));
                set_talking.set(true);
                start_speech_down();
                if let Some(el) = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                    let _ = el.set_pointer_capture(ev.pointer_id());
                }
            }
            on:pointermove=move |ev| {
                if matches!(state.get_untracked(), mascot::State::Dragging) {
                    *moved_move.borrow_mut() = true;
                    if let Some((gx, gy, ox, oy, _t)) = *grab_state_move.borrow() {
                        let dx = ev.client_x() as f64 - gx;
                        let dy = ev.client_y() as f64 - gy;
                        pen_pos.set((ox + dx, oy + dy));
                    }
                }
            }
            on:pointerup=move |ev: leptos::ev::PointerEvent| {
                stop_speech_up();
                if !matches!(state.get_untracked(), mascot::State::Dragging) {
                    return;
                }
                let did_move = *moved_for_up.borrow();
                if !did_move {
                    // Release without moving: send transcript and go home
                    *grab_state_up.borrow_mut() = None;
                    set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Release));
                    return;
                }
                // Update pen position one last time using delta from grab; also compute
                // drop velocity to detect hard drops (fast release → splat).
                let hard_drop = if let Some((gx, gy, ox, oy, grab_time)) = *grab_state_up.borrow() {
                    let dx = ev.client_x() as f64 - gx;
                    let dy = ev.client_y() as f64 - gy;
                    pen_pos.set((ox + dx, oy + dy));
                    let elapsed = (js_sys::Date::now() as u64).saturating_sub(grab_time) as f64;
                    if elapsed > 0.0 {
                        let distance = (dx * dx + dy * dy).sqrt();
                        let velocity = distance / elapsed;
                        velocity >= mascot::HARD_DROP_VELOCITY
                    } else {
                        false
                    }
                } else {
                    false
                };
                *grab_state_up.borrow_mut() = None;
                let client_x = ev.client_x() as f64;
                let client_y = ev.client_y() as f64;

                // Check if dropped on a task row
                let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                    set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::DropEmpty));
                    start_return_with(hard_drop);
                    return;
                };
                let Some(el) = doc.element_from_point(client_x as f32, client_y as f32) else {
                    set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::DropEmpty));
                    start_return_with(hard_drop);
                    return;
                };
                let Some(li) = el.closest("li[data-id]").ok().flatten() else {
                    set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::DropEmpty));
                    start_return_with(hard_drop);
                    return;
                };
                {
                    let task_id = li.get_attribute("data-id").unwrap_or_default();
                    let task_title = li.get_attribute("data-title").unwrap_or_default();
                    set_state.set(mascot::transition(
                        &state.get_untracked(),
                        mascot::Event::DropTask { task_id, title: task_title },
                    ));
                }
            }
            on:pointercancel=move |_| {
                stop_speech_cancel();
                if matches!(state.get_untracked(), mascot::State::Dragging) {
                    set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::DropEmpty));
                    start_return();
                }
            }
            on:keydown=move |ev| {
                if ev.key() == " " || ev.key() == "Enter" {
                    ev.prevent_default();
                    if talking.get_untracked() {
                        stop_speech_key();
                        set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Release));
                    } else {
                        set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Grab));
                        set_talking.set(true);
                        start_speech_key();
                    }
                }
            }
        >
            <img
                src=move || mascot::art_for(pen.get(), &state.get(), &brain_pose.get(), typing.get())
                alt="Rambo"
            />
            // The mood's little picture, when the mood has one and he is home.
            {move || mascot::fx_for(&state.get(), &brain_pose.get(), typing.get()).map(|fx| view! {
                <img class="rambo-fx" src=fx.art() alt="" aria-hidden="true" />
            })}
        </button>
        <span class="mascot-status" aria-live="polite">
            {move || brain_speech.get().map(|line| view! { <span class="brain-line">{line}</span> })}
            {move || match state.get() {
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
                            set_state.set(mascot::State::Home);
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
                        }>"×"</button>
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

/// A deadline's clock time as it reads in its own zone, for the edit dialog. Unlike
/// `local_input_value` this does not depend on the browser's zone, which is exactly
/// the point of `due_tz` (docs/ARCHITECTURE.md 4.1): a deadline set as Friday 17:00
/// New York reads Friday 17:00 no matter where the browser is.
fn due_input_value(time: DateTime<Utc>, tz: chrono_tz::Tz) -> String {
    let local = time.with_timezone(&tz);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        local.year(),
        local.month(),
        local.day(),
        local.hour(),
        local.minute()
    )
}

/// How the edit dialog shows a deadline: in the zone it was set in when the task
/// has one, else in the browser's zone (tasks created before `due_tz` existed).
fn due_display(task: &todo_core::Task, time: DateTime<Utc>) -> String {
    match task.due_tz.as_deref().and_then(|name| name.parse().ok()) {
        Some(tz) => due_input_value(time, tz),
        None => local_input_value(time),
    }
}

/// Parse a deadline typed into the edit dialog as a time in `tz`, so a deadline
/// shown in its original zone saves as the same local time even after a move.
/// Fall-back makes a wall clock ambiguous (the hour repeats); `.earliest()` picks
/// the first of the two, where `.single()` used to give up silently. A time in a
/// spring-forward gap genuinely does not exist and still fails.
fn parse_due_input(value: &str, tz: chrono_tz::Tz) -> Option<DateTime<Utc>> {
    let naive = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").ok()?;
    tz.from_local_datetime(&naive)
        .earliest()
        .map(|local| local.with_timezone(&Utc))
}

/// The deadline the edited task already has, parsed in the zone that set it (so the
/// shown value round-trips), or in the browser's zone for a task without `due_tz`.
fn parse_edited_due(value: &str, task: &todo_core::Task) -> Option<When> {
    let due = match task.due_tz.as_deref().and_then(|name| name.parse().ok()) {
        Some(tz) => parse_due_input(value, tz),
        None => parse_local_input(value),
    };
    due.map(When::DueBy)
}

/// The `when` a save sends, or the message to show when the typed clock time names no
/// instant. A wall-clock time inside a spring-forward gap — 02:30 on the day the clocks
/// jump from 02:00 to 03:00 — never happened, so both parsers give up on it; the dialog
/// has to say that and stay open, because a Save that quietly does nothing looks broken.
/// `deadline` is the edited task, which carries the zone its due time is read in.
fn typed_when(
    kind: EditWhen,
    value: &str,
    deadline: Option<&todo_core::Task>,
) -> Result<When, &'static str> {
    let when = match kind {
        EditWhen::Anytime => Some(When::Anytime),
        EditWhen::DueBy => deadline.and_then(|task| parse_edited_due(value, task)),
        EditWhen::At => parse_local_input(value).map(When::At),
    };
    when.ok_or("That time does not exist on this day (clocks change)")
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
    mascot_state: ReadSignal<mascot::State>,
    set_mascot_state: WriteSignal<mascot::State>,
    pen_queue: RwSignal<Vec<TaskId>>,
    pen_pos: RwSignal<(f64, f64)>,
) -> impl IntoView {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = pen_pos;
    let (dragged_task, set_dragged_task) = signal::<Option<TaskId>>(None);
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
                        set_state.set(mascot::transition(&state, mascot::Event::WriteRow));
                        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                            if let Some(row) = doc
                                .query_selector(&format!("li[data-id=\"{}\"]", task_id.0))
                                .ok()
                                .flatten()
                            {
                                {
                                    let arr = Array::new();
                                    arr.push(&"pen-pending".into());
                                    row.class_list().add(&arr).ok();
                                }
                                let rect = row.get_bounding_client_rect();
                                let center = pen::writing_pen_center(
                                    rect.left() + 40.0,
                                    rect.top() + rect.height() / 2.0,
                                );
                                // Convert to offset from Rambo's home position
                                if let Some(rambo) = doc.query_selector(".rambo").ok().flatten() {
                                    let rrect = rambo.get_bounding_client_rect();
                                    let hx = rrect.left() + rrect.width() / 2.0;
                                    let hy = rrect.top() + rrect.height() / 2.0;
                                    pen_pos.set((center.0 - hx, center.1 - hy));
                                }
                            }
                        }
                        gloo_timers::future::TimeoutFuture::new(pen::FLY_MS).await;
                        set_state.set(mascot::State::WritingRow);
                        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                            if let Some(row) = doc
                                .query_selector(&format!("li[data-id=\"{}\"]", task_id.0))
                                .ok()
                                .flatten()
                            {
                                {
                                    let remove_arr = Array::new();
                                    remove_arr.push(&"pen-pending".into());
                                    row.class_list().remove(&remove_arr).ok();
                                }
                                {
                                    let add_arr = Array::new();
                                    add_arr.push(&"pen-writing".into());
                                    row.class_list().add(&add_arr).ok();
                                }
                            }
                        }
                        gloo_timers::future::TimeoutFuture::new(pen::WRITE_MS).await;
                        // Remove pen-writing class so the title becomes fully visible
                        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                            if let Some(row) = doc
                                .query_selector(&format!("li[data-id=\"{}\"]", task_id.0))
                                .ok()
                                .flatten()
                            {
                                let remove_arr = Array::new();
                                remove_arr.push(&"pen-writing".into());
                                row.class_list().remove(&remove_arr).ok();
                            }
                        }
                        // Fly home after this row
                        set_state.set(mascot::transition(
                            &mascot::State::WritingRow,
                            mascot::Event::FlyHome,
                        ));
                        gloo_timers::future::TimeoutFuture::new(pen::FLY_MS).await;
                        set_state.set(mascot::State::Home);
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
                        let drop_task_title = task_title.clone();
                        let render_task_id = task_id.0.clone();
                        let render_task_id_attr = render_task_id.clone();
                        let render_task_title = task_title.clone();

                        let toggle_done = {
                            let task_id_clone = task_id.clone();
                            move |_| {
                                let task_id_clone = task_id_clone.clone();
                                let sink = EditSink { set_snapshot, set_error, view_settings };
                                let Some(action) =
                                    snapshot.with_untracked(|s| tick_action(s, &task_id_clone.0))
                                else {
                                    return;
                                };
                                wasm_bindgen_futures::spawn_local(async move {
                                    // A task edit is refused with a reload-and-retry note
                                    // (patch_and_show); a series write has nothing to reload,
                                    // so its refusals speak for themselves.
                                    match action {
                                        TickAction::Task(body) => {
                                            patch_and_show(&task_id_clone, body, sink).await;
                                        }
                                        TickAction::MarkDone { series, date } => {
                                            match api::mark_occurrence(&series, &date.to_string(), "done").await {
                                                Ok(snapshot) => {
                                                    sink.set_error.set(None);
                                                    sink.set_snapshot.set(snapshot);
                                                }
                                                Err(e) => sink.set_error.set(Some(series_note(&e))),
                                            }
                                        }
                                        TickAction::ClearDone { series, date } => {
                                            match api::clear_occurrence(&series, &date.to_string()).await {
                                                Ok(snapshot) => {
                                                    sink.set_error.set(None);
                                                    sink.set_snapshot.set(snapshot);
                                                }
                                                Err(e) => sink.set_error.set(Some(series_note(&e))),
                                            }
                                        }
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

                                // Handle task reorder drop. A drop that involves a
                                // series occurrence is a no-op here, not a reorder
                                // the server would refuse (reorder_ids explains).
                                if let Some(dragged_id) = dragged_task.get()
                                    && dragged_id != task_id_clone
                                    && let Some(new_order) =
                                        reorder_ids(&snapshot.get_untracked(), &dragged_id, &task_id_clone)
                                {
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
                        class="task-row"
                        data-id={render_task_id_attr}
                        data-title={render_task_title.clone()}
                        on:drop=handle_drop
                        on:dragover=handle_drag_over
                        draggable="true"
                        on:dragstart=handle_drag_start
                        on:dragend=handle_drag_end
                        // The row answers its own touch gestures and stops them here, so nothing
                        // above the row sees the same touch. This does NOT stop the page
                        // flip: that listens on pointer events, which are a separate
                        // dispatch. What keeps swipe right (delete) and swipe left (turn
                        // the page) apart is the direction rule in flip_target.
                        on:touchstart=move |ev: leptos::ev::TouchEvent| ev.stop_propagation()
                        on:touchend=move |ev: leptos::ev::TouchEvent| ev.stop_propagation()
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
                                            set_edit_datetime.set(due_display(&task, time));
                                        }
                                        When::At(time) => {
                                            set_edit_when.set(EditWhen::At);
                                            set_edit_datetime.set(local_input_value(time));
                                        }
                                    }
                                    // A series row edits the series: its fields come from
                                    // the rule, not from the one occurrence row clicked.
                                    match snapshot
                                        .with_untracked(|s| series_of(s, &task.id.0).cloned())
                                    {
                                        Some(series) => {
                                            edit_weekdays.set(series.rule.weekdays.clone());
                                            edit_until.set(
                                                series.rule.until.map(|day| day.to_string()).unwrap_or_default(),
                                            );
                                            edit_time.set(
                                                series
                                                    .time_of_day
                                                    .map(|time| time.format("%H:%M").to_string())
                                                    .unwrap_or_default(),
                                            );
                                        }
                                        None => {
                                            edit_weekdays.set(Vec::new());
                                            edit_until.set(String::new());
                                            edit_time.set(String::new());
                                        }
                                    }
                                    set_editing_task.set(Some(task.id.clone()));
                                    set_edit_time_error.set(None);
                                }
                            }
                        >
                            <span class="pen-title-wrap">
                                <strong class="pen-title">{render_task_title.clone()}</strong>
                                {repeat_mark}
                            </span> " — " {task.duration_min} " min"
                            {move || {
                                let time = task_time(&snapshot.get(), &render_task_id);
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
                                                            // one-off row it grew out of.
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
                                            let sink = EditSink { set_snapshot, set_error, view_settings };
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
    fn whisper_rotation_visits_every_line_then_wraps() {
        let len = WHISPER_PROMPTS.len();
        let mut current = 0;
        let mut seen = vec![current];
        for _ in 0..len {
            current = next_whisper(current, len);
            seen.push(current);
        }
        assert_eq!(seen, (0..=len).map(|step| step % len).collect::<Vec<_>>());
    }

    #[test]
    fn whisper_rotation_is_total_even_without_lines() {
        assert_eq!(next_whisper(3, 0), 0);
    }

    #[test]
    fn whisper_prompts_stay_short_and_few() {
        assert!(
            (4..=8).contains(&WHISPER_PROMPTS.len()),
            "the work order asks for 4-8 lines"
        );
        assert!(
            WHISPER_PROMPTS
                .iter()
                .all(|prompt| !prompt.trim().is_empty() && prompt.chars().count() <= 40)
        );
    }

    #[test]
    fn reduced_motion_stops_the_whisper() {
        assert!(!whisper_rotates(true));
        assert!(whisper_rotates(false));
    }

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

    /// Reorder sends the server exactly the stored tasks, so the occurrence rows
    /// a snapshot carries are kept out of the list — and a drop that starts or
    /// lands on one is not a reorder at all.
    #[test]
    fn a_reorder_lists_stored_tasks_once_and_never_an_occurrence() {
        let mut snapshot = api::fixture_snapshot();
        // An expanded day of a repeating series sits between the stored tasks.
        let mut occurrence = snapshot.tasks[0].clone();
        occurrence.id = TaskId("standup:2026-10-14".into());
        occurrence.source = Source::Series;
        snapshot.tasks.insert(1, occurrence);

        let first = snapshot.tasks[0].id.clone();
        let third = snapshot.tasks[2].id.clone();
        let order = reorder_ids(&snapshot, &third, &first).expect("stored slip onto stored slip");
        assert_eq!(
            order.first(),
            Some(&third),
            "the slip moves before its target"
        );
        assert_eq!(order.len(), 2, "only stored tasks, exactly once");
        assert!(!order.iter().any(|id| id.0.contains(':')));

        let occ = snapshot.tasks[1].id.clone();
        assert_eq!(
            reorder_ids(&snapshot, &occ, &first),
            None,
            "dragged occurrence"
        );
        assert_eq!(
            reorder_ids(&snapshot, &first, &occ),
            None,
            "dropped on occurrence"
        );
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

    #[test]
    fn a_capture_sends_the_fresh_transcript_or_the_box_and_never_blank() {
        // Rambo's release: the transcript is newer than the box.
        assert_eq!(
            capture_title(" buy milk ", "old text").as_deref(),
            Some("buy milk")
        );
        // Typed: typing cleared the transcript.
        assert_eq!(capture_title("", "call mum ").as_deref(), Some("call mum"));
        assert_eq!(capture_title("", "   "), None, "blank adds nothing");
        assert_eq!(capture_title(" ", ""), None);
    }

    #[test]
    fn a_capture_that_failed_is_named_in_the_note() {
        let offline = api::ApiError::Network("offline".into());
        assert!(capture_note("buy milk", &offline).contains("\"buy milk\""));
        assert!(capture_note("buy milk", &offline).contains("Could not reach the server"));
        let refused = api::ApiError::Server(todo_core::ApiError {
            code: todo_core::ErrorCode::Forbidden,
            message: "this account is paused".into(),
        });
        assert_eq!(
            capture_note("buy milk", &refused),
            "\"buy milk\" was not added: this account is paused"
        );
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

    /// docs/ARCHITECTURE.md 4.1: a deadline is edited in the zone it was set in.
    /// The value shown and the value parsed on save must round-trip in that zone,
    /// on both sides of the November DST change.
    #[test]
    fn a_due_deadline_round_trips_in_its_own_zone_across_dst() {
        let ny = chrono_tz::America::New_York;
        // Friday 2026-10-30 23:59 New York is still EDT (UTC-4).
        let before = Utc.with_ymd_and_hms(2026, 10, 31, 3, 59, 0).unwrap();
        assert_eq!(due_input_value(before, ny), "2026-10-30T23:59");
        assert_eq!(parse_due_input("2026-10-30T23:59", ny), Some(before));

        // Monday 2026-11-02 23:59 New York is EST (UTC-5): a wall clock set after
        // the fall-back is 04:59:00Z, not 03:59:00Z.
        let after = Utc.with_ymd_and_hms(2026, 11, 3, 4, 59, 0).unwrap();
        assert_eq!(due_input_value(after, ny), "2026-11-02T23:59");
        assert_eq!(parse_due_input("2026-11-02T23:59", ny), Some(after));

        // The fall-back itself: 01:30 New York on 2026-11-01 happens twice — 01:30
        // EDT and again 01:30 EST after the clocks fall back. `.earliest()` picks
        // the first one; `.single()` used to reject the whole time silently.
        let fold_earliest = Utc.with_ymd_and_hms(2026, 11, 1, 5, 30, 0).unwrap();
        assert_eq!(parse_due_input("2026-11-01T01:30", ny), Some(fold_earliest));

        // A spring-forward gap is not a time at all: 02:30 on 2026-03-08 New York
        // (the clocks jump 02:00 to 03:00) must not invent an instant.
        assert_eq!(parse_due_input("2026-03-08T02:30", ny), None);
    }

    /// A deadline set as 17:00 New York reads 17:00 New York even when the browser
    /// lives somewhere else — that is the whole point of `due_tz`.
    #[test]
    fn a_due_task_is_shown_in_the_zone_it_was_set_in() {
        let deadline = Utc.with_ymd_and_hms(2026, 10, 16, 21, 0, 0).unwrap();
        let mut task = api::fixture_snapshot().tasks[0].clone();
        task.when = When::DueBy(deadline);
        task.due_tz = Some("America/New_York".into());

        assert_eq!(due_display(&task, deadline), "2026-10-16T17:00");
        // Saving without touching the field keeps the same instant: 17:00 in New
        // York is not 17:00 in Los Angeles.
        assert_eq!(
            parse_edited_due("2026-10-16T17:00", &task),
            Some(When::DueBy(deadline))
        );
        // ... and the same deadline reads 14:00 only when shown in Los Angeles.
        assert_eq!(
            due_input_value(deadline, chrono_tz::America::Los_Angeles),
            "2026-10-16T14:00"
        );
    }

    /// A time inside a spring-forward gap names no instant, so the dialog refuses it
    /// with a message and stays open, rather than a Save that does nothing quietly.
    #[test]
    fn a_time_in_the_spring_forward_gap_is_refused_with_a_message() {
        let mut task = api::fixture_snapshot().tasks[0].clone();
        task.when = When::DueBy(Utc.with_ymd_and_hms(2027, 3, 13, 7, 0, 0).unwrap());
        task.due_tz = Some("America/New_York".into());

        // 02:30 on 2027-03-14 New York: the clocks jump 02:00 to 03:00 that day.
        assert_eq!(
            typed_when(EditWhen::DueBy, "2027-03-14T02:30", Some(&task)),
            Err("That time does not exist on this day (clocks change)")
        );
        // An empty field is no instant either, and gets the same refusal.
        assert!(typed_when(EditWhen::DueBy, "", Some(&task)).is_err());

        // An ordinary time in that zone saves, and "Anytime" never looks at the field.
        assert_eq!(
            typed_when(EditWhen::DueBy, "2027-03-14T09:30", Some(&task)),
            Ok(When::DueBy(
                Utc.with_ymd_and_hms(2027, 3, 14, 13, 30, 0).unwrap()
            ))
        );
        assert_eq!(typed_when(EditWhen::Anytime, "", None), Ok(When::Anytime));
    }

    /// A task from before `due_tz` existed has no zone: the edit dialog falls back
    /// to the browser's own reading (UTC on the host test target), so old behaviour
    /// is untouched.
    #[test]
    fn a_task_without_due_tz_falls_back_to_the_browser_zone() {
        let deadline = Utc.with_ymd_and_hms(2026, 10, 16, 21, 0, 0).unwrap();
        let mut task = api::fixture_snapshot().tasks[0].clone();
        task.when = When::DueBy(deadline);
        task.due_tz = None;
        assert_eq!(due_display(&task, deadline), "2026-10-16T21:00");
        assert_eq!(
            parse_edited_due("2026-10-16T21:00", &task),
            Some(When::DueBy(deadline))
        );
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

    #[test]
    fn the_day_buttons_sort_monday_first_and_toggle() {
        assert_eq!(
            sorted_weekdays(&[Weekday::Sun, Weekday::Mon, Weekday::Wed, Weekday::Mon]),
            vec![Weekday::Mon, Weekday::Wed, Weekday::Sun]
        );
        let picked = toggled_day(Vec::new(), Weekday::Thu);
        assert_eq!(
            toggled_day(picked.clone(), Weekday::Thu),
            Vec::<Weekday>::new(),
            "picking a picked day unpicks it"
        );
        assert_eq!(
            toggled_day(picked, Weekday::Fri),
            vec![Weekday::Thu, Weekday::Fri]
        );
    }

    #[test]
    fn the_dialog_builds_a_series_body_from_its_fields() {
        let body = series_from_dialog(
            "  Gym  ",
            Some(30),
            &[Weekday::Thu, Weekday::Mon],
            "2026-12-15",
            "07:00",
        )
        .expect("every field names a rule");
        assert_eq!(body.title, "Gym");
        assert_eq!(body.duration_min, 30);
        assert_eq!(body.weekdays, vec![Weekday::Mon, Weekday::Thu]);
        assert_eq!(body.until.as_deref(), Some("2026-12-15"));
        assert_eq!(body.time_of_day.as_deref(), Some("07:00"));
        // An empty end date or time is simply absent on the wire.
        let floating = series_from_dialog("Gym", Some(30), &[Weekday::Mon], "", "")
            .expect("no end and no time is a valid rule");
        assert_eq!(floating.until, None);
        assert_eq!(floating.time_of_day, None);
        // No title, no length, or no day picked: nothing to save.
        assert!(series_from_dialog("  ", Some(30), &[Weekday::Mon], "", "").is_none());
        assert!(series_from_dialog("Gym", None, &[Weekday::Mon], "", "").is_none());
        assert!(series_from_dialog("Gym", Some(30), &[], "", "").is_none());
    }

    #[test]
    fn a_series_edit_sends_only_the_fields_the_dialog_holds() {
        let patch = series_patch_from_dialog("Gym", Some(45), &[Weekday::Fri], "", "08:30")
            .expect("a usable edit");
        assert_eq!(patch.title.as_deref(), Some("Gym"));
        assert_eq!(patch.duration_min, Some(45));
        assert_eq!(patch.weekdays, Some(vec![Weekday::Fri]));
        // An emptied end date cannot clear the series' one (7.1: null reads absent).
        assert_eq!(patch.until, None);
        assert_eq!(patch.time_of_day.as_deref(), Some("08:30"));
        assert!(series_patch_from_dialog("Gym", Some(45), &[], "", "").is_none());
    }

    #[test]
    fn a_refused_series_write_says_the_server_s_word() {
        let refused = api::ApiError::Server(todo_core::ApiError {
            code: todo_core::ErrorCode::BadRequest,
            message: "not an occurrence".into(),
        });
        assert_eq!(series_note(&refused), "not an occurrence");
        assert!(series_note(&api::ApiError::Network("offline".into())).contains("network error"));
    }
}

#[cfg(test)]
mod pane_tests {
    use super::{curl_for, pane_at};

    #[test]
    fn a_phone_swipe_maps_to_the_nearest_pane() {
        assert_eq!(pane_at(0.0, 400.0), 0);
        assert_eq!(pane_at(180.0, 400.0), 0);
        assert_eq!(pane_at(220.0, 400.0), 1);
        assert_eq!(pane_at(400.0, 400.0), 1);
        assert_eq!(pane_at(900.0, 400.0), 1, "never past the last pane");
        assert_eq!(pane_at(100.0, 0.0), 0, "no width yet, so the first pane");
    }

    #[test]
    fn a_turn_leans_the_curl_towards_the_page_it_is_going_to() {
        // The scroll is the turn now: whatever direction the finger went, the
        // curl leans the way the pages are travelling.
        assert_eq!(curl_for(0, 1), Some(true), "list → planner");
        assert_eq!(curl_for(1, 0), Some(false), "planner → list");
    }

    #[test]
    fn a_scroll_that_stays_on_its_page_is_not_a_turn() {
        // Vertical scrolling, a rubber-band at the end, and the quiet after a
        // turn has settled all leave the pages where they were: no curl.
        assert_eq!(curl_for(0, 0), None);
        assert_eq!(curl_for(1, 1), None);
    }
}

/// Hook exposed for Chuan's Rambo scribble-out animation before removal.
pub fn on_delete_requested(task_id: todo_core::TaskId) {
    // Placeholder: the row's delete event handler will call the existing
    // delete API; this function is the public hook for pre-delete effects.
    let _ = task_id;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwipeAction {
    Delete,
    SnapBack,
}

pub fn swipe_action(delta_x: f64, threshold: f64) -> SwipeAction {
    if delta_x >= threshold {
        SwipeAction::Delete
    } else {
        SwipeAction::SnapBack
    }
}

#[cfg(test)]
mod swipe_tests {
    use super::*;
    #[test]
    fn swipe_past_threshold_deletes() {
        assert_eq!(swipe_action(80.0, 50.0), SwipeAction::Delete);
    }
    #[test]
    fn short_swipe_snaps_back() {
        assert_eq!(swipe_action(20.0, 50.0), SwipeAction::SnapBack);
    }
    #[test]
    fn exactly_threshold_deletes() {
        assert_eq!(swipe_action(50.0, 50.0), SwipeAction::Delete);
    }
}

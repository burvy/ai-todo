//! The capture line: the box you type or ramble into, and its whispered prompts.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use todo_core::{Snapshot, TaskId};

use crate::{api, mascot, settings};
use rambl_web_core::store::Command;

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

/// What is sent for `title` while Rambo notes a task at the hour `at` (a date phrase
/// Jev reads): the hour first, so the task starts then.
fn noted(at: &NotedHour, title: &str) -> todo_core::TaskDraft {
    todo_core::TaskDraft {
        title: title.trim().to_owned(),
        duration_min: todo_core::tuning::TASK_DEFAULT_DURATION_MIN,
        when: todo_core::When::At(at.at),
        label: todo_core::Label::Other,
        priority: todo_core::Priority::Low,
        source: todo_core::Source::Text,
    }
}

/// The tasks in `after` that were not in `before`: what a create just added.
fn added(before: &Snapshot, after: &Snapshot) -> Vec<TaskId> {
    after
        .tasks
        .iter()
        .filter(|task| before.tasks.iter().all(|old| old.id != task.id))
        .map(|task| task.id.clone())
        .collect()
}

/// The empty planner hour Rambo was dropped on: its date phrase, which the capture box
/// shows, and the instant a task noted there starts.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NotedHour {
    pub(crate) phrase: String,
    pub(crate) at: chrono::DateTime<chrono::Utc>,
}

/// How much of the bottom of the layout a phone's keyboard covers, in px: the layout's
/// height below the visible part of the page (`visible_top` + `visible_height`, the
/// visual viewport). Never negative: a page scrolled past its end hides nothing. A
/// pinch-zoomed page (`scale` above 1) shows less of itself without any keyboard, so
/// it covers nothing either: the keyboard never zooms the page.
fn keyboard_inset(layout_height: f64, visible_top: f64, visible_height: f64, scale: f64) -> f64 {
    match (scale - 1.0).abs() < 0.01 {
        true => (layout_height - visible_top - visible_height).max(0.0),
        false => 0.0,
    }
}

/// Keeps `--keyboard-inset` on the page's root the height a phone's keyboard covers, so
/// the capture bar (and the nest, and Rambo in it) sits on top of the keyboard rather
/// than under it (Interview 1 item 8). Rambo follows his nest on the same events.
fn ride_the_keyboard() {
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(visible) = window.visual_viewport() else {
        return;
    };
    let follow = {
        let visible = visible.clone();
        move || {
            let layout = window
                .inner_height()
                .ok()
                .and_then(|height| height.as_f64())
                .unwrap_or_default();
            let inset = keyboard_inset(
                layout,
                visible.offset_top(),
                visible.height(),
                visible.scale(),
            );
            if let Some(root) = window
                .document()
                .and_then(|doc| doc.document_element())
                .and_then(|root| root.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = root
                    .style()
                    .set_property("--keyboard-inset", &format!("{inset:.0}px"));
            }
        }
    };
    let listener = wasm_bindgen::closure::Closure::<dyn Fn()>::new(follow);
    for event in ["resize", "scroll"] {
        let _ = visible.add_event_listener_with_callback(event, listener.as_ref().unchecked_ref());
    }
    // The capture bar lives as long as the page, and so does its listener.
    listener.forget();
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
pub(crate) fn CaptureBox(
    form_ref: NodeRef<leptos::html::Form>,
    nest_ref: NodeRef<leptos::html::Button>,
    snapshot: ReadSignal<Snapshot>,
    set_snapshot: WriteSignal<Snapshot>,
    state: ReadSignal<mascot::State>,
    set_state: WriteSignal<mascot::State>,
    view_settings: ReadSignal<settings::Settings>,
    pen_pos: RwSignal<(f64, f64)>,
    capture_text: RwSignal<String>,
    /// The hour Rambo was dropped on, as a date phrase: what is sent starts then.
    noting_at: RwSignal<Option<NotedHour>>,
    pen_queue: RwSignal<Vec<TaskId>>,
    set_typing: WriteSignal<bool>,
    set_error: WriteSignal<Option<String>>,
    /// Set when you tap the +: the list opens its sheet on a new task.
    new_task: RwSignal<bool>,
) -> impl IntoView {
    let (text, set_text) = signal(String::new());

    // Watch for speech transcript from Rambo: when capture_text changes, fill the input
    Effect::new(move |_| {
        let transcript = capture_text.get();
        if !transcript.is_empty() {
            set_text.set(transcript);
        }
    });
    let input_ref = NodeRef::<leptos::html::Input>::new();
    // Never mind on the hour he was dropped on: nothing is sent, and the box empties.
    Effect::new(move |was: Option<mascot::State>| {
        let now = state.get();
        if was == Some(mascot::State::Noting) && now != mascot::State::Noting {
            set_text.set(String::new());
            capture_text.set(String::new());
        }
        now
    });
    ride_the_keyboard();

    // Rambo's whisper: the placeholder is the rotating prompt. Read the index
    // before the node ref so a ref that loads later can't drop a swap.
    let whisper = RwSignal::new(0);
    let whisper_fading = RwSignal::new(false);
    Effect::new(move |_| {
        let index = whisper.get();
        let Some(input) = input_ref.get() else {
            return;
        };
        match noting_at.get() {
            Some(at) => input.set_placeholder(&format!("A task for {}", at.phrase.trim())),
            None => input.set_placeholder(WHISPER_PROMPTS[index]),
        }
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
            let timer = Timeout::new(crate::tuning::LINGER_MS, move || {
                set_state.update(|s| {
                    *s = mascot::transition(s, mascot::Event::FlyHome);
                });
                // He sets off for the nest now, not when the flight is over.
                if state.get_untracked() == mascot::State::Flying
                    && let Some(nest) = crate::rambo::nest_box(nest_ref)
                {
                    pen_pos.set(mascot::nest_home(nest));
                }
                let set_state = set_state;
                let timer = Timeout::new(crate::tuning::FLY_MS, move || {
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
            let sel = input_el.selection_start().ok().flatten().unwrap_or(0) as usize;
            // The nib is at the end of the words before the caret, on the paper: measured
            // in the box's own font, past its padding.
            let style =
                web_sys::window().and_then(|window| window.get_computed_style(&el).ok().flatten());
            let px_of = |name: &str| {
                style
                    .as_ref()
                    .and_then(|style| style.get_property_value(name).ok())
                    .and_then(|value| value.trim_end_matches("px").parse::<f64>().ok())
                    .unwrap_or(0.0)
            };
            // The torn ends are the box's borders (theme.css): the words start inside them.
            let inset_left = px_of("padding-left") + px_of("border-left-width");
            let inset_right = px_of("padding-right") + px_of("border-right-width");
            let value = input_el.value();
            let before = crate::pen::before_caret(&value, sel);
            let (advance, baseline) =
                crate::tasks::write::text_on_line(&el, before, rect.top(), rect.height());
            let content = rect.width() - inset_left - inset_right;
            let caret_x = crate::pen::caret_x(
                rect.left(),
                inset_left,
                advance,
                f64::from(input_el.scroll_left()),
                content,
            );
            // His nib rests on the writing line: the baseline of the words.
            let center = crate::pen::writing_pen_center(caret_x, baseline);
            // Rambo is now a fixed layer — pen_pos is absolute page coords
            pen_pos.set(center);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (input_ref, pen_pos);
        }
    };

    // The page scrolls under the pinned scrap as it sticks: while he writes, the nib
    // follows the words, not where they were.
    #[cfg(target_arch = "wasm32")]
    {
        // The capture box lives as long as the page, and so does this listener.
        let _ = window_event_listener(leptos::ev::scroll, move |_| {
            if mascot::pen_busy(&state.get_untracked()) {
                measure_caret();
            }
        });
    }

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
        // Already on his way or writing: the nib goes where the words are now, and the
        // wait before he goes home starts again. No new flight.
        if mascot::pen_busy(&st) {
            measure_caret();
            restart_linger();
            return;
        }
        // Held, or waiting on an hour for the task you type, he stays where he is.
        if matches!(st, mascot::State::Dragging | mascot::State::Noting) {
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
    let store = expect_context::<crate::store::Store>();
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
            // Sent from an hour he was dropped on: the task starts then, and he goes home.
            let hour = noting_at.get_untracked();
            let st = state.get_untracked();
            if st == mascot::State::Noting {
                set_state.set(mascot::transition(&st, mascot::Event::Return));
            }
            if let Some(timer) = linger_timer.borrow_mut().take() {
                timer.cancel();
            }
            // The box empties now, so whatever is typed while the task is on its way
            // is the next task, not wiped when this one lands.
            sending.set(true);
            set_text.set(String::new());
            capture_text.set(String::new());
            // Noted on an hour he was dropped on, it is a task at that hour, whatever its
            // words: it is made as it is, never asked of the judge.
            if let Some(hour) = hour {
                let before = snapshot.get_untracked();
                wasm_bindgen_futures::spawn_local(async move {
                    if store.send(Command::CreateTask(noted(&hour, &title))).await {
                        pen_queue.update(|queue| {
                            queue.extend(added(&before, &snapshot.get_untracked()));
                        });
                    } else if text.get_untracked().trim().is_empty() {
                        // Never lose what was said: it goes back in the box when the box
                        // is still empty; the store's note says why it was not kept.
                        set_text.set(title.clone());
                    }
                    sending.set(false);
                });
                return;
            }
            wasm_bindgen_futures::spawn_local(async move {
                // The capture pipeline (docs/ARCHITECTURE.md 5) splits the ramble into
                // tasks and reads their days, times, labels and priorities.
                match api::capture_text(&title).await {
                    Ok(captured) => {
                        if captured.created.is_empty() {
                            if text.get_untracked().trim().is_empty() {
                                set_text.set(title.clone());
                            }
                            set_error.set(Some(format!("No task found in \"{title}\".")));
                        }
                        if !captured.created.is_empty() {
                            crate::sound::play(crate::sound::Cue::Send);
                        }
                        pen_queue.update(|queue| queue.extend(captured.created));
                        set_snapshot.set(captured.snapshot);
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
            // For a task you would rather write out than ramble: the same sheet an edit uses.
            <button
                type="button"
                class="add-by-hand"
                aria-label="Add a task by hand"
                on:click=move |_| new_task.set(true)
            >"+"</button>
            <input
                node_ref=input_ref
                prop:value=text
                on:input=handle_input
                placeholder=WHISPER_PROMPTS[0]
                aria-label="Ramble"
                spellcheck="false"
            />
            // The nest: clicking it submits the form (replaces the Add button)
            <button
                node_ref=nest_ref
                type="submit"
                class="nest"
                aria-label="Submit task"
            >
                // The back of the dish, behind him. Drawn through <use> so it
                // takes the page's --nest-* tokens and follows the theme. FE-2.
                <svg class="nest-back" viewBox="0 0 170 100" aria-hidden="true">
                    <use href="assets/nest/back.svg#art" />
                </svg>
            </button>
        </form>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyboard_inset_is_what_the_visible_page_leaves_uncovered_at_the_bottom() {
        // No keyboard: the visible page is the whole layout.
        assert_eq!(keyboard_inset(844.0, 0.0, 844.0, 1.0), 0.0);
        // A 336 px keyboard over a 844 px page.
        assert_eq!(keyboard_inset(844.0, 0.0, 508.0, 1.0), 336.0);
        // Scrolled up inside the layout as the keyboard opens (iOS): only what is below
        // the visible part is covered.
        assert_eq!(keyboard_inset(844.0, 120.0, 508.0, 1.0), 216.0);
        // Pinch-zoomed to twice the size, half the page shows, and no keyboard is there.
        assert_eq!(keyboard_inset(844.0, 0.0, 422.0, 2.0), 0.0);
        // Overscrolled, the visible part can reach past the layout's end: nothing is
        // covered, never a negative inset.
        let mut seed = 5_u64;
        for _ in 0..1000 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let layout = 300.0 + (seed >> 40) as f64 % 900.0;
            let top = (seed >> 20) as f64 % 400.0;
            let height = 50.0 + (seed >> 30) as f64 % 1000.0;
            let inset = keyboard_inset(layout, top, height, 1.0);
            assert!(inset >= 0.0);
            assert!(inset <= layout, "never more than the page");
            if top + height >= layout {
                assert_eq!(inset, 0.0);
            }
        }
    }

    #[test]
    fn the_capture_bar_sits_on_the_keyboard() {
        let css = include_str!("../theme.css");
        let rule = css
            .split(
                "
.capture {",
            )
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("theme.css styles .capture");
        assert!(
            rule.contains("bottom: var(--keyboard-inset, 0px)"),
            "{rule}"
        );
    }

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
    fn a_task_noted_on_an_hour_is_made_at_that_hour_as_it_is_worded() {
        use chrono::TimeZone as _;
        let hour = NotedHour {
            phrase: "today 11:15 ".into(),
            at: chrono::Utc
                .with_ymd_and_hms(2026, 10, 15, 15, 15, 0)
                .unwrap(),
        };
        // QA round 3: words that ask for no action are still the task: never judged.
        for title in ["ff2d434 enter test", " call mum ", "the quarterly report"] {
            let draft = noted(&hour, title);
            assert_eq!(draft.title, title.trim());
            assert_eq!(draft.when, todo_core::When::At(hour.at));
            assert_eq!(
                draft.duration_min,
                todo_core::tuning::TASK_DEFAULT_DURATION_MIN
            );
        }
    }

    #[test]
    fn what_a_create_added_is_the_tasks_that_were_not_there_before() {
        let after = api::fixture_snapshot();
        let mut before = after.clone();
        let made = before.tasks.remove(1);
        assert_eq!(added(&before, &after), vec![made.id]);
        assert_eq!(added(&after, &after), Vec::<TaskId>::new());
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
}

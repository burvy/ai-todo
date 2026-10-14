//! The capture line: the box you type or ramble into, and its whispered prompts.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use todo_core::{Label, Priority, Snapshot, Source, TaskDraft, TaskId, When};

use crate::{api, mascot, pen, settings};

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
    pen_queue: RwSignal<Vec<TaskId>>,
    set_typing: WriteSignal<bool>,
    set_error: WriteSignal<Option<String>>,
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
            // Rambo is now a fixed layer — pen_pos is absolute page coords
            pen_pos.set(center);
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
                node_ref=nest_ref
                type="submit"
                class="nest"
                aria-label="Submit task"
            >
            </button>
        </form>
    }
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
}

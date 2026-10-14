//! Rambo the pen: dragging, flying, landing on a row, and listening.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use todo_core::{Snapshot, TaskId};
use wasm_bindgen::JsCast;

use crate::edits::{EditSink, next_priority_body, patch_and_show, toggle_done_body};
use crate::{api, mascot, settings, speech, voice};

/// The single Rambo element: independent fixed layer above everything.
///
/// Rambo is not a child of the nest or the capture form. He sits at a
/// `position: fixed` spot measured from the nest's bounding box. Drag moves
/// only Rambo; he follows the pointer exactly and skitters back to the nest.
#[component]
pub(crate) fn Rambo(
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
    /// The nest button: Rambo measures its bounding box to know where home is
    nest_ref: NodeRef<leptos::html::Button>,
    /// The tool he holds: it decides which drawing of each pose he wears
    pen: Signal<settings::Pen>,
    /// True while you are typing, so he peeks over at the box
    typing: ReadSignal<bool>,
) -> impl IntoView {
    // Measure the nest's center so Rambo knows where to sit at home.
    // Update on mount, on resize, and whenever state changes to Home.
    #[cfg(target_arch = "wasm32")]
    {
        // Reactive effect: re-measure when state changes (so returning home updates pen_pos)
        Effect::new(move |_| {
            let _ = state.get(); // track state
            if let Some(nest) = nest_ref.get() {
                let rect = nest.get_bounding_client_rect();
                let cx = rect.left() + rect.width() / 2.0;
                let cy = rect.top() + rect.height() / 2.0;
                if matches!(state.get_untracked(), mascot::State::Home) {
                    pen_pos.set((cx, cy));
                }
            }
        });
        // Also measure on window resize (non-reactive, just a listener)
        window_event_listener(leptos::ev::resize, move |_| {
            if matches!(state.get_untracked(), mascot::State::Home) {
                if let Some(nest) = nest_ref.get() {
                    let rect = nest.get_bounding_client_rect();
                    let cx = rect.left() + rect.width() / 2.0;
                    let cy = rect.top() + rect.height() / 2.0;
                    pen_pos.set((cx, cy));
                }
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (nest_ref, pen_pos, state);

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
            // He stops mid-sentence: his own voice would otherwise come back
            // through the microphone as a transcript of what he just said.
            voice::cancel();
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
                // Save grab state: pointer position and pen position at grab
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
                    // Rambo follows the pointer exactly — pen_pos is the pointer position
                    pen_pos.set((ev.client_x() as f64, ev.client_y() as f64));
                }
            }
            on:pointerup=move |ev: leptos::ev::PointerEvent| {
                stop_speech_up();
                if !matches!(state.get_untracked(), mascot::State::Dragging) {
                    return;
                }
                let did_move = *moved_for_up.borrow();
                if !did_move {
                    // Tap (no drag): submit the form (sends typed text) and go home
                    *grab_state_up.borrow_mut() = None;
                    if let Some(form) = form_ref.get() {
                        let _ = form.request_submit();
                    }
                    set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Release));
                    return;
                }
                // Update pen position one last time to the pointer position; also compute
                // drop velocity to detect hard drops (fast release → splat).
                let hard_drop = if let Some((gx, gy, _ox, _oy, grab_time)) = *grab_state_up.borrow() {
                    let px = ev.client_x() as f64;
                    let py = ev.client_y() as f64;
                    pen_pos.set((px, py));
                    let elapsed = (js_sys::Date::now() as u64).saturating_sub(grab_time) as f64;
                    if elapsed > 0.0 {
                        let dx = px - gx;
                        let dy = py - gy;
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
            // Speech bubble: inside the button so it follows Rambo everywhere
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
        </button>
    }
}

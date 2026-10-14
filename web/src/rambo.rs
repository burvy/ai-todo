//! Rambo the pen: dragging, flying, landing on a row, and listening.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use todo_core::{Snapshot, TaskId};
use wasm_bindgen::JsCast;

use crate::edits::{
    EditSink, heard_edit_applies, revisable, revise_and_show, revise_body, series_note, sheet_left,
};
use crate::tasks::{apply_tick, delete_with_scribble, tick_action};
use crate::{ink, mascot, settings, speech, tuning, voice};

/// The single Rambo element: independent fixed layer above everything.
///
/// Rambo is not a child of the nest or the capture form. He sits at a
/// `position: fixed` spot measured from the nest's bounding box. Drag moves
/// only Rambo; he follows the pointer exactly. Dropped on the page he is dazed
/// where he lands, writes when dragged again, and skitters back to the nest.
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
    /// Set to the task he was dropped on when you choose "Edit": the list opens its sheet
    edit_request: RwSignal<Option<TaskId>>,
    /// The task whose edit sheet is open, if any: listening for an edit ends with it
    open_sheet: RwSignal<Option<TaskId>>,
    /// The nest button: Rambo measures its bounding box to know where home is
    nest_ref: NodeRef<leptos::html::Button>,
    /// The corner a deleted row's scrap lies in: his × lays one like any delete
    scraps: RwSignal<Option<crate::scrap::Scrap>>,
    /// The tool he holds: it decides which drawing of each pose he wears
    pen: Signal<settings::Pen>,
    /// True while you are typing, so he peeks over at the box
    typing: ReadSignal<bool>,
    /// What a dazed Rambo has written on the page, one path per drag
    strokes: RwSignal<Vec<ink::Path>>,
) -> impl IntoView {
    // At home he sits on the nest wherever the page has carried it. He is a
    // fixed layer and the nest is not, so every time the page moves he
    // measures it again.
    let follow_nest = move || {
        if state.get_untracked() == mascot::State::Home
            && let Some(home) = nest_centre(nest_ref)
        {
            pen_pos.set(home);
        }
    };
    Effect::new(move |_| {
        state.track();
        follow_nest();
    });
    #[cfg(target_arch = "wasm32")]
    on_page_moved(follow_nest);

    // Near an edge of the screen his ask bubble slides in rather than run off it.
    let bubble_ref = NodeRef::<leptos::html::Span>::new();
    let bubble_shift = RwSignal::new(0.0_f64);
    Effect::new(move |_| {
        let asking = matches!(state.get(), mascot::State::Asking { .. });
        let shift = bubble_ref
            .get()
            .filter(|_| asking)
            .zip(screen_width())
            .map_or(0.0, |(bubble, screen)| {
                // From values that do not animate: the bubble is centred on him, and its
                // drawn box is still riding his settle after the drop when this runs.
                let width = f64::from(bubble.offset_width());
                bubble_offset(pen_pos.get().0, width, screen, tuning::BUBBLE_EDGE_PX)
            });
        bubble_shift.set(shift);
    });

    // Home again, the page he wrote on is tidied away with him.
    Effect::new(move |_| {
        if state.get() == mascot::State::Home && !strokes.with_untracked(Vec::is_empty) {
            strokes.set(Vec::new());
        }
    });

    // The run home: hops along `skitter_point` from where he sat dazed to the
    // middle of the nest, then home.
    let skitter_home = move || {
        set_state.set(mascot::transition(
            &state.get_untracked(),
            mascot::Event::Return,
        ));
        let Some(home) = nest_centre(nest_ref) else {
            set_state.set(mascot::transition(
                &state.get_untracked(),
                mascot::Event::Return,
            ));
            return;
        };
        if mascot::prefers_reduced_motion() {
            set_state.set(mascot::transition(
                &state.get_untracked(),
                mascot::Event::Return,
            ));
            return;
        }
        let start = pen_pos.get_untracked();
        let begun = js_sys::Date::now();
        // The nest can move while he runs (the page scrolls), so he aims at
        // where it is now, not where it was when he set off.
        let run = std::rc::Rc::new(std::cell::Cell::new(None::<IntervalHandle>));
        let handle = set_interval_with_handle(
            {
                let run = run.clone();
                move || {
                    let progress =
                        ((js_sys::Date::now() - begun) / f64::from(mascot::SKITTER_MS)) as f32;
                    let running = state.get_untracked() == mascot::State::Skittering;
                    if running {
                        let home = nest_centre(nest_ref).unwrap_or(home);
                        pen_pos.set(mascot::skitter_point(start, home, 3, 14.0, progress));
                    }
                    if !running || progress >= 1.0 {
                        if let Some(handle) = run.take() {
                            handle.clear();
                        }
                        if running {
                            set_state.set(mascot::transition(
                                &state.get_untracked(),
                                mascot::Event::Return,
                            ));
                        }
                    }
                }
            },
            std::time::Duration::from_millis(16),
        );
        run.set(handle.ok());
    };

    // Each daze gets a ticket; a newer drop or a grab takes a new one, so an
    // older timer finds its ticket stale and leaves him be.
    let daze = std::rc::Rc::new(std::cell::Cell::new(0_u32));
    let arm_daze = {
        let daze = daze.clone();
        move || {
            let ticket = daze.get().wrapping_add(1);
            daze.set(ticket);
            let daze = daze.clone();
            Timeout::new(mascot::DAZED_MS, move || {
                if daze.get() == ticket && state.get_untracked() == mascot::State::Dazed {
                    skitter_home();
                }
            })
            .forget();
        }
    };
    // Let go on the page: dazed where he lands, after a splat if he was flung.
    let land_dazed = {
        let arm_daze = arm_daze.clone();
        move |hard_drop: bool| {
            let event = if hard_drop && !mascot::prefers_reduced_motion() {
                mascot::Event::DropHard
            } else {
                mascot::Event::DropEmpty
            };
            let landed = mascot::transition(&state.get_untracked(), event);
            set_state.set(landed.clone());
            if landed == mascot::State::Splatting {
                let arm_daze = arm_daze.clone();
                Timeout::new(mascot::SPLAT_MS, move || {
                    set_state.set(mascot::transition(
                        &state.get_untracked(),
                        mascot::Event::Return,
                    ));
                    arm_daze();
                })
                .forget();
            } else {
                arm_daze();
            }
        }
    };
    // How far below his middle the nib is, measured when he is picked up.
    let nib_below = std::rc::Rc::new(std::cell::Cell::new(0.0_f64));

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
    // A long press listens: each grab takes a ticket, and the hold's timer starts
    // listening only if its grab is still the current one and he was held still.
    let hold = std::rc::Rc::new(std::cell::Cell::new(0_u32));
    let still = std::rc::Rc::new(std::cell::Cell::new(false));

    // Speech session for grab-to-listen: transcript goes into capture_text so
    // the capture box can submit it when the user releases Rambo.
    let speech_session = std::rc::Rc::new(std::cell::RefCell::new(None::<speech::Session>));
    let transcript = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    // True while a hold or the keyboard has him listening: only then does letting
    // go send what he heard.
    let listening = std::rc::Rc::new(std::cell::Cell::new(false));
    let start_speech = {
        let ss = speech_session.clone();
        let listening = listening.clone();
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
                    listening.set(true);
                    set_talking.set(true);
                }
                Err(message) => set_error.set(Some(message)),
            }
        }
    };
    let stop_speech_and_send = {
        let ss = speech_session.clone();
        let listening = listening.clone();
        let capture = capture_text;
        let form = form_ref;
        move || {
            if !listening.replace(false) {
                return;
            }
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

    // Edit opens the sheet and he listens: what he hears is the change, which the
    // server's revise call (Jev) applies, and the sheet then shows the result. The
    // session ends by itself after a pause, so nothing has to be let go.
    let editing = RwSignal::new(None::<TaskId>);
    let heard = RwSignal::new(String::new());
    let edit_session = StoredValue::new_local(None::<speech::Session>);
    let listen_for_edit = move |id: TaskId| {
        // Quietly not: the sheet is open, and typing in it still works.
        if !speech::supported() || !snapshot.with_untracked(|s| revisable(s, &id.0)) {
            return;
        }
        voice::cancel();
        set_error.set(None);
        heard.set(String::new());
        editing.set(Some(id));
        let result = speech::Session::start(
            std::rc::Rc::new(std::cell::RefCell::new(String::new())),
            move |text| heard.set(text),
            move |on| {
                set_talking.set(on);
                if !on {
                    send_heard_edit(
                        editing,
                        heard,
                        snapshot,
                        EditSink {
                            set_snapshot,
                            set_error,
                        },
                        edit_request,
                        open_sheet,
                    );
                }
            },
            // Silence is fine here: the sheet is open, so typing the change works too.
            move |value: Option<String>| {
                if value.as_deref() != Some(speech::NO_SPEECH_NOTE) {
                    set_error.set(value);
                }
            },
        );
        match result {
            Ok(session) => {
                edit_session.set_value(Some(session));
                set_talking.set(true);
            }
            Err(message) => set_error.set(Some(message)),
        }
    };

    // Closing the sheet, or opening another task's, ends the listening for it, and
    // what was heard is dropped rather than sent.
    Effect::new(move |was: Option<Option<TaskId>>| {
        let now = open_sheet.get();
        if let Some(was) = was
            && sheet_left(editing.get_untracked().as_ref(), was.as_ref(), now.as_ref())
        {
            editing.set(None);
            edit_session.with_value(|session| {
                if let Some(session) = session {
                    session.stop();
                }
            });
        }
        now
    });

    // Clone closures for multiple event handlers
    let still_move = still.clone();
    let hold_up = hold.clone();
    let hold_cancel = hold.clone();
    let listening_up = listening.clone();
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
            on:pointerdown={
                let daze = daze.clone();
                let nib_below = nib_below.clone();
                move |ev: leptos::ev::PointerEvent| {
                    ev.prevent_default();
                    let before = state.get_untracked();
                    if mascot::pen_busy(&before) {
                        return;
                    }
                    let after = mascot::transition(&before, mascot::Event::Grab);
                    if after == before {
                        // Splatting, on his way home, or asking: not now.
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
                    // In hand he never skitters off on his own.
                    daze.set(daze.get().wrapping_add(1));
                    set_state.set(after.clone());
                    let ticket = hold.get().wrapping_add(1);
                    hold.set(ticket);
                    if mascot::listens_on_grab(&before) {
                        // Held still, he listens; a drag only carries him.
                        still.set(true);
                        let (hold, still) = (hold.clone(), still.clone());
                        let start = start_speech_down.clone();
                        Timeout::new(mascot::LISTEN_HOLD_MS, move || {
                            if hold.get() == ticket
                                && still.get()
                                && state.get_untracked() == mascot::State::Dragging
                            {
                                start();
                            }
                        })
                        .forget();
                    } else if let Some(rambo) = rambo_ref.get() {
                        // Dazed: this drag writes, starting where the nib is now.
                        let below = rambo.get_bounding_client_rect().height() * mascot::NIB_BELOW;
                        nib_below.set(below);
                        strokes.update(|strokes| strokes.push(vec![(ox, oy + below)]));
                    }
                    if let Some(el) = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                        let _ = el.set_pointer_capture(ev.pointer_id());
                    }
                }
            }
            on:pointermove={
                let nib_below = nib_below.clone();
                move |ev: leptos::ev::PointerEvent| {
                    let held = state.get_untracked();
                    if !matches!(held, mascot::State::Dragging | mascot::State::Inking) {
                        return;
                    }
                    *moved_move.borrow_mut() = true;
                    // Rambo follows the pointer exactly — pen_pos is the pointer position
                    let (x, y) = (ev.client_x() as f64, ev.client_y() as f64);
                    if let Some((gx, gy, ..)) = *grab_state_move.borrow()
                        && !mascot::held_still((gx, gy), (x, y))
                    {
                        still_move.set(false);
                    }
                    pen_pos.set((x, y));
                    if held == mascot::State::Inking {
                        let below = nib_below.get();
                        strokes.update(|strokes| {
                            if let Some(stroke) = strokes.last_mut() {
                                stroke.push((x, y + below));
                            }
                        });
                    }
                }
            }
            on:pointerup={
                let arm_daze = arm_daze.clone();
                let land_dazed = land_dazed.clone();
                move |ev: leptos::ev::PointerEvent| {
                    let before = state.get_untracked();
                    // Let go before the hold ended: he never starts listening.
                    hold_up.set(hold_up.get().wrapping_add(1));
                    let was_listening = listening_up.get();
                    stop_speech_up();
                    if !matches!(before, mascot::State::Dragging | mascot::State::Inking) {
                        return;
                    }
                    let did_move = *moved_for_up.borrow();
                    if !did_move {
                        *grab_state_up.borrow_mut() = None;
                        if before == mascot::State::Dragging && !was_listening {
                            // Tap (no drag): submit the form (sends typed text) and go home
                            if let Some(form) = form_ref.get() {
                                let _ = form.request_submit();
                            }
                        } else {
                            arm_daze();
                        }
                        set_state.set(mascot::transition(&before, mascot::Event::Release));
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

                    // Put back in his nest: home at once, and forgiven a little
                    // if he was dazed.
                    let over_nest = nest_ref.get().is_some_and(|nest| {
                        let rect = nest.get_bounding_client_rect();
                        mascot::over_nest(
                            (client_x, client_y),
                            (rect.left(), rect.top(), rect.right(), rect.bottom()),
                        )
                    });
                    if over_nest {
                        let after = mascot::transition(&before, mascot::Event::DropNest);
                        if mascot::put_back(&before, &after) {
                            set_brain_event.set(Some(rambl_brain::Event::PutBack));
                        }
                        set_state.set(after);
                        return;
                    }
                    // A listening Rambo dropped on a task row asks what to do with it.
                    if before == mascot::State::Dragging
                        && let Some(row) = row_under(client_x, client_y)
                    {
                        let task_id = row.get_attribute("data-id").unwrap_or_default();
                        let task_title = row.get_attribute("data-title").unwrap_or_default();
                        set_state.set(mascot::transition(
                            &before,
                            mascot::Event::DropTask { task_id, title: task_title },
                        ));
                        return;
                    }
                    land_dazed(hard_drop);
                }
            }
            on:pointercancel={
                let arm_daze = arm_daze.clone();
                let land_dazed = land_dazed.clone();
                move |_| match state.get_untracked() {
                    mascot::State::Dragging => {
                        hold_cancel.set(hold_cancel.get().wrapping_add(1));
                        stop_speech_cancel();
                        land_dazed(false);
                    }
                    mascot::State::Inking => {
                        // The browser took the gesture over: he lands where he
                        // is, like a drop on the page.
                        set_state.set(mascot::transition(
                            &state.get_untracked(),
                            mascot::Event::DropEmpty,
                        ));
                        arm_daze();
                    }
                    _ => {}
                }
            }
            on:keydown=move |ev| {
                if ev.key() == " " || ev.key() == "Enter" {
                    ev.prevent_default();
                    if talking.get_untracked() {
                        stop_speech_key();
                        set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Release));
                    } else if mascot::listens_on_grab(&state.get_untracked()) {
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
                        <span
                            class="ask"
                            node_ref=bubble_ref
                            style:margin-left=move || format!("{:.1}px", bubble_shift.get())
                        >
                            <span>{format!("Hey! What do you want with \"{title}\"?")}</span>
                            <button type="button" on:click=move |_| {
                                if let mascot::State::Asking { ref task_id, .. } = state.get_untracked() {
                                    let id = TaskId(task_id.clone());
                                    edit_request.set(Some(id.clone()));
                                    listen_for_edit(id);
                                }
                                set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Choose));
                            }>"Edit"</button>
                            <button type="button" on:click=move |_| {
                                set_state.set(mascot::transition(&state.get_untracked(), mascot::Event::Choose));
                            }>"Move"</button>
                            <button type="button" on:click=move |_| {
                                if let mascot::State::Asking { ref task_id, .. } = state.get_untracked() {
                                    let task_id = task_id.clone();
                                    // The list's tick, so a repeat's day is marked on its own date.
                                    let action = snapshot.with_untracked(|s| tick_action(s, &task_id));
                                    let sink = EditSink { set_snapshot, set_error };
                                    let set_state_clone = set_state;
                                    wasm_bindgen_futures::spawn_local(async move {
                                        if let Some(action) = action {
                                            apply_tick(&TaskId(task_id), action, sink).await;
                                        }
                                        set_state_clone.set(mascot::transition(&state.get_untracked(), mascot::Event::Choose));
                                    });
                                }
                            }>"Done"</button>
                            <button type="button" on:click=move |_| {
                                if let mascot::State::Asking { ref task_id, .. } = state.get_untracked() {
                                    let task_id = task_id.clone();
                                    let set_snapshot_clone = set_snapshot;
                                    let set_state_clone = set_state;
                                    wasm_bindgen_futures::spawn_local(async move {
                                        // A delete like any other: crossed out first, and a
                                        // repeat's row skips just its own date.
                                        match delete_with_scribble(&TaskId(task_id), snapshot, scraps).await {
                                            Ok(new_snap) => set_snapshot_clone.set(new_snap),
                                            Err(e) => set_error.set(Some(series_note(&e))),
                                        }
                                        set_state_clone.set(mascot::transition(&state.get_untracked(), mascot::Event::Choose));
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

/// Send what Rambo heard while an edit sheet was open, once: the speech session
/// reports its end more than once (an error, then the end).
fn send_heard_edit(
    editing: RwSignal<Option<TaskId>>,
    heard: RwSignal<String>,
    snapshot: ReadSignal<Snapshot>,
    sink: EditSink,
    edit_request: RwSignal<Option<TaskId>>,
    open_sheet: RwSignal<Option<TaskId>>,
) {
    let Some(id) = editing.try_update(Option::take).flatten() else {
        return;
    };
    if !heard_edit_applies(Some(&id), open_sheet.get_untracked().as_ref()) {
        return;
    }
    let Some(body) = snapshot.with_untracked(|s| revise_body(s, &id.0, &heard.get_untracked()))
    else {
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        if revise_and_show(&id, body, sink).await {
            // Fill the sheet again from the result; fields typed meanwhile are kept.
            edit_request.set(Some(id));
        }
    });
}

/// The middle of the nest on screen, where he sits at home.
fn nest_centre(nest_ref: NodeRef<leptos::html::Button>) -> Option<(f64, f64)> {
    let rect = nest_ref.get()?.get_bounding_client_rect();
    Some((
        rect.left() + rect.width() / 2.0,
        rect.top() + rect.height() / 2.0,
    ))
}

/// Calls `moved` whenever the page may have shifted under a fixed layer: any
/// scroll (the window's, or the panes' sideways one on a phone), a resize, the
/// page changing size as tasks load and a scrollbar appears, and the fonts
/// settling. None of these but the resize reaches a plain window listener.
#[cfg(target_arch = "wasm32")]
fn on_page_moved(moved: impl Fn() + Copy + 'static) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let listener = wasm_bindgen::closure::Closure::<dyn Fn()>::new(moved);
    let callback = listener.as_ref().unchecked_ref();
    // Scroll does not bubble; the capture phase hears every scroller.
    let _ = window.add_event_listener_with_callback_and_bool("scroll", callback, true);
    let _ = window.add_event_listener_with_callback("resize", callback);
    if let Some(root) = window.document().and_then(|doc| doc.document_element())
        && let Ok(observer) = web_sys::ResizeObserver::new(callback)
    {
        observer.observe(&root);
    }
    // Rambo lives as long as the page, and so do his listeners.
    listener.forget();
    wasm_bindgen_futures::spawn_local(async move {
        crate::tasks::write::wait_for_fonts().await;
        moved();
    });
}

/// The task row under a point, looking past Rambo himself: he is under the
/// pointer while he is carried, so the topmost element there is always him.
fn row_under(x: f64, y: f64) -> Option<web_sys::Element> {
    let doc = web_sys::window()?.document()?;
    doc.elements_from_point(x as f32, y as f32)
        .iter()
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .filter(|el| el.closest(".rambo").ok().flatten().is_none())
        .find_map(|el| el.closest("li[data-id]").ok().flatten())
}

/// The page's width without its scrollbar, which the bubble must stay inside.
fn screen_width() -> Option<f64> {
    let root = web_sys::window()?.document()?.document_element()?;
    Some(f64::from(root.client_width()))
}

/// How far to slide a bubble of `width`, centred on `centre_x`, so it keeps `edge`
/// pixels from both sides of a `screen` that wide. A bubble too wide for the screen
/// starts at the left edge, where its question begins.
fn bubble_offset(centre_x: f64, width: f64, screen: f64, edge: f64) -> f64 {
    let left = centre_x - width / 2.0;
    let fitted = if width > screen - 2.0 * edge {
        edge
    } else {
        left.clamp(edge, screen - edge - width)
    };
    fitted - left
}

#[cfg(test)]
mod tests {
    use super::bubble_offset;

    // Rambo at x, a 200px bubble, a 1000px screen, 16px from each edge.

    #[test]
    fn a_bubble_that_fits_stays_centred() {
        assert_eq!(bubble_offset(500.0, 200.0, 1000.0, 16.0), 0.0);
    }

    #[test]
    fn near_the_left_edge_it_slides_right() {
        // It spans -70 to 130, so it moves 86 to start at 16.
        assert_eq!(bubble_offset(30.0, 200.0, 1000.0, 16.0), 86.0);
        assert_eq!(bubble_offset(10.0, 200.0, 1000.0, 16.0), 106.0);
    }

    #[test]
    fn near_the_right_edge_it_slides_left() {
        // It spans 870 to 1070, so it moves 86 to end at 984.
        assert_eq!(bubble_offset(970.0, 200.0, 1000.0, 16.0), -86.0);
    }

    #[test]
    fn a_bubble_wider_than_the_screen_starts_at_the_left_edge() {
        // It spans -100 to 1100; it starts at 16 and runs off the right instead.
        assert_eq!(bubble_offset(500.0, 1200.0, 1000.0, 16.0), 116.0);
        // Exactly as wide as the room between the edges: it fits.
        assert_eq!(bubble_offset(500.0, 968.0, 1000.0, 16.0), 0.0);
    }
}

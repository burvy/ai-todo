//! Rambo the pen: dragging, flying, landing on a row, and listening.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use todo_core::{Snapshot, TaskId};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::edits::{
    EditSink, heard_edit_applies, revisable, revise_and_show, revise_body, sheet_left,
};
use crate::tasks::{apply_tick, tick_action};
use crate::{gesture, ink, mascot, settings, sound, speech, tuning, voice};

/// The single Rambo element: independent fixed layer above everything.
///
/// Rambo is not a child of the nest or the capture form. He sits at a
/// `position: fixed` spot measured from the nest's bounding box. Drag moves
/// only Rambo; he rides just above the pointer. Dropped on the page he is dazed
/// where he lands, writes when dragged again, and hops back to the nest.
#[component]
pub(crate) fn Rambo(
    /// The empty planner hour he was dropped on, as a date phrase: the capture box's
    /// next task starts then
    noting_at: RwSignal<Option<crate::capture::NotedHour>>,
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
    /// Set to the task he was dropped on when you choose "Move": the planner places it
    move_request: RwSignal<Option<TaskId>>,
    /// The task whose edit sheet is open, if any: listening for an edit ends with it
    open_sheet: RwSignal<Option<TaskId>>,
    /// The nest button: Rambo measures its bounding box to know where home is
    nest_ref: NodeRef<leptos::html::Button>,
    /// The tool he holds: it decides which drawing of each pose he wears
    pen: Signal<settings::Pen>,
    /// True while you are typing, so he peeks over at the box
    typing: ReadSignal<bool>,
    /// What a dazed Rambo has written on the page, one path per drag
    strokes: RwSignal<Vec<ink::Stroke>>,
) -> impl IntoView {
    // Where he is drawn, and how squashed by a hop: see the hop mover below.
    let shown = RwSignal::new(pen_pos.get_untracked());
    let squash = RwSignal::new(0.0_f64);
    // The face he makes in the beat before he sets off (`mascot::notice`).
    let noticing = RwSignal::new(None::<mascot::Pose>);
    // At home he sits on the nest wherever the page has carried it. He is a
    // fixed layer and the nest is not, so every time the page moves he
    // measures it again.
    let nest_at = RwSignal::new(None::<(f64, f64, f64, f64)>);
    let follow_nest = move || {
        let nest = nest_box(nest_ref);
        nest_at.set(nest);
        let nest = nest.map(mascot::nest_home);
        if matches!(
            state.get_untracked(),
            mascot::State::Home | mascot::State::Skittering
        ) && let Some(home) = nest
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

    // Near an edge of the screen his ask bubble slides in rather than run off it,
    // and near the top it opens below him (dropped on the first row).
    let bubble_ref = NodeRef::<leptos::html::Span>::new();
    let bubble_shift = RwSignal::new(0.0_f64);
    let bubble_below = RwSignal::new(false);
    // What the bubble asks about. It outlives the ask by `ASK_FOLD_MS`, the time
    // it takes to fold back into him (`folding`), so it never vanishes in a frame.
    let asked = RwSignal::new(None::<String>);
    // What he was dropped on, a row or a slip, which decides the bubble's choices.
    let asked_on = RwSignal::new(mascot::Asked::Row);
    let folding = RwSignal::new(false);
    // Opening, it starts tucked into him until it is measured and placed (above or
    // below him), then grows out to that place.
    let tucked = RwSignal::new(false);
    let fold_ticket = std::rc::Rc::new(std::cell::Cell::new(0_u32));
    Effect::new(move |_| match state.get() {
        mascot::State::Asking { title, .. } => {
            fold_ticket.set(fold_ticket.get().wrapping_add(1));
            folding.set(false);
            if asked.with_untracked(Option::is_none) {
                tucked.set(true);
                // One frame to lay it out and place it, one to draw it tucked there.
                crate::engine::next_frame(move || {
                    crate::engine::next_frame(move || tucked.set(false));
                });
            }
            asked.set(Some(title));
        }
        _ if asked.with_untracked(Option::is_some) && !folding.get_untracked() => {
            folding.set(true);
            let ticket = fold_ticket.get().wrapping_add(1);
            fold_ticket.set(ticket);
            let fold_ticket = fold_ticket.clone();
            crate::engine::every(f64::from(tuning::ASK_FOLD_MS), move || {
                if fold_ticket.get() == ticket {
                    asked.set(None);
                    folding.set(false);
                }
                crate::engine::Flow::Stop
            });
        }
        _ => {}
    });
    Effect::new(move |_| {
        // Measured while it shows; folding, it keeps the place it opened in.
        if folding.get() {
            return;
        }
        // A line he starts or stops saying moves what the bubble sits on.
        brain_speech.track();
        let shown_bubble = asked.with(Option::is_some);
        let (shift, below) = bubble_ref
            .get()
            .filter(|_| shown_bubble)
            .zip(screen_width())
            .map_or((0.0, false), |(bubble, screen)| {
                // From values that do not animate: the bubble is centred on him, and its
                // drawn box is still riding his settle after the drop when this runs.
                let width = f64::from(bubble.offset_width());
                let his_h = rambo_height();
                let his_top = shown.get().1 - his_h / 2.0;
                // It sits on his status line, which is taller while he says something.
                let line_h = bubble
                    .parent_element()
                    .map_or(0.0, |line| line.get_bounding_client_rect().height());
                (
                    bubble_offset(shown.get().0, width, screen, tuning::BUBBLE_EDGE_PX),
                    opens_below(
                        his_top - line_h,
                        f64::from(bubble.offset_height()),
                        tuning::BUBBLE_EDGE_PX,
                    ),
                )
            });
        bubble_shift.set(shift);
        bubble_below.set(below);
    });

    // Where he is drawn: his body in the engine's world (`engine::World`), which hops,
    // rides after the finger, keeps his notice beat and times his splat and daze. Once
    // a frame the page reads where the nest and his spot (`pen_pos`) are, steps the
    // world and writes what it draws. His brain state goes both ways: the rest of the
    // page sets `state`, and the world's own timers and landings set it back.
    let world = std::rc::Rc::new(std::cell::RefCell::new(crate::engine::World::new(
        pen_pos.get_untracked(),
    )));
    // His pose on top of where he is: lean, size, sink, the writing pose
    // (`engine::look`). With `shown`, his button's only transform.
    let look = RwSignal::new(world.borrow().look());
    // How the nest sags and rocks under him (`engine::Nest`): both its layers, the
    // back on its button and the rim over him, move by it.
    let nest_shift = StoredValue::new((0.0_f64, 0.0_f64));
    let rim_ref = NodeRef::<leptos::html::Span>::new();
    let ticking = std::rc::Rc::new(std::cell::Cell::new(false));
    let thrower = world.clone();
    let sweeper = world.clone();
    let listener = world.clone();
    let wake = move || {
        if ticking.replace(true) {
            return;
        }
        let (world, ticking) = (world.clone(), ticking.clone());
        crate::engine::on_tick(move |tick| {
            let anchors = crate::engine::Anchors {
                nest: nest_box(nest_ref)
                    .map(mascot::nest_home)
                    .unwrap_or_else(|| pen_pos.get_untracked()),
                spot: pen_pos.get_untracked(),
                reduced_motion: mascot::prefers_reduced_motion(),
                leaf: crate::engine::leaf(),
                walls: screen_walls(),
            };
            let (brain, draw, posed, at_rest, laid, knocks, nest_moved) = {
                let mut world = world.borrow_mut();
                world.adopt(state.get_untracked());
                // The music is heard only when he could dance to it, so the rest of the
                // time nothing reads it and his loop may sleep.
                let level = world
                    .can_dance(anchors.reduced_motion)
                    .then(crate::music::level)
                    .flatten();
                world.hear(level);
                world.advance(tick.steps, &anchors);
                crate::engine::set_push(world.leaf_push());
                (
                    world.rambo.brain.clone(),
                    world.draw(),
                    world.look(),
                    world.at_rest(),
                    world.take_ink(),
                    world.take_knocks(),
                    world.nest_offset(),
                )
            };
            if knocks > 0 {
                sound::play(sound::Cue::Knock);
            }
            if !laid.is_empty() {
                let pen = pen.get_untracked();
                strokes.update(|strokes| {
                    strokes.extend(
                        laid.into_iter()
                            .flat_map(|splash| ink_of(splash, pen, tick.now)),
                    );
                });
            }
            if brain != state.get_untracked() {
                set_state.set(brain);
            }
            if shown.get_untracked() != draw.pos {
                shown.set(draw.pos);
            }
            if squash.get_untracked() != draw.squash {
                squash.set(draw.squash);
            }
            if nest_shift.get_value() != nest_moved {
                nest_shift.set_value(nest_moved);
                let (sag, rock) = nest_moved;
                let layers = [
                    nest_ref
                        .get_untracked()
                        .map(|nest| web_sys::HtmlElement::style(&nest)),
                    rim_ref
                        .get_untracked()
                        .map(|rim| web_sys::HtmlElement::style(&rim)),
                ];
                for style in layers.into_iter().flatten() {
                    let _ = style.set_property("--nest-sag", &format!("{sag:.2}px"));
                    let _ = style.set_property("--nest-rock", &format!("{rock:.2}deg"));
                }
            }
            if look.get_untracked() != posed {
                look.set(posed);
            }
            if noticing.get_untracked() != draw.notice {
                noticing.set(draw.notice);
            }
            // The first frame after a sleep takes no step, so it never ends the run.
            if tick.steps > 0 && at_rest {
                ticking.set(false);
                crate::engine::Flow::Stop
            } else {
                crate::engine::Flow::Go
            }
        });
    };
    let wake = std::rc::Rc::new(wake);
    // The page turn moving its leaf wakes him, so a leaf closing over him is seen even
    // while his loop sleeps.
    crate::engine::on_leaf({
        let wake = wake.clone();
        move || wake()
    });
    Effect::new({
        let wake = wake.clone();
        move |_| {
            state.track();
            pen_pos.track();
            wake();
        }
    });
    // With music on he listens for it now and then, and wakes to dance when it plays and
    // he could dance; while he dances his loop keeps hearing it. Picking another preset
    // stops the listening.
    let listening = std::rc::Rc::new(std::cell::Cell::new(0_u32));
    Effect::new({
        let (wake, world) = (wake.clone(), listener);
        move |_| {
            if view_settings.with(|s| s.sound) != settings::SoundPreset::Music {
                return;
            }
            let ticket = listening.get().wrapping_add(1);
            listening.set(ticket);
            let (wake, listening, world) = (wake.clone(), listening.clone(), world.clone());
            crate::engine::every(tuning::DANCE_LISTEN_MS, move || {
                let still_music = listening.get() == ticket
                    && view_settings.with_untracked(|s| s.sound) == settings::SoundPreset::Music;
                if !still_music {
                    return crate::engine::Flow::Stop;
                }
                let could_dance = world
                    .try_borrow()
                    .is_ok_and(|world| world.can_dance(mascot::prefers_reduced_motion()));
                if could_dance && crate::music::level().is_some() {
                    wake();
                }
                crate::engine::Flow::Go
            });
        }
    });

    // Let go on the page: let go mid-flick he is thrown at the flick's speed (the
    // world slides him, and only a smack into an edge dazes him); gently, he hops home.
    let release_on_page = move |flick: Option<(f64, f64)>| {
        let before = state.get_untracked();
        let event = match flick {
            Some(vel) if before == mascot::State::Dragging && !mascot::prefers_reduced_motion() => {
                thrower.borrow_mut().throw(vel);
                mascot::Event::DropHard
            }
            _ => mascot::Event::DropEmpty,
        };
        set_state.set(mascot::transition(&before, event));
    };
    // The paper he lies on drags him as it scrolls (Interview 1 item 2): slowly he rides
    // along on it; faster than he can hold on, he is flung with it, and the throw does
    // the rest (`engine::swept`).
    on_paper_scrolled(move |scroller, delta, dt_ms| {
        let before = state.get_untracked();
        if !on_paper(scroller.as_ref(), shown.get_untracked()) {
            return;
        }
        match crate::engine::swept(&before, delta, dt_ms, mascot::prefers_reduced_motion()) {
            crate::engine::Swept::Stays => {}
            crate::engine::Swept::Carried(moved) => {
                pen_pos.update(|spot| *spot = (spot.0 + moved.0, spot.1 + moved.1));
            }
            crate::engine::Swept::Flung(vel) => {
                sweeper.borrow_mut().throw(vel);
                set_state.set(mascot::transition(&before, mascot::Event::Swept));
            }
        }
    });
    // How far below his middle the nib is, measured when he is picked up.
    let nib_below = std::rc::Rc::new(std::cell::Cell::new(0.0_f64));

    let rambo_ref = NodeRef::<leptos::html::Button>::new();

    // Track whether pointer moved (to distinguish Release from Drop)
    let moved = std::rc::Rc::new(std::cell::RefCell::new(false));
    let moved_down = moved.clone();
    let moved_move = moved.clone();
    let moved_for_up = moved.clone();

    // Where the pointer picked him up, to tell a still hold from a drag.
    let grab_state = std::rc::Rc::new(std::cell::RefCell::new(None::<(f64, f64)>));
    let grab_state_down = grab_state.clone();
    let grab_state_move = grab_state.clone();
    let grab_state_up = grab_state.clone();
    // The finger that holds him, read by the page's one gesture recogniser: letting
    // go mid-flick is a hard drop.
    let finger = std::rc::Rc::new(std::cell::RefCell::new(gesture::Recognizer::new()));
    // Whether `ev` comes from a finger other than the one holding him: a second
    // finger never moves, drops or throws him.
    let other_finger = {
        let finger = finger.clone();
        move |ev: &leptos::ev::PointerEvent| {
            finger
                .borrow()
                .finger()
                .is_some_and(|id| id != ev.pointer_id())
        }
    };
    let (finger_down, finger_move, finger_up, finger_cancel) = (
        finger.clone(),
        finger.clone(),
        finger.clone(),
        finger.clone(),
    );
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

    // Where he rides while you hold him at `pointer`: above it, by his height on
    // this screen.
    let held_spot = move |ev: &leptos::ev::PointerEvent, pointer: (f64, f64)| {
        let box_h = rambo_ref
            .get_untracked()
            .map_or(0.0, |rambo| f64::from(rambo.offset_height()));
        mascot::held_at(pointer, box_h, ev.pointer_type() == "touch")
    };

    // What each of the bubble's buttons does with the task he was dropped on.
    let choose = move |choice: mascot::Choice| {
        let mascot::State::Asking { task_id, .. } = state.get_untracked() else {
            return;
        };
        let chosen = move || {
            set_state.set(mascot::transition(
                &state.get_untracked(),
                mascot::Event::Choose,
            ))
        };
        match choice {
            mascot::Choice::Edit => {
                let id = TaskId(task_id);
                edit_request.set(Some(id.clone()));
                listen_for_edit(id);
                chosen();
            }
            mascot::Choice::Move => {
                move_request.set(Some(TaskId(task_id)));
                chosen();
            }
            mascot::Choice::Done => {
                // The list's tick, so a repeat's day is marked on its own date.
                let action = snapshot.with_untracked(|s| tick_action(s, &task_id));
                crate::scrap::note_departure(&task_id);
                let sink = EditSink {
                    set_snapshot,
                    set_error,
                };
                // He has his answer before the tick is saved, so a second tap on
                // Done finds him no longer asking and cannot tick it back.
                chosen();
                wasm_bindgen_futures::spawn_local(async move {
                    if let Some(action) = action {
                        apply_tick(&TaskId(task_id.clone()), action, sink).await;
                    }
                    crate::scrap::forget_departure(&task_id);
                });
            }
            mascot::Choice::Longer | mascot::Choice::Shorter => {
                let stretch = if choice == mascot::Choice::Longer {
                    crate::placing::Stretch::Longer
                } else {
                    crate::placing::Stretch::Shorter
                };
                crate::week::stretch(&TaskId(task_id), stretch, snapshot, set_snapshot, set_error);
                chosen();
            }
            mascot::Choice::Off => {
                crate::week::take_off(&TaskId(task_id), snapshot, set_snapshot, set_error);
                chosen();
            }
        }
    };

    // Dropped on an empty planner hour he notes a new task there: the box takes what
    // you type, and he listens, sending what he hears as you stop speaking. A tap off
    // the box, or Esc, is never mind (the ask's listeners below).
    let note_session = std::rc::Rc::new(std::cell::RefCell::new(None::<speech::Session>));
    Effect::new(move |_| {
        if state.get() != mascot::State::Noting {
            if let Some(session) = note_session.borrow_mut().take() {
                session.stop();
                set_talking.set(false);
            }
            if noting_at.with_untracked(Option::is_some) {
                noting_at.set(None);
            }
            return;
        }
        if let Some(input) = document()
            .query_selector(".capture input")
            .ok()
            .flatten()
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = input.focus();
        }
        if !speech::supported() {
            return;
        }
        voice::cancel();
        let session = speech::Session::start(
            std::rc::Rc::default(),
            move |heard| capture_text.set(heard),
            move |on| {
                set_talking.set(on);
                if !on
                    && state.get_untracked() == mascot::State::Noting
                    && !capture_text.with_untracked(|heard| heard.trim().is_empty())
                    && let Some(form) = form_ref.get_untracked()
                {
                    let _ = form.request_submit();
                }
            },
            // Typing the task instead of saying it is not a failure to hear one.
            move |note: Option<String>| {
                if note.as_deref() != Some(speech::NO_SPEECH_NOTE) {
                    set_error.set(note);
                }
            },
        );
        if let Ok(session) = session {
            *note_session.borrow_mut() = Some(session);
            set_talking.set(true);
        }
    });

    // While he asks, a tap anywhere off what he asks with (his bubble, or the capture
    // box while he notes a task), on him too, or Esc is never mind
    // (`mascot::asking_dismissed_by`). The listeners live only as long as the ask,
    // and start after the drop that opened it, so that drop is never a tap.
    let ask_listeners =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::<WindowListenerHandle>::new()));
    Effect::new(move |_| {
        let answer = match state.get() {
            mascot::State::Asking { .. } => Some(".ask"),
            mascot::State::Noting => Some(".capture"),
            _ => None,
        };
        ask_listeners
            .borrow_mut()
            .drain(..)
            .for_each(WindowListenerHandle::remove);
        let Some(answer) = answer else {
            return;
        };
        let finger = std::rc::Rc::new(std::cell::RefCell::new(gesture::Recognizer::new()));
        let on_bubble = std::rc::Rc::new(std::cell::Cell::new(false));
        let never_mind = move || {
            set_state.set(mascot::transition(
                &state.get_untracked(),
                mascot::Event::Return,
            ));
        };
        let sample = |ev: &leptos::ev::PointerEvent, pointer| gesture::Sample {
            pointer,
            at: (f64::from(ev.client_x()), f64::from(ev.client_y())),
            t: ev.time_stamp(),
            id: ev.pointer_id(),
        };
        let within = |ev: &leptos::ev::PointerEvent, selector: &str| {
            ev.target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                .and_then(|el| el.closest(selector).ok().flatten())
                .is_some()
        };
        let mut listeners = ask_listeners.borrow_mut();
        listeners.push(window_event_listener(leptos::ev::pointerdown, {
            let (finger, on_bubble) = (finger.clone(), on_bubble.clone());
            move |ev| {
                // Only the first finger's place counts: the recogniser ignores the rest.
                if finger.borrow().finger().is_none() {
                    on_bubble.set(within(&ev, answer));
                }
                let target = match within(&ev, ".rambo") {
                    true => gesture::Target::Rambo,
                    false => gesture::Target::Page(gesture::Leaf::List),
                };
                finger
                    .borrow_mut()
                    .advance(sample(&ev, gesture::Pointer::Down(target)));
            }
        }));
        listeners.push(window_event_listener(leptos::ev::pointermove, {
            let finger = finger.clone();
            move |ev| {
                finger
                    .borrow_mut()
                    .advance(sample(&ev, gesture::Pointer::Move));
            }
        }));
        listeners.push(window_event_listener(leptos::ev::pointerup, {
            let finger = finger.clone();
            move |ev| {
                let gesture = finger
                    .borrow_mut()
                    .advance(sample(&ev, gesture::Pointer::Up));
                if gesture.is_some_and(|g| {
                    mascot::asking_dismissed_by(&state.get_untracked(), &g, on_bubble.get())
                }) {
                    never_mind();
                }
            }
        }));
        listeners.push(window_event_listener(
            leptos::ev::pointercancel,
            move |ev| {
                finger
                    .borrow_mut()
                    .advance(sample(&ev, gesture::Pointer::Cancel));
            },
        ));
        // Esc anywhere on the page is never mind: nothing else is open while he asks.
        listeners.push(window_event_listener(leptos::ev::keydown, move |ev| {
            if ev.key() == "Escape" {
                never_mind();
                if let Some(rambo) = rambo_ref.get_untracked() {
                    let _ = rambo.focus();
                }
            }
        }));
        // The first choice takes the focus, so the keyboard can answer him at once.
        if answer != ".ask" {
            return;
        }
        crate::engine::next_frame(move || {
            if let Some(button) = bubble_ref
                .get_untracked()
                .and_then(|bubble| bubble.query_selector("button").ok().flatten())
                .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = button.focus();
            }
        });
    });

    // A stroke of his ink begun at `(x, y)`, on whatever lies there.
    let start_stroke = move |(x, y): (f64, f64), t: f64| {
        let page = list_page();
        let on = ink::surface_at(x, y, page);
        let (x, y) = ink::on_surface(on, x, y, page);
        strokes.update(|strokes| {
            strokes.push(ink::Stroke::new(
                pen.get_untracked(),
                on,
                ink::Sample { x, y, t },
            ))
        });
    };

    // Dazed, a hold anywhere on the page draws him to the finger, and the finger is
    // his from then on, as if it had picked him up there: his own handlers write
    // and let go. What had the finger's Down lets go of it first, so a row never
    // swipes or lifts beneath him.
    let draw_over = {
        let (nib_below, finger) = (nib_below.clone(), finger.clone());
        let (moved, grab_state) = (moved.clone(), grab_state.clone());
        move |watch: Watch| {
            watch.tick.stop();
            let before = state.get_untracked();
            let (Some(event), Some(rambo)) = (
                mascot::drawn_over(&before, &gesture::Gesture::Hold),
                rambo_ref.get_untracked(),
            ) else {
                return;
            };
            let_go(&watch.on, watch.id);
            *moved.borrow_mut() = false;
            *grab_state.borrow_mut() = Some(watch.at);
            let mut held = gesture::Recognizer::new();
            held.advance(gesture::Sample {
                pointer: gesture::Pointer::Down(gesture::Target::Rambo),
                at: watch.at,
                t: watch.t,
                id: watch.id,
            });
            *finger.borrow_mut() = held;
            let below = rambo.get_bounding_client_rect().height() * mascot::NIB_BELOW;
            nib_below.set(below);
            set_state.set(mascot::transition(&before, event));
            pen_pos.set(watch.at);
            start_stroke((watch.at.0, watch.at.1 + below), watch.t);
            let _ = rambo.set_pointer_capture(watch.id);
        }
    };

    // While he is dazed, and only then, the page's first finger is watched for a
    // hold. The engine's tick times it, so a finger that never moves still holds
    // on time.
    let watching = std::rc::Rc::new(std::cell::RefCell::new(None::<Watch>));
    let dazed = Memo::new(move |_| state.get() == mascot::State::Dazed);
    let watchers = StoredValue::new_local(Vec::<WindowListenerHandle>::new());
    Effect::new(move |_| {
        for listener in watchers
            .try_update_value(std::mem::take)
            .unwrap_or_default()
        {
            listener.remove();
        }
        if let Some(watch) = watching.borrow_mut().take() {
            watch.tick.stop();
        }
        if !dazed.get() {
            return;
        }
        // One step of the watched finger. A hold is his; anything else it decides
        // leaves the finger to the page.
        let step = {
            let (watching, draw_over) = (watching.clone(), draw_over.clone());
            move |pointer: gesture::Pointer, id: i32, at: Option<(f64, f64)>, t: f64| {
                let decided = {
                    let mut slot = watching.borrow_mut();
                    let Some(watch) = slot.as_mut().filter(|watch| watch.id == id) else {
                        return;
                    };
                    watch.at = at.unwrap_or(watch.at);
                    watch.t = t;
                    let sample = gesture::Sample {
                        pointer,
                        at: watch.at,
                        t,
                        id,
                    };
                    match watch.finger.advance(sample) {
                        Some(gesture) => gesture,
                        None => return,
                    }
                };
                let Some(watch) = watching.borrow_mut().take() else {
                    return;
                };
                if matches!(decided, gesture::Gesture::Hold) {
                    draw_over(watch);
                } else {
                    watch.tick.stop();
                }
            }
        };
        let down = {
            let (watching, step) = (watching.clone(), step.clone());
            window_event_listener(leptos::ev::pointerdown, move |ev| {
                if ev.button() != 0 || watching.borrow().is_some() {
                    return;
                }
                let Some(on) = ev
                    .target()
                    .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                else {
                    return;
                };
                // On him, his own handlers pick him up.
                if on.closest(".rambo").ok().flatten().is_some() {
                    return;
                }
                let (at, t, id) = (
                    (f64::from(ev.client_x()), f64::from(ev.client_y())),
                    ev.time_stamp(),
                    ev.pointer_id(),
                );
                let mut finger = gesture::Recognizer::new();
                finger.advance(gesture::Sample {
                    pointer: gesture::Pointer::Down(target_of(&on)),
                    at,
                    t,
                    id,
                });
                let step = step.clone();
                let tick = crate::engine::on_tick(move |tick| {
                    step(gesture::Pointer::Tick, id, None, tick.now);
                    crate::engine::Flow::Go
                });
                *watching.borrow_mut() = Some(Watch {
                    finger,
                    id,
                    on,
                    at,
                    t,
                    tick,
                });
            })
        };
        let at =
            |ev: &web_sys::PointerEvent| Some((f64::from(ev.client_x()), f64::from(ev.client_y())));
        let moved = {
            let step = step.clone();
            window_event_listener(leptos::ev::pointermove, move |ev| {
                step(
                    gesture::Pointer::Move,
                    ev.pointer_id(),
                    at(&ev),
                    ev.time_stamp(),
                );
            })
        };
        let up = {
            let step = step.clone();
            window_event_listener(leptos::ev::pointerup, move |ev| {
                step(
                    gesture::Pointer::Up,
                    ev.pointer_id(),
                    at(&ev),
                    ev.time_stamp(),
                );
            })
        };
        let cancel = window_event_listener(leptos::ev::pointercancel, move |ev| {
            step(
                gesture::Pointer::Cancel,
                ev.pointer_id(),
                at(&ev),
                ev.time_stamp(),
            );
        });
        watchers.set_value(vec![down, moved, up, cancel]);
    });

    // Writing, a finger that went down on the page would pan it instead: the page
    // lets a touch scroll it. While he writes, the page holds still.
    let inking = Memo::new(move |_| state.get() == mascot::State::Inking);
    let hold_page = StoredValue::new_local(None::<Closure<dyn Fn(web_sys::Event)>>);
    Effect::new(move |_| {
        let Some(window) = web_sys::window() else {
            return;
        };
        if let Some(block) = hold_page.try_update_value(Option::take).flatten() {
            let _ = window
                .remove_event_listener_with_callback("touchmove", block.as_ref().unchecked_ref());
        }
        if inking.get() {
            let block = Closure::<dyn Fn(web_sys::Event)>::new(|ev: web_sys::Event| {
                ev.prevent_default();
            });
            let options = web_sys::AddEventListenerOptions::new();
            options.set_passive(false);
            let _ = window.add_event_listener_with_callback_and_add_event_listener_options(
                "touchmove",
                block.as_ref().unchecked_ref(),
                &options,
            );
            hold_page.set_value(Some(block));
        }
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
            style:transform=move || look.get().transform(shown.get())
            style:--hop-squash=move || format!("{:.3}", squash.get())
            on:pointerdown={
                let nib_below = nib_below.clone();
                let other_finger = other_finger.clone();
                move |ev: leptos::ev::PointerEvent| {
                    ev.prevent_default();
                    if other_finger(&ev) {
                        return;
                    }
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
                    let (ox, oy) = shown.get_untracked();
                    *grab_state_down.borrow_mut() = Some((gx, gy));
                    // A fresh gesture: if this finger still holds one, its Up was lost.
                    *finger_down.borrow_mut() = gesture::Recognizer::new();
                    finger_down.borrow_mut().advance(gesture::Sample {
                        pointer: gesture::Pointer::Down(gesture::Target::Rambo),
                        at: (gx, gy),
                        t: ev.time_stamp(),
                        id: ev.pointer_id(),
                    });
                    set_state.set(after.clone());
                    if after == mascot::State::Dragging {
                        pen_pos.set(held_spot(&ev, (gx, gy)));
                    }
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
                        start_stroke((ox, oy + below), ev.time_stamp());
                    }
                    if let Some(el) = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                        let _ = el.set_pointer_capture(ev.pointer_id());
                    }
                }
            }
            on:pointermove={
                let nib_below = nib_below.clone();
                let other_finger = other_finger.clone();
                move |ev: leptos::ev::PointerEvent| {
                    let held = state.get_untracked();
                    if !matches!(held, mascot::State::Dragging | mascot::State::Inking)
                        || other_finger(&ev)
                    {
                        return;
                    }
                    *moved_move.borrow_mut() = true;
                    let (x, y) = (ev.client_x() as f64, ev.client_y() as f64);
                    finger_move.borrow_mut().advance(gesture::Sample {
                        pointer: gesture::Pointer::Move,
                        at: (x, y),
                        t: ev.time_stamp(),
                        id: ev.pointer_id(),
                    });
                    if let Some((gx, gy)) = *grab_state_move.borrow()
                        && !mascot::held_still((gx, gy), (x, y))
                    {
                        still_move.set(false);
                    }
                    pen_pos.set(match held {
                        // Writing, his nib is at the finger; carried, he rides above it.
                        mascot::State::Inking => (x, y),
                        _ => held_spot(&ev, (x, y)),
                    });
                    if held == mascot::State::Inking {
                        let below = nib_below.get();
                        let page = list_page();
                        strokes.update(|strokes| {
                            if let Some(stroke) = strokes.last_mut() {
                                let (x, y) = ink::on_surface(stroke.on, x, y + below, page);
                                stroke.samples.push(ink::Sample { x, y, t: ev.time_stamp() });
                            }
                        });
                    }
                }
            }
            on:pointerup={
                let other_finger = other_finger.clone();
                let release_on_page = release_on_page.clone();
                move |ev: leptos::ev::PointerEvent| {
                    if other_finger(&ev) {
                        return;
                    }
                    let (client_x, client_y) = (ev.client_x() as f64, ev.client_y() as f64);
                    // Let go mid-flick: thrown at the flick's speed.
                    let flick = match finger_up.borrow_mut().advance(gesture::Sample {
                        pointer: gesture::Pointer::Up,
                        at: (client_x, client_y),
                        t: ev.time_stamp(),
                        id: ev.pointer_id(),
                    }) {
                        Some(gesture::Gesture::Fling { vel }) => Some(vel),
                        _ => None,
                    };
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
                        if before == mascot::State::Dragging
                            && !was_listening
                            && let Some(form) = form_ref.get()
                        {
                            // Tap (no drag): submit the form (sends typed text) and go home
                            let _ = form.request_submit();
                        }
                        set_state.set(mascot::transition(&before, mascot::Event::Release));
                        return;
                    }
                    *grab_state_up.borrow_mut() = None;
                    // He lands where you saw him, above the finger.
                    pen_pos.set(match before {
                        mascot::State::Inking => (client_x, client_y),
                        _ => held_spot(&ev, (client_x, client_y)),
                    });

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
                    // Dropped on an empty planner hour, he notes a new task that starts then.
                    if before == mascot::State::Dragging
                        && let Some(at) = free_hour_under(
                            (client_x, client_y),
                            &snapshot.get_untracked(),
                            &view_settings.get_untracked(),
                        )
                    {
                        noting_at.set(Some(at));
                        set_state.set(mascot::transition(&before, mascot::Event::DropHour));
                        return;
                    }
                    // A listening Rambo dropped on a task row asks what to do with it.
                    if before == mascot::State::Dragging
                        && let Some(row) = row_under(client_x, client_y)
                    {
                        let task_id = row.get_attribute("data-id").unwrap_or_default();
                        let task_title = row.get_attribute("data-title").unwrap_or_default();
                        asked_on.set(mascot::Asked::Row);
                        set_state.set(mascot::transition(
                            &before,
                            mascot::Event::DropTask { task_id, title: task_title },
                        ));
                        return;
                    }
                    // Dropped on a slip in the planner, he asks about its time.
                    if before == mascot::State::Dragging
                        && let Some((id, title, repeats)) = crate::week::slip_at(client_x, client_y)
                    {
                        asked_on.set(mascot::Asked::Slip { repeats });
                        set_state.set(mascot::transition(
                            &before,
                            mascot::Event::DropTask { task_id: id.0, title },
                        ));
                        return;
                    }
                    release_on_page(flick);
                }
            }
            on:pointercancel={
                let release_on_page = release_on_page.clone();
                move |ev: leptos::ev::PointerEvent| {
                    if other_finger(&ev) {
                        return;
                    }
                    finger_cancel.borrow_mut().advance(gesture::Sample {
                        pointer: gesture::Pointer::Cancel,
                        at: (ev.client_x() as f64, ev.client_y() as f64),
                        t: ev.time_stamp(),
                        id: ev.pointer_id(),
                    });
                    match state.get_untracked() {
                        mascot::State::Dragging => {
                            hold_cancel.set(hold_cancel.get().wrapping_add(1));
                            stop_speech_cancel();
                            release_on_page(None);
                        }
                        mascot::State::Inking => {
                            // The browser took the gesture over: he lands where he
                            // is, like a drop on the page.
                            set_state.set(mascot::transition(
                                &state.get_untracked(),
                                mascot::Event::DropEmpty,
                            ));
                        }
                        _ => {}
                    }
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
            // His drawing comes in through <use>, not <img>: an <img> would load the file
            // as a document of its own, and he would follow the operating system
            // instead of the page. Every drawing is laid out on the same 64x112
            // page, fitted into his box (centred, never stretched) exactly as the <img>
            // was. FE-2.
            <svg class="rambo-drawing" viewBox="0 0 64 112" aria-hidden="true">
                <use href=move || match noticing.get() {
                    Some(face) => face.art(pen.get()),
                    None => mascot::art_for(pen.get(), &state.get(), &brain_pose.get(), typing.get()),
                } />
            </svg>
            // The mood's little picture, when the mood has one and he is home.
            {move || mascot::fx_for(&state.get(), &brain_pose.get(), typing.get()).map(|fx| view! {
                <svg class="rambo-fx" viewBox="0 0 48 48" aria-hidden="true">
                    <use href=fx.art() />
                </svg>
            })}
            // Speech bubble: inside the button so it follows Rambo everywhere
            <span class="mascot-status" aria-live="polite">
                {move || brain_speech.get().map(|line| view! { <span class="brain-line">{line}</span> })}
                {move || match asked.get() {
                    Some(title) => view! {
                        <span
                            class="ask"
                            class:below=move || bubble_below.get()
                            class:folding=move || folding.get()
                            class:tucked=move || tucked.get()
                            style:--ask-fold-ms=format!("{}ms", tuning::ASK_FOLD_MS)
                            style:--ask-below-by=move || format!("{:.1}px", rambo_height())
                            node_ref=bubble_ref
                            style:margin-left=move || format!("{:.1}px", bubble_shift.get())
                        >
                            <span class="ask-question">"What about "<q>{title.clone()}</q>"?"</span>
                            <span class="ask-choices">
                                {mascot::offered(asked_on.get_untracked()).iter().map(|&choice| view! {
                                    <button type="button" class="ask-choice" on:click=move |_| choose(choice)>
                                        {choice.label()}
                                    </button>
                                }).collect_view()}
                            </span>
                        </span>
                    }.into_any(),
                    None => view! { <span></span> }.into_any(),
                }}
                {move || talking.get().then(|| view! {
                    <span class="talking">"I'm listening."</span>
                })}
            </span>
        </button>
        // While he listens the light is turned down over everything but him.
        <div class="listening-dim" class:on=move || talking.get() aria-hidden="true"></div>
        // The nest's front rim, over his tip: a layer of its own above him, on the
        // nest's box wherever the page has carried it. Its back is drawn on the nest.
        <span
            class="nest-rim"
            node_ref=rim_ref
            aria-hidden="true"
            style:display=move || nest_at.get().is_none().then_some("none")
            style:left=move || format!("{:.1}px", nest_at.get().unwrap_or_default().0)
            style:top=move || format!("{:.1}px", nest_at.get().unwrap_or_default().1)
            style:width=move || format!("{:.1}px", nest_at.get().unwrap_or_default().2)
            style:height=move || format!("{:.1}px", nest_at.get().unwrap_or_default().3)
        >
            // The front rim, shown the same way as his drawing so the dish is
            // themed too. Fitted into the box (centred, not stretched), as the background was. FE-2.
            <svg viewBox="0 0 170 100">
                <use href="assets/nest/rim.svg#art" />
            </svg>
        </span>
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

/// The nest's box on screen, `(left, top, width, height)`: the one measurement his home
/// and the nest's front rim come from.
pub(crate) fn nest_box(nest_ref: NodeRef<leptos::html::Button>) -> Option<(f64, f64, f64, f64)> {
    let rect = nest_ref.get()?.get_bounding_client_rect();
    Some((rect.left(), rect.top(), rect.width(), rect.height()))
}

/// The list page's box in the window while it lies open at rest: ink laid on it goes
/// with the paper. None while a turn runs or the planner is the page showing.
fn list_page() -> Option<ink::PageBox> {
    let main = document().query_selector("main").ok().flatten()?;
    if main.class_list().contains("bending")
        || main.get_attribute("data-pane").as_deref() != Some("0")
    {
        return None;
    }
    let page = main.query_selector(".tasks-pane").ok().flatten()?;
    let rect = page.get_bounding_client_rect();
    // From inside the page's border, where its ink layer starts, and counting the rows
    // scrolled past inside it.
    Some(ink::PageBox {
        left: rect.left() + f64::from(page.client_left()),
        top: rect.top() + f64::from(page.client_top()),
        width: rect.width(),
        height: rect.height(),
        scroll_left: f64::from(page.scroll_left()),
        scroll_top: f64::from(page.scroll_top()),
    })
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
    // A phone's keyboard opening or closing moves the capture bar and his nest with it:
    // `--keyboard-inset` on the root (`capture.rs:ride_the_keyboard`). He watches the
    // inset itself rather than the events that set it, so whatever sets it, he measures
    // again once the bar has moved.
    if let Some(root) = window.document().and_then(|doc| doc.document_element()) {
        let after_layout = wasm_bindgen::closure::Closure::<dyn Fn()>::new(move || {
            crate::engine::next_frame(moved)
        });
        let watched = web_sys::MutationObserverInit::new();
        watched.set_attribute_filter(&js_sys::Array::of1(&"style".into()));
        if let Ok(observer) = web_sys::MutationObserver::new(after_layout.as_ref().unchecked_ref())
        {
            let _ = observer.observe_with_options(&root, &watched);
        }
        after_layout.forget();
    }
    // Rambo lives as long as the page, and so do his listeners.
    listener.forget();
    wasm_bindgen_futures::spawn_local(async move {
        crate::tasks::write::wait_for_fonts().await;
        moved();
    });
}

/// Calls `moved(scroller, delta, dt_ms)` whenever a page scrolls: `delta` is how far its
/// paper moved on screen (against the scroll) since that scroller's last scroll event,
/// and `dt_ms` how long that took. The scroller is `None` for the window.
fn on_paper_scrolled(moved: impl Fn(Option<web_sys::Element>, (f64, f64), f64) + 'static) {
    let Some(window) = web_sys::window() else {
        return;
    };
    type Seen = (Option<web_sys::Element>, f64, f64, f64);
    let last = std::cell::RefCell::new(None::<Seen>);
    let listener = Closure::<dyn Fn(web_sys::Event)>::new(move |ev: web_sys::Event| {
        let scroller = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        let (left, top) = match &scroller {
            Some(el) => (f64::from(el.scroll_left()), f64::from(el.scroll_top())),
            None => web_sys::window().map_or((0.0, 0.0), |w| {
                (w.scroll_x().unwrap_or(0.0), w.scroll_y().unwrap_or(0.0))
            }),
        };
        let at = ev.time_stamp();
        let seen = last.replace(Some((scroller.clone(), left, top, at)));
        if let Some((was, was_left, was_top, was_at)) = seen
            && was == scroller
        {
            moved(scroller, (was_left - left, was_top - top), at - was_at);
        }
    });
    // Scroll does not bubble; the capture phase hears every scroller.
    let _ = window.add_event_listener_with_callback_and_bool(
        "scroll",
        listener.as_ref().unchecked_ref(),
        true,
    );
    // Rambo lives as long as the page, and so does his listener.
    listener.forget();
}

/// Whether he lies on the paper of `scroller` (the window's when `None`) at `at`: the
/// first thing under him that is not him is inside it.
fn on_paper(scroller: Option<&web_sys::Element>, at: (f64, f64)) -> bool {
    let Some(scroller) = scroller else {
        return true;
    };
    document()
        .elements_from_point(at.0 as f32, at.1 as f32)
        .iter()
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .find(|el| el.closest(".rambo").ok().flatten().is_none())
        .is_some_and(|el| scroller.contains(Some(&el)))
}

/// A finger on the page while he is dazed, watched to see whether it holds.
struct Watch {
    finger: gesture::Recognizer,
    id: i32,
    /// What it went down on, which lets go of it if he takes it.
    on: web_sys::Element,
    at: (f64, f64),
    t: f64,
    tick: crate::engine::Handle,
}

/// What a finger went down on, as the recogniser names it.
fn target_of(on: &web_sys::Element) -> gesture::Target {
    match on
        .closest("li[data-id]")
        .ok()
        .flatten()
        .and_then(|row| row.get_attribute("data-id"))
    {
        Some(id) => gesture::Target::Row(id),
        None if list_page().is_some() => gesture::Target::Page(gesture::Leaf::List),
        None => gesture::Target::Page(gesture::Leaf::Planner),
    }
}

/// Tells whatever had finger `id`'s Down that the finger is gone, as the browser
/// does when it takes a finger for itself, so it lets go of its press.
fn let_go(on: &web_sys::Element, id: i32) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_pointer_id(id);
    if let Ok(cancel) = web_sys::PointerEvent::new_with_event_init_dict("pointercancel", &init) {
        let _ = on.dispatch_event(&cancel);
    }
}

/// The task row under a point, looking past Rambo himself: he is under the
/// pointer while he is carried, so the topmost element there is always him.
/// The empty planner hour under `(x, y)`, as the date phrase a task starting then is
/// sent with; None off the planner, over a placed task, or at a time the clock skips.
fn free_hour_under(
    (x, y): (f64, f64),
    snapshot: &Snapshot,
    settings: &settings::Settings,
) -> Option<crate::capture::NotedHour> {
    let slot = crate::week::hour_at(x, y, settings)?;
    let ctx = crate::week::planner_context(&snapshot.tz);
    let at = crate::week::drop_start(&ctx, slot.day, slot.minutes)?;
    crate::week::free_at(&snapshot.plan.placements, &ctx, slot).then(|| crate::capture::NotedHour {
        phrase: crate::week::hour_prefill_text(&ctx, slot.day, slot.minutes),
        at,
    })
}

fn row_under(x: f64, y: f64) -> Option<web_sys::Element> {
    let doc = web_sys::window()?.document()?;
    doc.elements_from_point(x as f32, y as f32)
        .iter()
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .filter(|el| el.closest(".rambo").ok().flatten().is_none())
        .find_map(|el| el.closest("li[data-id]").ok().flatten())
}

/// His height as laid out, before any transform.
fn rambo_height() -> f64 {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|doc| doc.query_selector(".rambo").ok().flatten())
        .and_then(|rambo| rambo.dyn_into::<web_sys::HtmlElement>().ok())
        .map_or(0.0, |rambo| f64::from(rambo.offset_height()))
}

/// Whether a bubble `height` tall would run off the top of the screen sitting on
/// something whose top is at `sits_on` (his status line, above him), keeping
/// `edge` pixels clear: then it opens below him instead.
fn opens_below(sits_on: f64, height: f64, edge: f64) -> bool {
    sits_on - height < edge
}

/// The box his middle stays in when thrown: the screen, without its scrollbar, less
/// half his size on every side.
fn screen_walls() -> crate::engine::Walls {
    let half = rambo_height() / 2.0;
    let (width, height) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|doc| doc.document_element())
        .map_or((0.0, 0.0), |root| {
            (
                f64::from(root.client_width()),
                f64::from(root.client_height()),
            )
        });
    crate::engine::Walls {
        left: half,
        top: half,
        right: (width - half).max(half),
        bottom: (height - half).max(half),
    }
}

/// The strokes a [`crate::engine::Splash`] lays in `pen` at `now`, on whatever lies
/// under it. A dot comes from his nib, below his middle.
fn ink_of(splash: crate::engine::Splash, pen: settings::Pen, now: f64) -> Vec<ink::Stroke> {
    let paths = match splash {
        crate::engine::Splash::Splat(strokes) => strokes,
        crate::engine::Splash::Dot((x, y)) => {
            let nib = y + rambo_height() * mascot::NIB_BELOW;
            vec![vec![(x, nib), (x + DOT_PX, nib)]]
        }
    };
    let page = list_page();
    paths
        .into_iter()
        .filter_map(|path| {
            let &(x0, y0) = path.first()?;
            let on = ink::surface_at(x0, y0, page);
            let last = path.len() - 1;
            let samples = path
                .into_iter()
                .enumerate()
                .map(|(i, (x, y))| {
                    let (x, y) = ink::on_surface(on, x, y, page);
                    // Laid over a few ms, so it has the speed of a quick flick of ink.
                    let t = now - (last - i) as f64 * SPLASH_SAMPLE_MS;
                    ink::Sample { x, y, t }
                })
                .collect();
            Some(ink::Stroke { samples, pen, on })
        })
        .collect()
}

/// How far a skitter dot's nib travels: just enough to lay one dab.
const DOT_PX: f64 = 1.5;
/// The time between the points of a splash's stroke, in ms.
const SPLASH_SAMPLE_MS: f64 = 8.0;

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
    use super::{bubble_offset, opens_below};

    /// Rambo and the nest's rim are fixed layers placed by measuring the nest, so they
    /// follow the keyboard only if he measures again when the inset changes. He watches
    /// the root's style, where the inset lives, so it holds whatever sets the inset; a
    /// screenshot that set it by hand once left him and the rim at the screen's foot.
    #[test]
    fn he_measures_again_whenever_the_keyboard_inset_changes() {
        // The code only, not these tests, which name what they look for.
        let code = |source: &'static str| source.split("#[cfg(test)]").next().unwrap_or(source);
        let rambo = code(include_str!("rambo.rs"));
        let capture = code(include_str!("capture.rs"));
        assert!(capture.contains(r#".set_property("--keyboard-inset""#));
        assert!(
            capture.contains(".document_element()"),
            "the inset is on the root"
        );
        let watch = rambo
            .split("MutationObserver::new(")
            .nth(1)
            .expect("rambo.rs watches the root with a MutationObserver");
        assert!(rambo.contains(r#"Array::of1(&"style".into())"#));
        assert!(watch.contains("observe_with_options(&root"));
    }

    #[test]
    fn a_bubble_without_room_above_him_opens_below() {
        // A 120px bubble, 8px from the top: room from his top at 128px down.
        assert!(!opens_below(400.0, 120.0, 8.0));
        assert!(!opens_below(128.0, 120.0, 8.0));
        assert!(opens_below(127.0, 120.0, 8.0));
        assert!(opens_below(-20.0, 120.0, 8.0));
        // Room above him, but not above the line he is saying: it opens below.
        let (his_top, line_h) = (150.0, 39.0);
        assert!(!opens_below(his_top, 120.0, 8.0));
        assert!(opens_below(his_top - line_h, 120.0, 8.0));
    }

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

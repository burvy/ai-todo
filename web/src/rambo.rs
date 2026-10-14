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
use crate::{gesture, ink, mascot, settings, speech, tuning, voice};

/// The single Rambo element: independent fixed layer above everything.
///
/// Rambo is not a child of the nest or the capture form. He sits at a
/// `position: fixed` spot measured from the nest's bounding box. Drag moves
/// only Rambo; he rides just above the pointer. Dropped on the page he is dazed
/// where he lands, writes when dragged again, and hops back to the nest.
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
                (
                    bubble_offset(shown.get().0, width, screen, tuning::BUBBLE_EDGE_PX),
                    opens_below(
                        his_top,
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
    let ticking = std::rc::Rc::new(std::cell::Cell::new(false));
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
            };
            let (brain, draw, at_rest) = {
                let mut world = world.borrow_mut();
                world.adopt(state.get_untracked());
                world.advance(tick.steps, &anchors);
                crate::engine::set_push(world.leaf_push());
                (world.rambo.brain.clone(), world.draw(), world.at_rest())
            };
            if brain != state.get_untracked() {
                set_state.set(brain);
            }
            if shown.get_untracked() != draw.pos {
                shown.set(draw.pos);
            }
            if squash.get_untracked() != draw.squash {
                squash.set(draw.squash);
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
    Effect::new(move |_| {
        state.track();
        pen_pos.track();
        wake();
    });

    // Let go on the page: dazed where he lands, after a splat if he was flung. The
    // world times the splat and the daze.
    let land_dazed = move |hard_drop: bool| {
        let event = if hard_drop && !mascot::prefers_reduced_motion() {
            mascot::Event::DropHard
        } else {
            mascot::Event::DropEmpty
        };
        set_state.set(mascot::transition(&state.get_untracked(), event));
    };
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
            // A stub until moving a task lands: he goes home.
            mascot::Choice::Move => chosen(),
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
        }
    };

    // While he asks, a tap anywhere off his bubble, on him too, or Esc is never mind
    // (`mascot::asking_dismissed_by`). The listeners live only as long as the ask,
    // and start after the drop that opened it, so that drop is never a tap.
    let ask_listeners =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::<WindowListenerHandle>::new()));
    Effect::new(move |_| {
        let asking = matches!(state.get(), mascot::State::Asking { .. });
        ask_listeners
            .borrow_mut()
            .drain(..)
            .for_each(WindowListenerHandle::remove);
        if !asking {
            return;
        }
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
                    on_bubble.set(within(&ev, ".ask"));
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
            style:--pen-x=move || format!("{:.1}px", shown.get().0)
            style:--pen-y=move || format!("{:.1}px", shown.get().1)
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
                move |ev: leptos::ev::PointerEvent| {
                    if other_finger(&ev) {
                        return;
                    }
                    let (client_x, client_y) = (ev.client_x() as f64, ev.client_y() as f64);
                    // Let go mid-flick: a hard drop, and he splats.
                    let hard_drop = matches!(
                        finger_up.borrow_mut().advance(gesture::Sample {
                            pointer: gesture::Pointer::Up,
                            at: (client_x, client_y),
                            t: ev.time_stamp(),
                            id: ev.pointer_id(),
                        }),
                        Some(gesture::Gesture::Fling { .. })
                    );
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
                            land_dazed(false);
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
            <img
                src=move || match noticing.get() {
                    Some(face) => face.art(pen.get()),
                    None => mascot::art_for(pen.get(), &state.get(), &brain_pose.get(), typing.get()),
                }
                alt="Rambo"
            />
            // The mood's little picture, when the mood has one and he is home.
            {move || mascot::fx_for(&state.get(), &brain_pose.get(), typing.get()).map(|fx| view! {
                <img class="rambo-fx" src=fx.art() alt="" aria-hidden="true" />
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
                                {mascot::Choice::ALL.map(|choice| view! {
                                    <button type="button" class="ask-choice" on:click=move |_| choose(choice)>
                                        {choice.label()}
                                    </button>
                                })}
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
        // The nest's front rim, over his tip: a layer of its own above him, on the
        // nest's box wherever the page has carried it. Its back is drawn on the nest.
        <span
            class="nest-rim"
            aria-hidden="true"
            style:display=move || nest_at.get().is_none().then_some("none")
            style:left=move || format!("{:.1}px", nest_at.get().unwrap_or_default().0)
            style:top=move || format!("{:.1}px", nest_at.get().unwrap_or_default().1)
            style:width=move || format!("{:.1}px", nest_at.get().unwrap_or_default().2)
            style:height=move || format!("{:.1}px", nest_at.get().unwrap_or_default().3)
        ></span>
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
    // Rambo lives as long as the page, and so do his listeners.
    listener.forget();
    wasm_bindgen_futures::spawn_local(async move {
        crate::tasks::write::wait_for_fonts().await;
        moved();
    });
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

/// Whether a bubble `height` tall would run off the top of the screen above a
/// Rambo whose top is at `his_top`, keeping `edge` pixels clear: then it opens
/// below him instead.
fn opens_below(his_top: f64, height: f64, edge: f64) -> bool {
    his_top - height < edge
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
    use super::{bubble_offset, opens_below};

    #[test]
    fn a_bubble_without_room_above_him_opens_below() {
        // A 120px bubble, 8px from the top: room from his top at 128px down.
        assert!(!opens_below(400.0, 120.0, 8.0));
        assert!(!opens_below(128.0, 120.0, 8.0));
        assert!(opens_below(127.0, 120.0, 8.0));
        assert!(opens_below(-20.0, 120.0, 8.0));
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

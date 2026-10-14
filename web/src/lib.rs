use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use todo_core::{Snapshot, TaskId};
use wasm_bindgen::JsCast;

pub mod account;
pub mod api;
pub mod brain;
pub mod clock;
pub mod ink;
pub mod inklayer;
pub mod mascot;
pub mod pen;
pub mod row;
pub mod settings;
pub mod sound;
pub mod speech;
pub mod tack;
pub mod voice;
pub mod week;

mod capture;
mod edits;
mod pane;
mod rambo;
mod tasks;

use crate::capture::CaptureBox;
use crate::pane::show_pane;
#[cfg(target_arch = "wasm32")]
use crate::pane::{SETTLE_MS, curl_for, nearest_pane};
use crate::rambo::Rambo;
use crate::tasks::TaskList;

/// The list before the server has answered: empty, never the demo fixture, whose
/// made-up tasks would read as the user's own.
fn empty_snapshot() -> Snapshot {
    Snapshot {
        tasks: Vec::new(),
        plan: todo_core::Plan::default(),
        series: Vec::new(),
        ..api::fixture_snapshot()
    }
}

#[component]
pub fn App() -> impl IntoView {
    let (snapshot, set_snapshot) = signal(empty_snapshot());
    let (view_settings, set_view_settings) = signal(settings::load());
    let (error, set_error) = signal::<Option<String>>(None);
    let (brain_speech, set_brain_speech) = signal::<Option<String>>(None);
    let (brain_pose, set_brain_pose) = signal("home".to_owned());
    let (brain_event, set_brain_event) = signal::<Option<rambl_brain::Event>>(None);
    // The tack board reports what just happened on it — a task cut out and
    // tacked, a tidy-up, a crumple. Rambo's poses for those land with Funny's
    // mood work (docs/DESIGN-DIRECTION.md, "Rambo styles"), so for now the
    // signal is one the board writes and this effect clears: a subscriber
    // mounted between the two gets the cue exactly once.
    let (tack_cue, set_tack_cue) = signal::<Option<tack::Cue>>(None);
    Effect::new(move |_| {
        if tack_cue.get().is_some() {
            set_tack_cue.set(None);
        }
    });

    // Load snapshot from API on mount
    Effect::new(move |_| {
        wasm_bindgen_futures::spawn_local(async move {
            match api::fetch_snapshot(&api::today_date()).await {
                Ok(loaded) => set_snapshot.set(loaded),
                Err(error) => set_error.set(Some(format!("Could not load your tasks: {error}"))),
            }
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
    let (typing, set_typing) = signal(false);
    let pen_pos = RwSignal::new((0.0_f64, 0.0));
    // What a dazed Rambo has written on the page, one path per drag.
    let strokes = RwSignal::new(Vec::<ink::Path>::new());
    // Transcript buffer: Rambo fills it while listening, CaptureBox reads it
    let capture_text = RwSignal::new(String::new());
    // Pen queue: task IDs the client just created, so the pen writes only those rows
    let pen_queue = RwSignal::new(Vec::<TaskId>::new());
    // Capture form ref so Rambo can requestSubmit on release
    let form_ref = NodeRef::<leptos::html::Form>::new();
    // Nest ref so Rambo (independent fixed layer) can measure where to sit at home
    let nest_ref = NodeRef::<leptos::html::Button>::new();

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
                nest_ref
                set_snapshot=set_snapshot
                state=mascot_state
                set_state=set_mascot_state
                view_settings
                pen_pos
                capture_text
                pen_queue
                set_typing
                set_error
            />
        </header>
        {move || error.get().map(|e| view! {
            <div class="error page-note" aria-live="polite">{e}</div>
        })}
        <brain::Controller snapshot mode=Signal::derive(move || view_settings.get().mode) event=brain_event set_event=set_brain_event audible=Signal::derive(move || voice::audible(view_settings.get().sounds, view_settings.get().rambo_talks)) set_speech=set_brain_speech set_pose=set_brain_pose />
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
                <tack::TackBoard snapshot set_snapshot set_error view_settings set_cue=set_tack_cue />
            </div>
            <week::WeekGrid
                snapshot=snapshot
                settings=view_settings
                set_snapshot=set_snapshot
                set_error=set_error
                capture_text
            />
        </main>
        // The ink a dazed Rambo leaves: above the page, below him.
        <inklayer::InkLayer
            dots=Signal::derive(move || {
                let pen = view_settings.get().pen;
                strokes.with(|strokes| {
                    strokes.iter().flat_map(|stroke| inklayer::trail_of(stroke, pen)).collect()
                })
            })
            pen=Signal::derive(move || view_settings.get().pen)
        />
        // Rambo: independent fixed layer above everything, not part of the capture form
        <Rambo
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
            pen_pos
            capture_text
            form_ref
            nest_ref
            pen=Signal::derive(move || view_settings.get().pen)
            typing
            strokes
        />
    }
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

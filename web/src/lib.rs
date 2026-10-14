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
pub mod pickers;
pub mod row;
pub mod settings;
pub mod sound;
pub mod speech;
pub mod tack;
pub mod tuning;
pub mod voice;
pub mod week;

#[cfg(any(target_arch = "wasm32", test))]
mod bend;
mod capture;
mod edits;
mod pane;
mod rambo;
mod scrap;
mod tasks;
mod turner;

use crate::capture::CaptureBox;
#[cfg(target_arch = "wasm32")]
use crate::pane::nearest_pane;
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
    // The page turn (bend.rs, turner.rs): the page bends from the finger's corner. The
    // browser's own scroll still moves `main` when a pane is shown, and `track` keeps the
    // pane switch in step with it.
    let fx_ref = NodeRef::<leptos::html::Div>::new();
    let turner = turner::Turner::new(main_ref, fx_ref, set_pane);
    #[cfg(target_arch = "wasm32")]
    let track = {
        let turner = turner.clone();
        move |_: leptos::ev::Event| {
            // A turn scrolls `main` itself to stack the panes; that is not a switch.
            if turner.turning() {
                return;
            }
            if let Some(element) = main_ref.get() {
                set_pane.set(nearest_pane(&element));
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    let track = |_: leptos::ev::Event| {};
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
    // A row Rambo was asked to edit: the task list opens its edit sheet
    let edit_request = RwSignal::new(None::<TaskId>);
    // The task whose edit sheet is open: the list owns it, Rambo listens while it is
    let open_sheet = RwSignal::new(None::<TaskId>);
    // The scrap a swiped-out row leaves in the corner, to tape it back
    let scraps = RwSignal::new(Vec::<scrap::Scrap>::new());
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
        <div class="capture-slot">
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
        </div>
        {move || error.get().map(|e| view! {
            <div class="error page-note" aria-live="polite">{e}</div>
        })}
        <brain::Controller snapshot mode=Signal::derive(move || view_settings.get().mode) event=brain_event set_event=set_brain_event audible=Signal::derive(move || voice::audible(view_settings.get().sounds, view_settings.get().rambo_talks)) quiet=Signal::derive(move || mascot::speaks_later(&mascot_state.get()) || pen_queue.with(|queue| !queue.is_empty())) set_speech=set_brain_speech set_pose=set_brain_pose />
        <main
                node_ref=main_ref
                data-pane=move || pane.get().to_string()
                on:scroll=track
                on:pointerdown={
                    let turner = turner.clone();
                    move |ev| turner.down(&ev)
                }
                on:pointermove={
                    let turner = turner.clone();
                    move |ev| turner.moved(&ev)
                }
                on:pointerup={
                    let turner = turner.clone();
                    move |ev| turner.up(&ev)
                }
                on:pointercancel={
                    let turner = turner.clone();
                    move |ev| turner.cancel(&ev)
                }
            >
            <div class="tasks-pane">
                <TaskList snapshot set_snapshot set_error view_settings mascot_state set_mascot_state pen_queue pen_pos edit_request open_sheet scraps />
                <tack::TackBoard snapshot set_snapshot set_error view_settings set_cue=set_tack_cue />
                <PageFoot
                    page=0
                    turn=std::rc::Rc::new({
                        let turner = turner.clone();
                        move || turner.turn_to(1)
                    })
                />
            </div>
            <week::WeekGrid
                snapshot=snapshot
                settings=view_settings
                set_settings=set_view_settings
                set_snapshot=set_snapshot
                set_error=set_error
                capture_text
                turn_back=std::rc::Rc::new({
                    let turner = turner.clone();
                    move || turner.turn_to(0)
                })
            />
            // What a turning page draws between the two panes: the shadow it casts on
            // the next page, the curl's rise, and the lifted flap with its own shadow. Hidden until a turn.
            <div class="bend-fx" node_ref=fx_ref aria-hidden="true">
                <div class="bend-away"><i class="bend-strip bend-cast"></i></div>
                <div class="bend-kept"><i class="bend-strip bend-rise"></i></div>
                <div class="bend-shadow bend-shadow-far"></div>
                <div class="bend-shadow bend-shadow-near"></div>
                <div class="bend-flap"><i class="bend-strip bend-ridge"></i></div>
            </div>
        </main>
        <scrap::ScrapCorner scraps set_snapshot set_error />
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
            edit_request
            open_sheet
            nest_ref
            scraps
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

/// The note at the foot of a phone's page, in the page itself so it turns with it: it says
/// where the other page is, and tapping it turns there. A desk shows both pages, so it hides.
#[component]
pub(crate) fn PageFoot(page: u8, turn: std::rc::Rc<dyn Fn()>) -> impl IntoView {
    view! {
        <button type="button" class="page-foot" on:click=move |_| turn()>
            {page_foot_note(page)}
        </button>
    }
}

/// What the note at a phone page's foot says: where the other page is.
fn page_foot_note(pane: u8) -> &'static str {
    match pane {
        0 => "planner on the next page \u{2192}",
        _ => "\u{2190} task list on the previous page",
    }
}

#[cfg(test)]
mod tests {
    use super::page_foot_note;

    #[test]
    fn each_page_names_the_other_one() {
        assert_eq!(page_foot_note(0), "planner on the next page \u{2192}");
        assert_eq!(page_foot_note(1), "\u{2190} task list on the previous page");
    }
}

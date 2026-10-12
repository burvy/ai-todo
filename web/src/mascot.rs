//! Rambo the pen: his states, and his body on the move.
//!
//! [`State`] and [`transition`] are the pure state machine — unit-tested
//! without the DOM. [`PenRig`] is the body the view moves: a fixed layer
//! that flies between his circle, the capture caret and fresh task rows.

use leptos::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// On his circle next to the capture box.
    Home,
    /// Picked up: listening, and ready to be dragged.
    Dragging,
    /// Dropped on empty space: sits there a moment, dazed.
    Recovering,
    /// Running home in little hops.
    Skittering,
    /// Dropped on a task: asking what to do with it.
    Asking { task_id: String, title: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Pointer down on him (or Space held): he starts listening.
    Grab,
    /// Released without moving: capture what he heard, back to his circle.
    Release,
    /// Dropped on empty space.
    DropEmpty,
    /// Dropped on a task row or planner block.
    DropTask { task_id: String, title: String },
    /// The recovering pause is over (he skitters), or the skitter ends (home).
    Return,
    /// He chose an action on the task he was dropped on.
    Choose,
    /// Hold-to-talk from wherever he is.
    Talk,
}

/// Milliseconds he sits dazed where he was dropped.
pub const RECOVER_MS: u32 = 600;
/// Milliseconds his hop home takes.
pub const SKITTER_MS: u32 = 900;

pub fn transition(state: &State, event: Event) -> State {
    match (state, event) {
        (State::Home, Event::Grab | Event::Talk) => State::Dragging,
        (State::Dragging, Event::Release) => State::Home,
        (State::Dragging, Event::DropEmpty) => State::Recovering,
        (State::Dragging, Event::DropTask { task_id, title }) => State::Asking { task_id, title },
        (State::Recovering, Event::Return) => State::Skittering,
        (State::Skittering, Event::Return) => State::Home,
        (State::Asking { .. }, Event::Choose | Event::Return) => State::Home,
        (state, Event::Talk) => state.clone(),
        (state, _) => state.clone(),
    }
}

/// Is he away from his circle (so the circle's own pen should hide)?
pub fn is_away(state: &State) -> bool {
    !matches!(state, State::Home)
}

/// Where a hop of the skitter run is at `progress` (0..=1): straight line from
/// `start` to `home`, with little bounces above it. Pure, so the DOM layer
/// only reads points off it.
pub fn skitter_point(
    start: (f64, f64),
    home: (f64, f64),
    hops: u32,
    hop_height: f64,
    progress: f32,
) -> (f64, f64) {
    let progress = f64::from(progress.clamp(0.0, 1.0));
    let x = start.0 + (home.0 - start.0) * progress;
    let y = start.1 + (home.1 - start.1) * progress
        - (progress * f64::from(hops) * std::f64::consts::PI)
            .sin()
            .abs()
            * hop_height;
    (x, y)
}

#[cfg(target_arch = "wasm32")]
pub fn prefers_reduced_motion() -> bool {
    leptos::prelude::window()
        .match_media("(prefers-reduced-motion: reduce)")
        .ok()
        .flatten()
        .is_some_and(|query| query.matches())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn prefers_reduced_motion() -> bool {
    false
}

/// The writing pen's trip. While he writes — with you at the capture caret,
/// or across fresh task rows — the view shows him through these phases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenFlight {
    /// Hidden on his circle; the circle's own pen shows.
    Parked,
    /// Writing along at the capture caret.
    WritingCaret,
    /// Flying to the fresh row `index` of `total`.
    FlyingToRow { index: u32, total: u32 },
    /// Writing that row's title, nib sweeping left to right.
    WritingRow { index: u32, total: u32 },
    /// Heading back to his circle.
    FlyingHome,
}

/// True while a row batch owns the pen (so typing must not yank him away).
pub fn pen_busy(flight: &PenFlight) -> bool {
    matches!(
        flight,
        PenFlight::FlyingToRow { .. } | PenFlight::WritingRow { .. }
    )
}

/// The class the rig carries for a flight. The CSS block in index.html owns
/// what each class looks like.
pub fn pen_class(flight: &PenFlight) -> &'static str {
    match flight {
        PenFlight::Parked => "pen-rig",
        PenFlight::WritingCaret => "pen-rig is-away is-caret",
        PenFlight::FlyingToRow { .. } => "pen-rig is-away is-flying",
        PenFlight::WritingRow { .. } => "pen-rig is-away is-writing",
        PenFlight::FlyingHome => "pen-rig is-away is-flying",
    }
}

/// Rambo's travelling body: a pen-sized image on a fixed layer, placed with
/// CSS variables and posed by the flight's class. Hidden while parked — his
/// circle in the capture row carries the resting pen.
#[component]
pub fn PenRig(
    flight: RwSignal<PenFlight>,
    pos: RwSignal<(f64, f64)>,
    pose: ReadSignal<String>,
) -> impl IntoView {
    view! {
        <div
            class=move || {
                let pose = pose.get();
                if pose.is_empty() { pen_class(&flight.get()).to_owned() }
                else { format!("{} pose-{pose}", pen_class(&flight.get())) }
            }
            style:--pen-x=move || format!("{:.1}px", pos.get().0)
            style:--pen-y=move || format!("{:.1}px", pos.get().1)
            aria-hidden="true"
        >
            <img class="pen-rig-pen" src="assets/pen.svg" alt="" />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grab_listens_and_release_goes_home() {
        let state = transition(&State::Home, Event::Grab);
        assert_eq!(state, State::Dragging);
        assert_eq!(transition(&state, Event::Release), State::Home);
    }

    #[test]
    fn empty_drop_recovers_then_skitters_home() {
        let state = transition(&State::Home, Event::Grab);
        let state = transition(&state, Event::DropEmpty);
        assert_eq!(state, State::Recovering);
        let state = transition(&state, Event::Return);
        assert_eq!(state, State::Skittering);
        assert_eq!(transition(&state, Event::Return), State::Home);
    }

    #[test]
    fn task_drop_preserves_target_then_choice_returns_home() {
        let state = transition(
            &State::Dragging,
            Event::DropTask {
                task_id: "t1".into(),
                title: "Lab report".into(),
            },
        );
        assert_eq!(
            state,
            State::Asking {
                task_id: "t1".into(),
                title: "Lab report".into()
            }
        );
        assert_eq!(transition(&state, Event::Choose), State::Home);
        assert_eq!(transition(&state, Event::Return), State::Home);
    }

    #[test]
    fn talk_is_available_while_away() {
        let state = State::Dragging;
        assert_eq!(transition(&state, Event::Talk), state);
    }

    #[test]
    fn the_skitter_hops_above_the_straight_line() {
        let (x, y) = skitter_point((0.0, 100.0), (100.0, 100.0), 3, 20.0, 0.5);
        assert_eq!(x, 50.0);
        // Half way through 3 hops: 1.5 hops in, so the bounce is at its peak.
        assert!((y - 80.0).abs() < 1e-9, "{y}");
        let (x, y) = skitter_point((0.0, 100.0), (100.0, 100.0), 3, 20.0, 0.0);
        assert_eq!((x, y), (0.0, 100.0));
        let (_, y) = skitter_point((0.0, 100.0), (100.0, 100.0), 3, 20.0, 1.0);
        assert!((y - 100.0).abs() < 1e-9, "{y}");
    }

    #[test]
    fn the_pen_is_busy_only_for_row_batches() {
        assert!(!pen_busy(&PenFlight::Parked));
        assert!(!pen_busy(&PenFlight::WritingCaret));
        assert!(pen_busy(&PenFlight::WritingRow { index: 0, total: 2 }));
        assert!(pen_busy(&PenFlight::FlyingToRow { index: 1, total: 2 }));
    }

    #[test]
    fn pen_classes_cover_every_flight() {
        assert_eq!(pen_class(&PenFlight::Parked), "pen-rig");
        assert!(pen_class(&PenFlight::WritingCaret).contains("is-caret"));
        assert!(pen_class(&PenFlight::FlyingHome).contains("is-flying"));
        assert!(pen_class(&PenFlight::WritingRow { index: 0, total: 1 }).contains("is-writing"));
    }
}

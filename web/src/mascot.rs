//! Rambo the pen: one mascot, one state machine, one DOM element.
//!
//! [`State`] and [`transition`] are the pure state machine — unit-tested
//! without the DOM. The single element is a fixed-position button at the
//! bottom-right corner; CSS classes and CSS variables move it for dragging,
//! writing at the caret, writing task rows, and skittering back home.
//!
//! Writing flight is part of the same state machine: when Rambo writes,
//! he leaves home and returns. The view layer reads the state and places
//! the element accordingly.

/// Rambo's complete state: where he is and what he's doing.
///
/// One element, one state — the element is always visible at home unless
/// it is Dragging, Recovering, Skittering, Asking, WritingCaret,
/// WritingRow, or Flying.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Resting at his home spot (bottom-right corner).
    Home,
    /// Picked up: listening, being dragged with the pointer.
    Dragging,
    /// Dropped on empty space: sits there a moment, dazed.
    Recovering,
    /// Dropped hard (fast release): ink splat + wobble, then recover.
    Splatting,
    /// Running home in little hops.
    Skittering,
    /// Dropped on a task: asking what to do with it.
    Asking { task_id: String, title: String },
    /// Flying to the capture caret or a task row (transit, not writing yet).
    Flying,
    /// Writing along at the capture caret while the user types.
    WritingCaret,
    /// Writing a fresh task row's title, nib sweeping left to right.
    WritingRow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Pointer down on him (or Space/Enter): he starts listening.
    Grab,
    /// Released without moving: capture what he heard, back to home.
    Release,
    /// Dropped on empty space.
    DropEmpty,
    /// Dropped hard on empty space (fast release — triggers splat).
    DropHard,
    /// Dropped on a task row or planner block.
    DropTask { task_id: String, title: String },
    /// The recovering pause is over (he skitters), or the skitter ends (home).
    Return,
    /// He chose an action on the task he was dropped on.
    Choose,
    /// Hold-to-talk from wherever he is.
    Talk,
    /// Start writing at the caret (user is typing).
    WriteCaret,
    /// Fly to a fresh task row and write it.
    WriteRow,
    /// Finish writing and fly home.
    FlyHome,
}

/// Milliseconds he sits dazed where he was dropped.
pub const RECOVER_MS: u32 = 600;
/// Milliseconds his hop home takes.
pub const SKITTER_MS: u32 = 900;
/// Milliseconds the splat wobble lasts on a hard drop.
pub const SPLAT_MS: u32 = 400;
/// Minimum drop velocity (px/ms) to trigger a hard-drop splat.
pub const HARD_DROP_VELOCITY: f64 = 0.5;

pub fn transition(state: &State, event: Event) -> State {
    match (state, event) {
        // From home: grab or talk starts dragging; writing starts flying.
        (State::Home, Event::Grab | Event::Talk) => State::Dragging,
        (State::Home, Event::WriteCaret) => State::WritingCaret,
        (State::Home, Event::WriteRow) => State::Flying,
        (State::Home, Event::FlyHome) => State::Home,

        // Dragging: release goes home; drop on task asks; drop empty recovers;
        // drop hard splats.
        (State::Dragging, Event::Release) => State::Home,
        (State::Dragging, Event::DropEmpty) => State::Recovering,
        (State::Dragging, Event::DropHard) => State::Splatting,
        (State::Dragging, Event::DropTask { task_id, title }) => State::Asking { task_id, title },

        // Splatting → Recovering → Skittering → Home.
        (State::Splatting, Event::Return) => State::Recovering,
        (State::Recovering, Event::Return) => State::Skittering,
        (State::Skittering, Event::Return) => State::Home,

        // Asking: choose or return goes home.
        (State::Asking { .. }, Event::Choose | Event::Return) => State::Home,

        // Writing caret: fly home when done, or fly to a row (after Add).
        (State::WritingCaret, Event::FlyHome) => State::Flying,
        (State::WritingCaret, Event::WriteRow) => State::Flying,

        // Flying to a row: arrive and write.
        (State::Flying, Event::WriteRow) => State::WritingRow,

        // Writing row: fly home when done.
        (State::WritingRow, Event::FlyHome) => State::Flying,

        // Flying home: arrive.
        (State::Flying, Event::FlyHome) => State::Home,

        // Talk is available while away (no state change).
        (state, Event::Talk) => state.clone(),

        // Default: stay.
        (state, _) => state.clone(),
    }
}

/// Is he away from home (not resting at his circle)?
pub fn is_away(state: &State) -> bool {
    !matches!(state, State::Home)
}

/// True if the state is one where Rambo is physically away from his nest
/// (dragging, dropped somewhere, skittering, writing, or asking).
pub fn is_off_nest(state: &State) -> bool {
    matches!(
        state,
        State::Dragging
            | State::Recovering
            | State::Splatting
            | State::Skittering
            | State::Asking { .. }
            | State::Flying
            | State::WritingCaret
            | State::WritingRow
    )
}

/// Is he writing or flying for writing (so typing must not yank him)?
pub fn pen_busy(state: &State) -> bool {
    matches!(
        state,
        State::Flying | State::WritingCaret | State::WritingRow
    )
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

/// The CSS class the single Rambo element carries for its current state.
/// The CSS block in theme.css (marked "Rambo") owns what each class looks like.
pub fn rambo_class(state: &State, pose: &str) -> String {
    let base = "rambo";
    let extra = match state {
        State::Home => "",
        State::Dragging => " is-dragging",
        State::Recovering => " is-recovering",
        State::Splatting => " is-splatting",
        State::Skittering => " is-skittering",
        State::Asking { .. } => " is-asking",
        State::Flying => " is-flying",
        State::WritingCaret => " is-flying is-writing is-caret",
        State::WritingRow => " is-flying is-writing",
    };
    if pose.is_empty() {
        format!("{base}{extra}")
    } else {
        format!("{base}{extra} pose-{pose}")
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
    fn hard_drop_splats_then_recovers_and_skitters_home() {
        let state = transition(&State::Home, Event::Grab);
        let state = transition(&state, Event::DropHard);
        assert_eq!(state, State::Splatting);
        let state = transition(&state, Event::Return);
        assert_eq!(state, State::Recovering);
        let state = transition(&state, Event::Return);
        assert_eq!(state, State::Skittering);
        assert_eq!(transition(&state, Event::Return), State::Home);
    }

    #[test]
    fn splatting_is_off_nest() {
        assert!(is_off_nest(&State::Splatting));
        assert!(is_off_nest(&State::Dragging));
        assert!(!is_off_nest(&State::Home));
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
    fn writing_caret_then_fly_home() {
        let state = transition(&State::Home, Event::WriteCaret);
        assert_eq!(state, State::WritingCaret);
        let state = transition(&state, Event::FlyHome);
        assert_eq!(state, State::Flying);
        assert_eq!(transition(&state, Event::FlyHome), State::Home);
    }

    #[test]
    fn writing_row_fly_through_flying_state() {
        let state = transition(&State::Home, Event::WriteRow);
        assert_eq!(state, State::Flying);
        let state = transition(&state, Event::WriteRow);
        assert_eq!(state, State::WritingRow);
        let state = transition(&state, Event::FlyHome);
        assert_eq!(state, State::Flying);
        assert_eq!(transition(&state, Event::FlyHome), State::Home);
    }

    #[test]
    fn pen_busy_during_writing_flight() {
        assert!(!pen_busy(&State::Home));
        assert!(!pen_busy(&State::Dragging));
        assert!(pen_busy(&State::Flying));
        assert!(pen_busy(&State::WritingCaret));
        assert!(pen_busy(&State::WritingRow));
    }

    #[test]
    fn rambo_classes_cover_every_state() {
        assert_eq!(rambo_class(&State::Home, ""), "rambo");
        assert!(rambo_class(&State::Dragging, "").contains("is-dragging"));
        assert!(rambo_class(&State::Splatting, "").contains("is-splatting"));
        assert!(rambo_class(&State::Recovering, "").contains("is-recovering"));
        assert!(rambo_class(&State::Skittering, "").contains("is-skittering"));
        assert!(rambo_class(&State::Flying, "").contains("is-flying"));
        assert!(rambo_class(&State::WritingCaret, "").contains("is-caret"));
        assert!(rambo_class(&State::WritingRow, "").contains("is-writing"));
        assert!(rambo_class(&State::Home, "listening").contains("pose-listening"));
    }

    #[test]
    fn is_away_returns_true_for_non_home_states() {
        assert!(!is_away(&State::Home));
        assert!(is_away(&State::Dragging));
        assert!(is_away(&State::WritingCaret));
        assert!(is_away(&State::Flying));
    }

    #[test]
    fn submit_while_writing_caret_writes_row_not_stuck() {
        // When the user presses Add while Rambo is at the caret, the pen queue
        // starts writing the new row directly from WritingCaret. He must never
        // stay stuck in WritingCaret.
        let state = transition(&State::WritingCaret, Event::WriteRow);
        assert_eq!(state, State::Flying);
        // After flying to the row, he writes it
        let state = State::WritingRow;
        // When done, fly home
        let state = transition(&state, Event::FlyHome);
        assert_eq!(state, State::Flying);
        // And arrives home
        let state = State::Home;
        assert_eq!(state, State::Home);
    }
}

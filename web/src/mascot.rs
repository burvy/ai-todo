#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Home,
    Dragging,
    Falling,
    Returning,
    Asking { task_id: String, title: String },
    // Clean design: writing pen states (not messy v1 copy).
    Writing { title: String, slot_index: u32 },
    Linger,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Grab,
    DropEmpty,
    DropTask { task_id: String, title: String },
    Choose,
    Talk,
    Return,
    // Writing pen events (v1 idea, clean Leptos design).
    TypeFollow,
    WriteTitle { title: String },
    ReturnHome,
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

pub fn transition(state: &State, event: Event) -> State {
    match (state, event) {
        (State::Home, Event::Grab) => State::Dragging,
        (State::Dragging, Event::DropEmpty) => State::Falling,
        (State::Dragging, Event::DropTask { task_id, title }) => State::Asking { task_id, title },
        (State::Falling, Event::Return) => State::Returning,
        (State::Returning, Event::Return | Event::Choose) => State::Home,
        (State::Asking { .. }, Event::Choose | Event::Return) => State::Home,
        (state, Event::Talk) => state.clone(),
        // Clean pen design: reduced-motion skips animation; write flows through slots.
        (State::Writing { .. }, Event::WriteTitle { .. }) => state.clone(),
        (State::Linger, Event::TypeFollow) => State::Linger,
        (State::Linger, Event::WriteTitle { title }) => State::Writing {
            title,
            slot_index: 0,
        },
        // type-follow starts writing only when not reduced-motion; else stay home.
        (State::Home, Event::TypeFollow) if prefers_reduced_motion() => State::Home,
        (State::Home, Event::TypeFollow) => State::Writing {
            title: String::new(),
            slot_index: 0,
        },
        (state, _) => state.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_drop_falls_then_returns() {
        let state = transition(&State::Home, Event::Grab);
        let state = transition(&state, Event::DropEmpty);
        let state = transition(&state, Event::Return);
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
    }

    #[test]
    fn talk_is_available_while_away() {
        let state = State::Dragging;
        assert_eq!(transition(&state, Event::Talk), state);
    }
}

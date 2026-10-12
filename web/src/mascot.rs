#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Home,
    Dragging,
    Falling,
    Returning,
    Asking { task_id: String, title: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Grab,
    DropEmpty,
    DropTask { task_id: String, title: String },
    Choose,
    Talk,
    Return,
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

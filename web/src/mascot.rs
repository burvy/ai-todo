#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Home,
    Dragging,
    Falling,
    Returning,
    Asking(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Grab,
    DropEmpty,
    DropTask,
    Choose,
    Talk,
    Return,
}

pub fn transition(state: &State, action: Action) -> State {
    match (state, action) {
        (State::Home, Action::Grab) => State::Dragging,
        (State::Home, Action::Talk) => State::Home,
        (State::Dragging, Action::DropEmpty) => State::Falling,
        (State::Dragging, Action::DropTask) => State::Asking("task".into()),
        (State::Dragging, Action::Talk) => State::Dragging,
        (State::Falling, Action::Return) => State::Returning,
        (State::Returning, Action::Return | Action::Choose) => State::Home,
        (State::Asking(_), Action::Choose | Action::Return) => State::Home,
        (state, Action::Talk) => state.clone(),
        (state, _) => state.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_drop_falls_then_returns_home() {
        let dragging = transition(&State::Home, Action::Grab);
        let falling = transition(&dragging, Action::DropEmpty);
        let returning = transition(&falling, Action::Return);
        assert_eq!(returning, State::Returning);
        assert_eq!(transition(&returning, Action::Return), State::Home);
    }

    #[test]
    fn task_drop_asks_and_any_choice_returns_home() {
        let asking = transition(&State::Dragging, Action::DropTask);
        assert_eq!(asking, State::Asking("task".into()));
        assert_eq!(transition(&asking, Action::Choose), State::Home);
        assert_eq!(transition(&asking, Action::Return), State::Home);
    }

    #[test]
    fn talking_does_not_disable_the_home_button() {
        assert_eq!(transition(&State::Home, Action::Talk), State::Home);
        assert_eq!(transition(&State::Dragging, Action::Talk), State::Dragging);
    }
}

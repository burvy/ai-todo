//! What he offers when he is dropped on a task, and what dismisses the bubble.

use super::*;

/// What he offers to do with a task he was dropped on (Interview 4, item 8). No
/// Delete: deleting is the scribble-out. No "Never mind" either: tapping anywhere
/// off the bubble, him included, is never mind ([`asking_dismissed_by`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Edit,
    Move,
    Done,
    /// A quarter hour more (a slip in the planner).
    Longer,
    /// A quarter hour less (a slip in the planner).
    Shorter,
    /// Off the planner: it stays in the list (a slip in the planner).
    Off,
}

impl Choice {
    pub fn label(self) -> &'static str {
        match self {
            Choice::Edit => "Edit",
            Choice::Move => "Move",
            Choice::Done => "Done",
            Choice::Longer => "Longer",
            Choice::Shorter => "Shorter",
            Choice::Off => "Take off",
        }
    }
}

/// What he was dropped on, which decides what he offers (Interview 3, items 2 and 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// A row of the list: the task itself.
    Row,
    /// A slip in the planner: its time. One day of a repeating series has its length
    /// from the series and is planned with it, so it can only be moved.
    Slip { repeats: bool },
}

/// The bubble's buttons for what he was dropped on, in order.
pub fn offered(on: Asked) -> &'static [Choice] {
    match on {
        Asked::Row => &[Choice::Edit, Choice::Move, Choice::Done],
        Asked::Slip { repeats: false } => {
            &[Choice::Move, Choice::Longer, Choice::Shorter, Choice::Off]
        }
        Asked::Slip { repeats: true } => &[Choice::Move],
    }
}

/// Whether `gesture` while he asks means never mind: a tap anywhere but on what he
/// asks with (`on_answer`: his bubble while he asks about a task, the capture box
/// while he notes a new one), on him as much as on the page. A drag or a hold is
/// the page's business, and a tap on the answer is the answer.
pub fn asking_dismissed_by(
    state: &State,
    gesture: &crate::gesture::Gesture,
    on_answer: bool,
) -> bool {
    matches!(state, State::Asking { .. } | State::Noting { .. })
        && *gesture == crate::gesture::Gesture::Tap
        && !on_answer
}

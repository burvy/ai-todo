//! Rambo's state machine: where he is, what he is doing, and what moves him between.

use super::*;

/// Rambo's complete state: where he is and what he's doing.
///
/// One element, one state: he sits in the nest only when `Home`.
///
/// The first drag listens. Dropped anywhere but a task or the nest he is
/// `Dazed`: he stays where he landed, does not listen, and dragging him again
/// (`Inking`) writes on the page instead. He comes round on his own and skitters
/// home after [`DAZED_MS`], or comes home at once when put back in the nest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Resting at his home spot (bottom-right corner).
    Home,
    /// Picked up: listening, being dragged with the pointer.
    Dragging,
    /// Let go mid-flick: he slides across the screen on the throw's speed and bounces
    /// off its edges until he stops (then he hops home) or hits one hard enough to splat.
    Thrown,
    /// Smacked into an edge of the screen: he lies where he fell and does not listen.
    Dazed,
    /// A dazed Rambo being dragged: he writes on the page and does not listen.
    Inking,
    /// Thrown into an edge hard enough to splat: ink splat + wobble, then dazed.
    Splatting,
    /// Running home after a daze in little hops, each leaving an ink dot.
    Skittering,
    /// Dropped on a task: asking what to do with it.
    Asking { task_id: String, title: String },
    /// Dropped on an empty planner hour: he stays on it and listens for the new task
    /// that starts there (SPEC-ANSWERS Interview 3 item 4). The hour is his to hold, so
    /// it goes when he leaves it.
    Noting { at: NotedHour },
    /// Flying to the capture caret or a task row (transit, not writing yet).
    Flying,
    /// Writing along at the capture caret while the user types.
    WritingCaret,
    /// Writing a fresh task row's title, nib sweeping left to right.
    WritingRow,
    /// The page turned over him: stuck under the leaf, pushing its corner up to get
    /// back onto the page. He is not dazed (only a hard throw dazes him): after
    /// [`COVERED_MS`] he is out and hops home. He cannot be picked up from here.
    Covered,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Pointer down on him (or Space/Enter): he starts listening.
    Grab,
    /// Released without moving: capture what he heard, back to home.
    Release,
    /// Dropped on empty space: he hops home (only a smack dazes him).
    DropEmpty,
    /// Let go mid-flick on empty space: thrown.
    DropHard,
    /// Thrown into an edge hard enough to splat.
    Smack,
    /// Dropped on a task row or planner block.
    DropTask { task_id: String, title: String },
    /// Dropped on an empty hour of the planner.
    DropHour(NotedHour),
    /// Put back in his nest.
    DropNest,
    /// The page he lies on scrolled faster than he can hold on: he is flung with it.
    Swept,
    /// The throw has stopped sliding (home), the splat is over (dazed), the daze is
    /// over (he skitters), or the skitter ends (home).
    Return,
    /// He chose an action on the task he was dropped on.
    Choose,
    /// Hold-to-talk from wherever he is.
    Talk,
    /// Start writing at the caret (user is typing).
    WriteCaret,
    /// Fly to a fresh task row and write it.
    WriteRow,
    /// The row he was about to write is not on screen (the other pane on a
    /// phone, or scrolled away): he writes nothing and stays home.
    SkipRow,
    /// Finish writing and fly home.
    FlyHome,
    /// A leaf closes over him where he stands on the page.
    Covered,
    /// He is out from under the leaf (he pushed his way out, or the leaf lifted away).
    Uncovered,
}

/// Milliseconds he stays dazed, untouched, before he skitters home on his own.
pub const DAZED_MS: u32 = 3000;

/// Milliseconds he is stuck under the leaf, pushing at its corner, before he crawls out.
pub const COVERED_MS: u32 = 1500;

/// Milliseconds the splat wobble lasts on a hard drop.
pub const SPLAT_MS: u32 = 400;

/// Milliseconds Rambo keeps peeking over at you after your last keystroke.
pub const PEEK_MS: u32 = 1400;

/// How far below his middle his nib is, as a share of his height: where a
/// dazed Rambo's ink comes out.
pub const NIB_BELOW: f64 = 0.42;

/// Milliseconds he must be held still before he listens. A drag only carries
/// him; listening starts on this long press, on Edit, or from the keyboard. Safe
/// range 300 to 1000: shorter and a slow start to a drag listens, longer and a
/// hold feels ignored.
pub const LISTEN_HOLD_MS: u32 = 450;

/// How far the pointer may wander during that hold and still count as still.
/// Safe range 4 to 16 px: a resting finger on a phone shakes a few pixels.
pub const LISTEN_SLOP_PX: f64 = 8.0;

pub fn transition(state: &State, event: Event) -> State {
    match (state, event) {
        // From home: grab or talk starts dragging; writing starts flying.
        (State::Home, Event::Grab | Event::Talk) => State::Dragging,
        (State::Home, Event::WriteCaret) => State::WritingCaret,
        (State::Home, Event::WriteRow) => State::Flying,
        (State::Home, Event::FlyHome) => State::Home,

        // Dragging: release, the nest or the page sends him home; drop on a task
        // asks; let go mid-flick throws him (SPEC-ANSWERS item 31).
        (State::Dragging, Event::Release | Event::DropNest | Event::DropEmpty) => State::Home,
        (State::Dragging, Event::DropHard) => State::Thrown,

        // Thrown: only a smack into an edge dazes him; a throw that just stops sends
        // him home. Caught mid-slide, he is held again.
        (State::Thrown, Event::Smack) => State::Splatting,
        (State::Thrown, Event::Return | Event::DropNest) => State::Home,
        (State::Thrown, Event::Grab) => State::Dragging,
        // Lying on the page (dazed, or asking about a row), a scroll faster than he can
        // hold on flings him with the paper: a throw like any other (Interview 1 item 2).
        (State::Dazed | State::Asking { .. }, Event::Swept) => State::Thrown,
        (State::Dragging, Event::DropTask { task_id, title }) => State::Asking { task_id, title },
        (State::Dragging, Event::DropHour(at)) => State::Noting { at },

        // Splatting → Dazed → Skittering → Home.
        (State::Splatting, Event::Return) => State::Dazed,
        (State::Dazed, Event::Return) => State::Skittering,
        (State::Skittering, Event::Return) => State::Home,

        // A leaf closing over him where he stands out on the page: he is stuck under it,
        // pushes his way out and hops home (`Home`, not `Skittering`: he is not dazed and leaves no dots),
        // undazed. Home is the nest, in the bar and
        // on no page; in hand he is above the page; in transit he is not standing there.
        (
            State::Dazed
            | State::Inking
            | State::Splatting
            | State::Asking { .. }
            | State::Noting { .. },
            Event::Covered,
        ) => State::Covered,
        (State::Covered, Event::Uncovered) => State::Home,

        // Dazed: picking him up writes; letting go anywhere but the nest
        // leaves him dazed where he is.
        (State::Dazed, Event::Grab) => State::Inking,
        (State::Inking, Event::DropNest) => State::Home,
        (
            State::Inking,
            Event::Release | Event::DropEmpty | Event::DropHard | Event::DropTask { .. },
        ) => State::Dazed,

        // Asking: choose or return goes home.
        (State::Asking { .. }, Event::Choose | Event::Return) => State::Home,
        // Noting: the new task sent, or never mind, goes home.
        (State::Noting { .. }, Event::Return) => State::Home,

        // Writing caret: fly home when done, or fly to a row (after Add). A
        // row he cannot see is skipped without a flight.
        (State::WritingCaret, Event::FlyHome) => State::Flying,
        (State::WritingCaret, Event::WriteRow) => State::Flying,
        (State::WritingCaret, Event::SkipRow) => State::Home,

        // Flying to a row: arrive and write.
        (State::Flying, Event::WriteRow) => State::WritingRow,

        // Writing row: fly home when done. Picked up mid-row he listens, and
        // typing a new capture calls him to the caret; either way the ink waits
        // and he goes back to finish once he is home (`pen::beat`).
        (State::WritingRow, Event::FlyHome) => State::Flying,
        (State::WritingRow, Event::Grab) => State::Dragging,
        (State::WritingRow, Event::WriteCaret) => State::WritingCaret,

        // Flying home: arrive.
        (State::Flying, Event::FlyHome) => State::Home,

        // Talk is available while away (no state change).
        (state, Event::Talk) => state.clone(),

        // Default: stay.
        (state, _) => state.clone(),
    }
}

/// Whether picking him up from `state` listens. Only a Rambo who was resting
/// listens: dazed, he writes instead.
pub fn listens_on_grab(state: &State) -> bool {
    transition(state, Event::Grab) == State::Dragging
}

/// Whether he lies on the page, so the paper drags him as it scrolls. Already thrown,
/// he is sliding over it, not lying on it.
pub fn lies_on_page(state: &State) -> bool {
    *state != State::Thrown && transition(state, Event::Swept) == State::Thrown
}

/// Whether his nib has run dry after `written_ms` of writing, so that home again he taps
/// it on the nest's rim (Interview 3, idea 2).
pub fn nib_dry(written_ms: f64) -> bool {
    written_ms >= crate::tuning::DRY_NIB_AFTER_MS
}

/// Whether a pointer that picked him up at `grab` and is now at `now` is still
/// holding him (a long press can listen) rather than dragging him.
pub fn held_still(grab: (f64, f64), now: (f64, f64)) -> bool {
    (now.0 - grab.0).hypot(now.1 - grab.1) <= LISTEN_SLOP_PX
}

/// Whether Rambo takes `gesture` whatever is under the finger: the brain is asked
/// before any target (ENGINE-PLAN §5). Dazed, a hold or a drag anywhere is his,
/// drawing him to the finger (Interview 4, item 2), and while he is being drawn the
/// gesture stays his until it ends. Home again, rows and slips get their holds
/// back. A router asks at `Hold` and `Drag`; once he has taken a gesture it keeps
/// it to the end (`Drop`, `Fling`, `Cancel`) whatever his state, and sends a
/// `Cancel` to the target that had its `Down`. Nothing routes through it yet: the
/// engine's router is the first caller (ENGINE-PLAN §5).
pub fn wants(state: &State, gesture: &crate::gesture::Gesture) -> bool {
    use crate::gesture::Gesture;
    matches!(
        (state, gesture),
        (
            State::Dazed | State::Inking,
            Gesture::Hold | Gesture::Drag { .. }
        ) | (
            State::Inking,
            Gesture::Drop { .. } | Gesture::Fling { .. } | Gesture::Cancel
        )
    )
}

/// What a finger elsewhere on the page does to him: dazed, a hold anywhere draws
/// him to the finger and he writes there (`Grab`, so he is `Inking` as if picked
/// up). Only a hold: a finger that slides first is the page's scroll or a row's
/// swipe, and a tap is still the row's.
pub fn drawn_over(state: &State, gesture: &crate::gesture::Gesture) -> Option<Event> {
    (*state == State::Dazed && matches!(gesture, crate::gesture::Gesture::Hold))
        .then_some(Event::Grab)
}

/// Whether going from `before` to `after` is the user putting a dazed Rambo
/// back in his nest, which he forgives them a little for.
pub fn put_back(before: &State, after: &State) -> bool {
    matches!(before, State::Dazed | State::Inking) && *after == State::Home
}

/// True if the state is one where Rambo is physically away from his nest
/// (dragging, dropped somewhere, skittering, writing, or asking).
pub fn is_off_nest(state: &State) -> bool {
    matches!(
        state,
        State::Dragging
            | State::Thrown
            | State::Dazed
            | State::Inking
            | State::Splatting
            | State::Covered
            | State::Skittering
            | State::Asking { .. }
            | State::Noting { .. }
            | State::Flying
            | State::WritingCaret
            | State::WritingRow
    )
}

/// Is he flying, or writing at the caret, so a grab or typing must not yank him?
/// A row he is writing is not busy: anything the user does interrupts it, and he
/// goes back to finish it (`pen::beat`).
pub fn pen_busy(state: &State) -> bool {
    matches!(state, State::Flying | State::WritingCaret)
}

/// Whether a line he says now waits until he is home: he is out writing a row, or
/// flying to or from one (DESIGN-DIRECTION §4 beat 4). Speaking mid-row puts his
/// bubble over the page he is writing on.
pub fn speaks_later(state: &State) -> bool {
    matches!(state, State::Flying | State::WritingRow)
}

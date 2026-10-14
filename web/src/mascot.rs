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
//!
//! [`Mood`] is the art side, also pure: the tool he holds decides which folder
//! the drawing comes from, and the mood (what he feels) decides which drawing
//! that is. Every path is built here so a missing file is a test failure, not a
//! broken image in the corner of the page.

use crate::settings::Pen;

/// The folder a tool's drawings live in. The fountain pen is `fountain-pen` in
/// the settings and in `data-pen`, and `fountain` on disk.
fn art_folder(pen: Pen) -> &'static str {
    match pen {
        Pen::FountainPen => "fountain",
        other => other.slug(),
    }
}

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
    /// Out at a tear in the list page, taping it (`rip.rs`): he hops there, stays until the
    /// tape is on ([`MEND_MS`]), then hops home.
    Mending,
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
    /// Put back in his nest.
    DropNest,
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
    /// The page was torn: he goes to tape it.
    Mend,
}

/// Milliseconds he stays dazed, untouched, before he skitters home on his own.
pub const DAZED_MS: u32 = 3000;
/// Milliseconds he is stuck under the leaf, pushing at its corner, before he crawls out.
pub const COVERED_MS: u32 = 1500;
/// Milliseconds he stays at a tear, taping it, before he hops home.
pub use crate::tuning::MEND_MS;
/// Milliseconds the splat wobble lasts on a hard drop.
pub const SPLAT_MS: u32 = 400;
/// Milliseconds Rambo keeps peeking over at you after your last keystroke.
pub const PEEK_MS: u32 = 1400;
/// How far below his middle his nib is, as a share of his height: where a
/// dazed Rambo's ink comes out.
pub const NIB_BELOW: f64 = 0.42;
/// How far outside the nest's edge a drop still counts as putting him back.
pub const NEST_SLACK_PX: f64 = 16.0;
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
        // A torn page: he goes to tape it. Only a resting Rambo does; one who is busy
        // elsewhere leaves it to fade.
        (State::Home, Event::Mend) => State::Mending,
        (State::Mending, Event::Return) => State::Home,
        (State::Mending, Event::Grab | Event::Talk) => State::Dragging,

        // Dragging: release, the nest or the page sends him home; drop on a task
        // asks; let go mid-flick throws him (SPEC-ANSWERS item 31).
        (State::Dragging, Event::Release | Event::DropNest | Event::DropEmpty) => State::Home,
        (State::Dragging, Event::DropHard) => State::Thrown,

        // Thrown: only a smack into an edge dazes him; a throw that just stops sends
        // him home. Caught mid-slide, he is held again.
        (State::Thrown, Event::Smack) => State::Splatting,
        (State::Thrown, Event::Return | Event::DropNest) => State::Home,
        (State::Thrown, Event::Grab) => State::Dragging,
        (State::Dragging, Event::DropTask { task_id, title }) => State::Asking { task_id, title },

        // Splatting → Dazed → Skittering → Home.
        (State::Splatting, Event::Return) => State::Dazed,
        (State::Dazed, Event::Return) => State::Skittering,
        (State::Skittering, Event::Return) => State::Home,

        // A leaf closing over him where he stands out on the page: he is stuck under it,
        // pushes his way out and hops home (`Home`, not `Skittering`: he is not dazed and leaves no dots),
        // undazed. Home is the nest, in the bar and
        // on no page; in hand he is above the page; in transit he is not standing there.
        (
            State::Dazed | State::Inking | State::Splatting | State::Asking { .. },
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

/// What he offers to do with a task he was dropped on (Interview 4, item 8). No
/// Delete: deleting is the scribble-out. No "Never mind" either: tapping anywhere
/// off the bubble, him included, is never mind ([`asking_dismissed_by`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Edit,
    Move,
    Done,
}

impl Choice {
    /// The bubble's buttons, in order.
    pub const ALL: [Choice; 3] = [Choice::Edit, Choice::Move, Choice::Done];

    pub fn label(self) -> &'static str {
        match self {
            Choice::Edit => "Edit",
            Choice::Move => "Move",
            Choice::Done => "Done",
        }
    }
}

/// Whether `gesture` while he asks means never mind: a tap anywhere but on his
/// bubble (`on_bubble`), on him as much as on the page. A drag or a hold is the
/// page's business, and a tap on the bubble is a choice.
pub fn asking_dismissed_by(
    state: &State,
    gesture: &crate::gesture::Gesture,
    on_bubble: bool,
) -> bool {
    matches!(state, State::Asking { .. }) && *gesture == crate::gesture::Gesture::Tap && !on_bubble
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

/// Where he rests in a nest whose box is `(left, top, width, height)`: the one point every
/// place that sends him home agrees on.
pub fn nest_home((left, top, width, height): (f64, f64, f64, f64)) -> (f64, f64) {
    (
        left + width * crate::tuning::NEST_HOME.0,
        top + height * crate::tuning::NEST_HOME.1,
    )
}

/// Whether a drop at `point` puts him back in the nest whose box is
/// `(left, top, right, bottom)`: on the dish, or near enough that a hand
/// aiming for it would call it in.
pub fn over_nest(point: (f64, f64), nest: (f64, f64, f64, f64)) -> bool {
    let (x, y) = point;
    let (left, top, right, bottom) = nest;
    (left - NEST_SLACK_PX..=right + NEST_SLACK_PX).contains(&x)
        && (top - NEST_SLACK_PX..=bottom + NEST_SLACK_PX).contains(&y)
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
            | State::Thrown
            | State::Dazed
            | State::Inking
            | State::Splatting
            | State::Covered
            | State::Mending
            | State::Skittering
            | State::Asking { .. }
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

/// Whether he hops to a new spot (`crate::hop`) rather than simply being there.
/// He is wherever his spot is, every frame, while he follows something exactly:
/// your finger, his nib on a row, the place he fell or the row he asks about. At
/// home he follows the nest as the page scrolls, once he has `settled_home`;
/// getting there is a hop like any other move.
pub fn hops_there(state: &State, settled_home: bool) -> bool {
    match state {
        State::Dragging
        | State::Thrown
        | State::Inking
        | State::WritingRow
        | State::Dazed
        | State::Splatting
        | State::Covered
        | State::Asking { .. } => false,
        State::Home => !settled_home,
        State::Flying | State::WritingCaret | State::Skittering | State::Mending => true,
    }
}

/// The face he makes in the beat before he sets off (DESIGN-DIRECTION §9, "Rambo
/// reacts before he moves"), or `None` when going from `before` to `after` sends him
/// nowhere new. He never moves instantly: every run of hops starts in `rambo.rs`'s
/// mover, which asks this first and holds the face for `NOTICE_BEAT_MS`.
pub fn notice(before: &State, after: &State) -> Option<Pose> {
    match (before, after) {
        _ if before == after => None,
        // Landing at the end of a trip is not setting off.
        (State::Flying | State::Skittering, State::Home) => None,
        // He comes round from the daze with a start, then runs home.
        (_, State::Skittering) => Some(Pose::Worried),
        // Called to write, or going home: he looks up, then goes.
        (_, State::Home | State::Flying | State::WritingCaret) => Some(Pose::Teasing),
        // The torn page puts him out: he looks it over, then goes.
        (_, State::Mending) => Some(Pose::Sulking),
        _ => None,
    }
}

/// How much he perks up `elapsed` ms into the notice beat, as a hop's squash
/// (negative is stretched): it rises and falls back to nothing, so the hop's crouch
/// starts from his resting shape.
pub fn notice_squash(elapsed: u32) -> f64 {
    let share = (f64::from(elapsed) / f64::from(crate::tuning::NOTICE_BEAT_MS)).clamp(0.0, 1.0);
    -crate::tuning::NOTICE_PERK * (std::f64::consts::PI * share).sin()
}

/// Where his centre rides while you hold him at `pointer`: just above it, so your
/// finger or cursor never covers him. `box_h` is his height on this screen; a
/// fingertip covers more than a cursor, so a `touch` holds him higher.
pub fn held_at(pointer: (f64, f64), box_h: f64, touch: bool) -> (f64, f64) {
    let gap = match touch {
        true => crate::tuning::HELD_GAP_TOUCH_PX,
        false => crate::tuning::HELD_GAP_POINTER_PX,
    };
    (pointer.0, pointer.1 - box_h / 2.0 - gap)
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
        State::Home => " is-home",
        State::Dragging => " is-dragging",
        State::Thrown => " is-thrown",
        State::Dazed => " is-dazed",
        State::Covered => " is-covered",
        State::Mending => " is-mending",
        State::Inking => " is-inking",
        State::Splatting => " is-splatting",
        State::Skittering => " is-skittering",
        State::Asking { .. } => " is-asking",
        State::Flying => " is-flying",
        State::WritingCaret => " is-flying is-writing is-caret",
        // Not a flight: the flight's eased transition would trail the nib behind
        // the ink, retargeted every frame.
        State::WritingRow => " is-writing",
    };
    if pose.is_empty() {
        format!("{base}{extra}")
    } else {
        format!("{base}{extra} pose-{pose}")
    }
}

/// One drawing per mood, shared by every tool: the file name inside that tool's
/// folder. Every variant here has a file on disk —
/// `assets_have_every_pose_for_every_tool` fails the moment that stops being
/// true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pose {
    Idle,
    /// Looking up from the page at you: peeking while you type, and the
    /// quizzical tilt when he asks what you want with a task.
    Teasing,
    Listening,
    Writing,
    Flying,
    Dozing,
    Happy,
    Sulking,
    Worried,
    Lobotomized,
}

impl Pose {
    pub fn id(self) -> &'static str {
        match self {
            Pose::Idle => "idle",
            Pose::Teasing => "teasing",
            Pose::Listening => "listening",
            Pose::Writing => "writing",
            Pose::Flying => "flying",
            Pose::Dozing => "dozing",
            Pose::Happy => "happy",
            Pose::Sulking => "sulking",
            Pose::Worried => "worried",
            Pose::Lobotomized => "lobotomized",
        }
    }

    /// The drawing of Rambo in this pose, holding this tool.
    pub fn art(self, pen: Pen) -> String {
        format!("assets/rambo/{}/{}.svg", art_folder(pen), self.id())
    }
}

/// The little pictures that float beside Rambo for a mood: `assets/rambo/fx/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fx {
    SleepyZ,
    Sparks,
    ScribbleCloud,
}

impl Fx {
    pub fn id(self) -> &'static str {
        match self {
            Fx::SleepyZ => "sleepy-z",
            Fx::Sparks => "sparks",
            Fx::ScribbleCloud => "scribble-cloud",
        }
    }

    pub fn art(self) -> String {
        format!("assets/rambo/fx/{}.svg", self.id())
    }
}

/// What Rambo is feeling while nothing else is happening.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mood {
    #[default]
    Resting,
    Peeking,
    Dozing,
    Celebrating,
    Sulking,
    Annoyed,
}

impl Mood {
    /// The drawing this mood wears.
    pub fn pose(self) -> Pose {
        match self {
            Mood::Resting => Pose::Idle,
            // The art has no `peek` and no `annoyed`: the grinning look and the
            // lowered brows are the nearest neighbours until Ping draws both.
            Mood::Peeking => Pose::Teasing,
            Mood::Annoyed => Pose::Worried,
            Mood::Dozing => Pose::Dozing,
            Mood::Celebrating => Pose::Happy,
            Mood::Sulking => Pose::Sulking,
        }
    }

    /// The picture that floats beside him for this mood, if it has one.
    pub fn fx(self) -> Option<Fx> {
        match self {
            Mood::Dozing => Some(Fx::SleepyZ),
            Mood::Celebrating => Some(Fx::Sparks),
            Mood::Annoyed => Some(Fx::ScribbleCloud),
            Mood::Resting | Mood::Peeking | Mood::Sulking => None,
        }
    }

    /// What one of the brain's pose names means for his mood. The names come
    /// from `rambl_brain`: `sleep` is night, `cheer` is a task ticked off,
    /// `worried` is an overdue task. Every other name leaves him as he is.
    pub fn from_brain_pose(pose: &str) -> Option<Self> {
        match pose {
            "sleep" => Some(Mood::Dozing),
            "cheer" => Some(Mood::Celebrating),
            "worried" => Some(Mood::Sulking),
            _ => None,
        }
    }
}

/// The mood on screen. Typing wakes him and makes him look up at you, whether
/// the brain is asleep or sulking; otherwise the brain's pose decides.
pub fn mood_for(brain_pose: &str, typing: bool) -> Mood {
    if typing {
        Mood::Peeking
    } else {
        Mood::from_brain_pose(brain_pose).unwrap_or_default()
    }
}

/// The drawing to show: what Rambo is doing beats how he feels, and a keystroke
/// beats the mood — that is how he wakes up at night and looks over while you
/// type. A lobotomized Rambl is vacant whatever you do, so it wins over both.
pub fn pose_for(state: &State, brain_pose: &str, typing: bool) -> Pose {
    match state {
        State::Dragging => Pose::Listening,
        State::WritingCaret | State::WritingRow | State::Inking => Pose::Writing,
        State::Flying | State::Thrown => Pose::Flying,
        State::Asking { .. } => Pose::Teasing,
        // Vacant: the nearest drawing to a daze until one is drawn.
        State::Dazed => Pose::Lobotomized,
        // Stuck and pushing, not dazed.
        State::Covered => Pose::Worried,
        // The torn page puts him in a mood: grumpy while he mends it.
        State::Mending => Pose::Sulking,
        State::Home | State::Splatting | State::Skittering => {
            if typing {
                Mood::Peeking.pose()
            } else if brain_pose == "lobotomized" {
                Pose::Lobotomized
            } else {
                mood_for(brain_pose, typing).pose()
            }
        }
    }
}

/// The mood picture beside him. Away from the nest it is hidden: he is mid-flight
/// or mid-drag, and a floating z would trail him across the page.
pub fn fx_for(state: &State, brain_pose: &str, typing: bool) -> Option<Fx> {
    if is_off_nest(state) {
        None
    } else {
        mood_for(brain_pose, typing).fx()
    }
}

/// The drawing of this tool, in this pose, at this moment: the one path the
/// page's `<img>` needs.
pub fn art_for(pen: Pen, state: &State, brain_pose: &str, typing: bool) -> String {
    pose_for(state, brain_pose, typing).art(pen)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every pose the app can ask for. The path test walks this list, so a new
    /// pose that ships without art fails the build.
    const REACHABLE_POSES: [Pose; 10] = [
        Pose::Idle,
        Pose::Teasing,
        Pose::Listening,
        Pose::Writing,
        Pose::Flying,
        Pose::Dozing,
        Pose::Happy,
        Pose::Sulking,
        Pose::Worried,
        Pose::Lobotomized,
    ];

    /// Where a page-relative art path lives in the checkout.
    fn art_file(path: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
    }

    #[test]
    fn grab_listens_and_release_goes_home() {
        let state = transition(&State::Home, Event::Grab);
        assert_eq!(state, State::Dragging);
        assert_eq!(transition(&state, Event::Release), State::Home);
    }

    #[test]
    fn a_gentle_drop_on_the_page_sends_him_home_undazed() {
        let state = transition(&State::Home, Event::Grab);
        assert_eq!(transition(&state, Event::DropEmpty), State::Home);
    }

    #[test]
    fn only_a_smack_dazes_him_and_after_the_daze_he_skitters_home() {
        let thrown = transition(&transition(&State::Home, Event::Grab), Event::DropHard);
        assert_eq!(thrown, State::Thrown);
        assert!(is_off_nest(&thrown));
        assert_eq!(
            transition(&thrown, Event::Return),
            State::Home,
            "it just stopped"
        );
        assert_eq!(transition(&thrown, Event::Grab), State::Dragging, "caught");
        let splat = transition(&thrown, Event::Smack);
        assert_eq!(splat, State::Splatting);
        let dazed = transition(&splat, Event::Return);
        assert_eq!(dazed, State::Dazed);
        let skittering = transition(&dazed, Event::Return);
        assert_eq!(skittering, State::Skittering);
        assert_eq!(transition(&skittering, Event::Return), State::Home);
    }

    #[test]
    fn only_the_first_drag_listens_a_dazed_drag_writes() {
        assert!(listens_on_grab(&State::Home));
        assert!(!listens_on_grab(&State::Dazed));
        let state = transition(&State::Dazed, Event::Grab);
        assert_eq!(state, State::Inking);
        assert_eq!(pose_for(&state, "", false), Pose::Writing);
        // Wherever a writing drag ends he is still dazed, task rows included.
        for drop in [
            Event::Release,
            Event::DropEmpty,
            Event::DropHard,
            Event::DropTask {
                task_id: "t1".into(),
                title: "Lab report".into(),
            },
        ] {
            assert_eq!(transition(&state, drop), State::Dazed);
        }
        // Not while he is on his way home or splatting either.
        assert!(!listens_on_grab(&State::Skittering));
        assert!(!listens_on_grab(&State::Splatting));
    }

    #[test]
    fn putting_him_back_in_the_nest_ends_the_daze_and_is_forgiven() {
        let state = transition(&State::Inking, Event::DropNest);
        assert_eq!(state, State::Home);
        assert!(put_back(&State::Inking, &state));
        assert!(listens_on_grab(&state), "home again, he listens again");
        // Dropping a listening Rambo on his nest just sends him home.
        assert_eq!(transition(&State::Dragging, Event::DropNest), State::Home);
        assert!(!put_back(&State::Dragging, &State::Home));
        // Skittering home on his own is not being put back.
        assert!(!put_back(&State::Skittering, &State::Home));
    }

    #[test]
    fn a_drop_on_or_just_beside_the_dish_puts_him_back() {
        let nest = (100.0, 100.0, 164.0, 164.0);
        assert!(over_nest((132.0, 132.0), nest));
        assert!(over_nest((100.0 - NEST_SLACK_PX, 170.0), nest));
        assert!(!over_nest((100.0 - NEST_SLACK_PX - 1.0, 132.0), nest));
        assert!(!over_nest((132.0, 300.0), nest));
    }

    #[test]
    fn he_rests_inside_the_nest_box_whatever_its_size_or_place() {
        for (left, top, width, height) in [
            (286.0, 771.0, 104.0, 73.0),
            (1100.0, 24.0, 112.0, 96.0),
            (0.0, 0.0, 60.0, 50.0),
        ] {
            let (x, y) = nest_home((left, top, width, height));
            assert!((left..=left + width).contains(&x), "inside across");
            assert!((top..=top + height).contains(&y), "inside down");
            assert!(
                (x - (left + width / 2.0)).abs() < 1e-9,
                "in the middle across"
            );
            assert!(
                y < top + height / 2.0,
                "a little above the middle of the box"
            );
        }
    }

    #[test]
    fn only_a_resting_rambo_goes_to_tape_a_tear() {
        assert_eq!(transition(&State::Home, Event::Mend), State::Mending);
        for busy in [
            State::Dazed,
            State::WritingRow,
            State::Covered,
            State::Flying,
        ] {
            assert_eq!(
                transition(&busy, Event::Mend),
                busy,
                "{busy:?} leaves it to fade"
            );
        }
        assert_eq!(transition(&State::Mending, Event::Return), State::Home);
        assert_eq!(transition(&State::Mending, Event::Grab), State::Dragging);
        assert!(hops_there(&State::Mending, false), "he hops to the tear");
        assert!(is_off_nest(&State::Mending));
        assert_eq!(notice(&State::Home, &State::Mending), Some(Pose::Sulking));
        assert_eq!(pose_for(&State::Mending, "home", false), Pose::Sulking);
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
    fn he_hops_between_spots_and_follows_what_he_tracks() {
        for moving in [State::Flying, State::WritingCaret, State::Skittering] {
            assert!(hops_there(&moving, true), "{moving:?}");
        }
        for tracking in [
            State::Dragging,
            State::Inking,
            State::WritingRow,
            State::Dazed,
            State::Splatting,
        ] {
            assert!(!hops_there(&tracking, false), "{tracking:?}");
        }
        assert!(hops_there(&State::Home, false), "the way home is hops");
        assert!(
            !hops_there(&State::Home, true),
            "home, he rides the scrolling nest"
        );
    }

    /// Every state, and every event, the guard tests below walk.
    fn every_state() -> Vec<State> {
        vec![
            State::Home,
            State::Dragging,
            State::Dazed,
            State::Inking,
            State::Splatting,
            State::Skittering,
            State::Asking {
                task_id: "t1".into(),
                title: "Lab report".into(),
            },
            State::Flying,
            State::WritingCaret,
            State::WritingRow,
        ]
    }

    fn every_event() -> Vec<Event> {
        vec![
            Event::Grab,
            Event::Release,
            Event::DropEmpty,
            Event::DropHard,
            Event::DropTask {
                task_id: "t1".into(),
                title: "Lab report".into(),
            },
            Event::DropNest,
            Event::Return,
            Event::Choose,
            Event::Talk,
            Event::WriteCaret,
            Event::WriteRow,
            Event::SkipRow,
            Event::FlyHome,
        ]
    }

    #[test]
    fn no_move_starts_without_the_notice_beat() {
        // DESIGN-DIRECTION §9: he never moves instantly. Any change of state that
        // sets him hopping somewhere new shows a face first; only landing at the
        // end of a trip does not.
        for before in every_state() {
            for event in every_event() {
                let after = transition(&before, event.clone());
                let sets_off = after != before
                    && hops_there(&after, false)
                    && !matches!(
                        (&before, &after),
                        (State::Flying | State::Skittering, State::Home)
                    );
                assert_eq!(
                    notice(&before, &after).is_some(),
                    sets_off,
                    "{before:?} --{event:?}--> {after:?}"
                );
            }
        }
    }

    #[test]
    fn he_reacts_to_what_sends_him() {
        assert_eq!(
            notice(&State::Home, &State::WritingCaret),
            Some(Pose::Teasing)
        );
        assert_eq!(
            notice(&State::WritingRow, &State::Flying),
            Some(Pose::Teasing)
        );
        assert_eq!(
            notice(&State::Dazed, &State::Skittering),
            Some(Pose::Worried),
            "he comes round with a start"
        );
        assert_eq!(notice(&State::Flying, &State::Home), None, "he has landed");
        assert_eq!(notice(&State::Home, &State::Dragging), None, "you move him");
    }

    #[test]
    fn the_perk_rises_and_settles_before_the_hop() {
        let beat = crate::tuning::NOTICE_BEAT_MS;
        assert_eq!(notice_squash(0), 0.0);
        assert!(notice_squash(beat / 2) < 0.0, "stretched at the top");
        assert!(notice_squash(beat).abs() < 1e-9, "back to rest as he hops");
        assert!(notice_squash(beat * 3).abs() < 1e-9);
    }

    #[test]
    fn held_he_rides_above_the_finger() {
        let finger = (100.0, 300.0);
        let (x, y) = held_at(finger, 64.0, true);
        assert_eq!(x, 100.0);
        assert!(
            y + 32.0 < finger.1,
            "his bottom edge is above the finger: {y}"
        );
        let (_, cursor_y) = held_at(finger, 64.0, false);
        assert!(
            cursor_y + 32.0 < finger.1 && cursor_y > y,
            "a cursor holds him lower"
        );
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
    fn writing_a_row_never_eases_him() {
        // Only a flight eases (`only_a_flight_eases_him_from_place_to_place`), so a
        // row written without the flight's class follows the ink frame by frame.
        assert!(!rambo_class(&State::WritingRow, "").contains("is-flying"));
        assert!(rambo_class(&State::Flying, "").contains("is-flying"));
    }

    #[test]
    fn he_speaks_once_he_is_home() {
        assert!(speaks_later(&State::WritingRow));
        assert!(speaks_later(&State::Flying));
        assert!(!speaks_later(&State::Home));
        assert!(
            !speaks_later(&State::Dragging),
            "picked up, he answers at once"
        );
    }

    #[test]
    fn anything_the_user_does_interrupts_a_row() {
        assert!(!pen_busy(&State::WritingRow));
        assert_eq!(transition(&State::WritingRow, Event::Grab), State::Dragging);
        assert_eq!(
            transition(&State::WritingRow, Event::WriteCaret),
            State::WritingCaret
        );
    }

    #[test]
    fn pen_busy_during_writing_flight() {
        assert!(!pen_busy(&State::Home));
        assert!(!pen_busy(&State::Dragging));
        assert!(pen_busy(&State::Flying));
        assert!(pen_busy(&State::WritingCaret));
    }

    #[test]
    fn he_sits_in_the_nest_between_its_back_and_its_rim() {
        // The order of the nest's layers is data in :root: its back is drawn with
        // the capture bar, he is above that, and the rim is above him.
        let theme = include_str!("../theme.css");
        let layer = |name: &str| -> i32 {
            let at = theme
                .find(&format!("--layer-{name}:"))
                .unwrap_or_else(|| panic!("--layer-{name} in :root"));
            theme[at..]
                .split([':', ';'])
                .nth(1)
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or_else(|| panic!("--layer-{name} is a number"))
        };
        assert!(
            layer("capture") < layer("rambo"),
            "the nest's back is behind him"
        );
        assert!(
            layer("rambo") < layer("nest-rim"),
            "the rim is in front of him"
        );
        assert!(
            layer("nest-rim") < layer("rambo-bubble"),
            "the rim never covers what he says"
        );
    }

    #[test]
    fn rambo_classes_cover_every_state() {
        assert_eq!(rambo_class(&State::Home, ""), "rambo is-home");
        assert!(rambo_class(&State::Dragging, "").contains("is-dragging"));
        assert!(rambo_class(&State::Splatting, "").contains("is-splatting"));
        assert!(rambo_class(&State::Dazed, "").contains("is-dazed"));
        assert!(rambo_class(&State::Inking, "").contains("is-inking"));
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
        let state = transition(&state, Event::FlyHome);
        assert_eq!(state, State::Home);
    }

    #[test]
    fn a_row_he_cannot_see_is_skipped_without_a_flight() {
        // After Add he sits at the caret; a new row that is off screen (the
        // other pane on a phone, or scrolled away) leaves nothing to fly to,
        // and he must not be left hovering at a caret the user has left.
        assert_eq!(
            transition(&State::WritingCaret, Event::SkipRow),
            State::Home
        );
        // Already home, the skip changes nothing.
        assert_eq!(transition(&State::Home, Event::SkipRow), State::Home);
    }

    #[test]
    fn every_drawing_defines_every_colour_it_uses() {
        // The art is shown through <img>, which never sees the page's CSS: a
        // colour the file uses but does not define itself paints Rambo black.
        let pens = [
            Pen::FountainPen,
            Pen::Pencil,
            Pen::Ballpoint,
            Pen::Quill,
            Pen::Marker,
        ];
        for pen in pens {
            for pose in REACHABLE_POSES {
                let path = pose.art(pen);
                let svg = std::fs::read_to_string(art_file(&path)).expect("art exists");
                // A `<` inside the style block (even in a CSS comment) is a tag
                // to the XML parser, and the browser then shows no Rambo at all.
                let style = svg
                    .split_once("<style>")
                    .and_then(|(_, rest)| rest.split_once("</style>"))
                    .map_or("", |(css, _)| css);
                assert!(
                    !style.contains('<'),
                    "{path}: '<' inside <style> breaks the SVG"
                );
                for used in svg.split("var(--").skip(1) {
                    let name = &used[..used.find([')', ',']).expect("var() closes")];
                    assert!(
                        svg.contains(&format!("--{name}:")),
                        "{path} uses --{name} but never defines it"
                    );
                }
            }
        }
    }

    #[test]
    fn every_tool_draws_from_its_own_folder() {
        assert_eq!(
            Pose::Idle.art(Pen::FountainPen),
            "assets/rambo/fountain/idle.svg"
        );
        assert_eq!(
            Pose::Writing.art(Pen::Quill),
            "assets/rambo/quill/writing.svg"
        );
        // The fountain pen is the one tool whose slug and folder differ.
        assert_eq!(Pen::FountainPen.slug(), "fountain-pen");
        assert_eq!(
            [
                Pen::FountainPen,
                Pen::Pencil,
                Pen::Ballpoint,
                Pen::Quill,
                Pen::Marker
            ]
            .map(art_folder),
            ["fountain", "pencil", "ballpoint", "quill", "marker"]
        );
    }

    #[test]
    fn assets_have_every_pose_for_every_tool() {
        // The paths are built from strings, so nothing stops them drifting from
        // the art on disk. Every pose a mood or a state can reach must exist as
        // a file, for every tool the gear menu offers.
        for pen in [
            Pen::FountainPen,
            Pen::Pencil,
            Pen::Ballpoint,
            Pen::Quill,
            Pen::Marker,
        ] {
            for pose in REACHABLE_POSES {
                let file = art_file(&pose.art(pen));
                assert!(file.is_file(), "missing art: {file:?}");
            }
        }
    }

    #[test]
    fn the_tool_in_settings_picks_the_drawing() {
        // The whole point of the pen picker: the tool the page stores is the
        // folder the drawing comes from.
        let state = State::Home;
        assert_eq!(
            art_for(Pen::Marker, &state, "", false),
            "assets/rambo/marker/idle.svg"
        );
        assert_eq!(
            art_for(Pen::Ballpoint, &state, "cheer", false),
            "assets/rambo/ballpoint/happy.svg"
        );
    }

    #[test]
    fn fx_art_lives_beside_the_skins() {
        for fx in [Fx::SleepyZ, Fx::Sparks, Fx::ScribbleCloud] {
            let file = art_file(&fx.art());
            assert!(file.is_file(), "missing art: {file:?}");
        }
    }

    #[test]
    fn what_he_is_doing_beats_how_he_feels() {
        // Being dragged is the held/listening pose, whatever the mood says.
        assert_eq!(pose_for(&State::Dragging, "cheer", false), Pose::Listening);
        assert_eq!(pose_for(&State::WritingRow, "sleep", false), Pose::Writing);
        assert_eq!(pose_for(&State::Flying, "", false), Pose::Flying);
        // Dropped on a task he asks: the quizzical tilt.
        assert_eq!(
            pose_for(
                &State::Asking {
                    task_id: "t1".into(),
                    title: "Lab report".into()
                },
                "",
                false
            ),
            Pose::Teasing
        );
    }

    #[test]
    fn he_doze_s_at_night_and_wakes_when_you_type() {
        assert_eq!(pose_for(&State::Home, "sleep", false), Pose::Dozing);
        assert_eq!(fx_for(&State::Home, "sleep", false), Some(Fx::SleepyZ));
        // A keystroke wakes him even in the middle of the night.
        assert_eq!(pose_for(&State::Home, "sleep", true), Pose::Teasing);
        assert_eq!(fx_for(&State::Home, "sleep", true), None);
    }

    #[test]
    fn he_peeks_and_celebrates_and_sulks() {
        assert_eq!(pose_for(&State::Home, "", true), Pose::Teasing);
        assert_eq!(pose_for(&State::Home, "cheer", false), Pose::Happy);
        assert_eq!(fx_for(&State::Home, "cheer", false), Some(Fx::Sparks));
        assert_eq!(pose_for(&State::Home, "worried", false), Pose::Sulking);
        // Nothing to sulk about, nothing to draw.
        assert_eq!(pose_for(&State::Home, "home", false), Pose::Idle);
        assert_eq!(fx_for(&State::Home, "home", false), None);
        // An unknown pose name is not a mood.
        assert_eq!(mood_for("dangle", false), Mood::Resting);
        assert_eq!(mood_for("cheer", false), Mood::Celebrating);
    }

    #[test]
    fn tidying_up_makes_him_annoyed() {
        assert_eq!(Mood::Annoyed.pose(), Pose::Worried);
        assert_eq!(Mood::Annoyed.fx(), Some(Fx::ScribbleCloud));
    }

    #[test]
    fn a_lobotomized_rambl_stands_vacant() {
        for state in [State::Home, State::Splatting] {
            assert_eq!(pose_for(&state, "lobotomized", false), Pose::Lobotomized);
        }
    }

    #[test]
    fn no_mood_picture_follows_him_off_the_nest() {
        for state in [
            State::Dragging,
            State::Dazed,
            State::Inking,
            State::Splatting,
            State::Skittering,
            State::Asking {
                task_id: "t1".into(),
                title: "Lab report".into(),
            },
            State::Flying,
            State::WritingCaret,
            State::WritingRow,
        ] {
            assert_eq!(fx_for(&state, "sleep", false), None, "{state:?}");
        }
    }

    #[test]
    fn the_element_class_still_carries_the_pose() {
        // The CSS keeps owning how a pose looks; the image src owns who Rambo is.
        assert!(rambo_class(&State::Home, "dozing").contains("pose-dozing"));
        assert!(!rambo_class(&State::Home, "").contains("pose-"));
    }

    /// Each `property` set on the Rambo button itself, in any state, as
    /// (selector, value).
    fn rambo_rules(css: &str, property: &str) -> Vec<(String, String)> {
        let property = format!("{property}:");
        let mut bare = String::new();
        let mut rest = css;
        while let Some((before, after)) = rest.split_once("/*") {
            bare.push_str(before);
            rest = after.split_once("*/").map_or("", |(_, tail)| tail);
        }
        bare.push_str(rest);
        let is_button = |selector: &str| {
            selector.strip_prefix(".rambo").is_some_and(|tail| {
                tail.split(['.', ':'])
                    .all(|part| part.chars().all(|c| c.is_alphanumeric() || c == '-'))
                    && !tail.contains("::")
            })
        };
        bare.split('}')
            .filter_map(|block| block.rsplit_once('{'))
            .flat_map(|(head, body)| {
                let selector = head.rsplit(['{', ';']).next().unwrap_or(head);
                selector
                    .split(',')
                    .map(str::trim)
                    .filter(|one| is_button(one))
                    .flat_map(|one| {
                        body.split(';')
                            .filter_map(|decl| decl.trim().strip_prefix(property.as_str()))
                            .map(|value| (one.to_owned(), value.trim().to_owned()))
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Every argument handed to a `.set(...)` signal write, trimmed. A call
    /// whose argument names a `State` constructor writes the mascot's state
    /// directly instead of asking `transition`.
    fn direct_state_sets(source: &str) -> Vec<&str> {
        let mut found = Vec::new();
        let mut rest = source;
        while let Some((_, after)) = rest.split_once(".set(") {
            // The argument runs to the parenthesis that closes the call.
            let mut depth = 1;
            let end = after
                .char_indices()
                .find_map(|(i, c)| match c {
                    '(' => {
                        depth += 1;
                        None
                    }
                    ')' => {
                        depth -= 1;
                        (depth == 0).then_some(i)
                    }
                    _ => None,
                })
                .unwrap_or(after.len());
            let arg = after[..end].trim();
            if arg.starts_with("State::") || arg.starts_with("mascot::State::") {
                found.push(arg);
            }
            rest = &after[end..];
        }
        found
    }

    #[test]
    fn only_a_still_hold_listens() {
        assert!(held_still((100.0, 100.0), (100.0, 100.0)));
        assert!(
            held_still((100.0, 100.0), (105.0, 106.0)),
            "a shaky finger is still"
        );
        assert!(
            !held_still((100.0, 100.0), (110.0, 100.0)),
            "a drag is not a hold"
        );
    }

    /// The body of the handler written right after `on:{event}=` in `rambo.rs`.
    fn rambo_handler(event: &str) -> &'static str {
        let source = include_str!("rambo.rs");
        let (_, rest) = source
            .split_once(&format!("on:{event}="))
            .expect("rambo.rs has the handler");
        rest.split_once("\n            on:")
            .map_or(rest, |(body, _)| body)
    }

    #[test]
    fn picking_him_up_does_not_listen_at_once() {
        // Emilio saw "No speech was detected." after dropping him on a task: every
        // grab listened. Listening waits for a still hold (`LISTEN_HOLD_MS`).
        let down = rambo_handler("pointerdown");
        assert!(down.contains("LISTEN_HOLD_MS"), "{down}");
        let before_timer = down.split_once("LISTEN_HOLD_MS").map_or(down, |(b, _)| b);
        assert!(
            !before_timer.contains("start_speech_down();"),
            "a grab must not start listening before the hold: {down}"
        );
    }

    #[test]
    fn edit_in_the_ask_bubble_opens_the_edit_sheet() {
        // QA on 39eb0d9: "Edit" closed the bubble and bumped the task's priority
        // instead of opening the sheet. The button must ask the list for its sheet.
        // It also listens for the change. The bubble's buttons are `Choice`s, and
        // what Edit does is its arm in rambo.rs's `choose`.
        let source = include_str!("rambo.rs");
        let (_, arm) = source
            .split_once("mascot::Choice::Edit =>")
            .expect("the bubble has an Edit choice");
        let handler = arm
            .split_once("mascot::Choice::Move")
            .map_or(arm, |(edit, _)| edit);
        assert!(handler.contains("listen_for_edit("), "{handler}");
        assert!(
            handler.contains("edit_request.set("),
            "Edit must open the edit sheet: {handler}"
        );
        assert!(
            !handler.contains("patch_and_show"),
            "Edit must not change the task itself: {handler}"
        );
    }

    #[test]
    fn every_state_change_goes_through_the_machine() {
        // The DOM layer never writes a `State` into the signal; it asks
        // `transition`, so the machine's tests cover every path the page can
        // take. `set(transition(&State::…, …))` is fine: that `State` is the
        // *before*, not the write.
        for (name, source) in [
            ("rambo.rs", include_str!("rambo.rs")),
            ("tasks/mod.rs", include_str!("tasks/mod.rs")),
            ("tasks/write.rs", include_str!("tasks/write.rs")),
            ("capture.rs", include_str!("capture.rs")),
        ] {
            let direct = direct_state_sets(source);
            assert!(
                direct.is_empty(),
                "{name} writes a State directly instead of asking `transition`: {direct:?}"
            );
        }
    }

    #[test]
    fn no_style_rule_moves_his_button() {
        // The engine owns his transform (engine/look.rs): a rule's transform or
        // animation on the button would fight it, or place him by itself and send him
        // to the window's top-left corner.
        let css = include_str!("../theme.css");
        let moved: Vec<_> = ["transform", "animation", "translate", "rotate", "scale"]
            .into_iter()
            .flat_map(|property| rambo_rules(css, property))
            .collect();
        assert!(moved.is_empty(), "a rule moves him: {moved:?}");
        assert!(
            !rambo_rules(css, "cursor").is_empty(),
            "the parser finds the Rambo rules"
        );
    }

    #[test]
    fn nothing_glides_him_from_place_to_place() {
        // He hops (`crate::hop`), drawn frame by frame. A transform transition on
        // the button would glide him instead, and also runs when a state's
        // animation ends, where Chrome starts it near the window's corner: every
        // drop sent him to the top-left and back.
        let eased: Vec<_> = rambo_rules(include_str!("../theme.css"), "transition")
            .into_iter()
            .filter(|(_, value)| value.contains("transform") || value.contains("all"))
            .collect();
        assert!(eased.is_empty(), "glides: {eased:?}");
    }

    /// docs/FRONTEND-PLAN.md 1.5: every z-index in the style sheets names a rung of the
    /// scale in `:root`, so no loose number can put Rambo over a dialog again.
    #[test]
    fn every_z_index_reads_the_layer_scale() {
        let css = include_str!("../theme.css");
        let loose: Vec<_> = css
            .lines()
            .filter(|line| line.trim_start().starts_with("z-index:"))
            .filter(|line| !line.contains("var(--layer-"))
            .collect();
        assert!(
            loose.is_empty(),
            "theme.css has loose z-index numbers: {loose:?}"
        );
    }

    /// Rambo and his bubbles sit under the edit sheet and the gear menu; the sheet is
    /// the highest rung a person can open.
    #[test]
    fn rambo_and_his_bubbles_sit_under_the_sheet() {
        let css = include_str!("../theme.css");
        let layer = |name: &str| -> i32 {
            css.lines()
                .find_map(|line| line.trim().strip_prefix(&format!("--layer-{name}:")))
                .and_then(|rest| rest.split(';').next())
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or_else(|| panic!("--layer-{name} is not a plain number in :root"))
        };
        assert!(layer("rambo") < layer("rambo-bubble"));
        assert!(layer("rambo-bubble") < layer("menu"));
        assert!(layer("menu") < layer("sheet"));
        assert!(layer("note") < layer("rambo"));
        // The capture bar is above everything sticky: day names.
        assert!(layer("sticky") < layer("capture"));
    }

    #[test]
    fn dazed_he_takes_every_hold_and_drag_whatever_is_under_the_finger() {
        use crate::gesture::Gesture;
        let tap = Gesture::Tap;
        let hold = Gesture::Hold;
        let drag = Gesture::Drag {
            pos: (10.0, 10.0),
            vel: (0.0, 0.0),
        };
        let drop = Gesture::Drop { pos: (10.0, 10.0) };
        let fling = Gesture::Fling { vel: (2.0, 0.0) };
        let row = Gesture::Down {
            target: crate::gesture::Target::Row("t1".into()),
            at: (10.0, 10.0),
        };
        let asking = State::Asking {
            task_id: "t1".into(),
            title: "Lab report".into(),
        };
        // (state, gesture, his whatever the target)
        let table = [
            (State::Dazed, &hold, true),
            (State::Dazed, &drag, true),
            (State::Dazed, &tap, false),
            (State::Dazed, &row, false),
            (State::Inking, &drag, true),
            (State::Inking, &drop, true),
            (State::Inking, &fling, true),
            (State::Inking, &Gesture::Cancel, true),
            (State::Home, &hold, false),
            (State::Home, &drag, false),
            (State::Dragging, &hold, false),
            (State::Splatting, &hold, false),
            (State::Skittering, &drag, false),
            (asking, &hold, false),
            (State::Flying, &drag, false),
            (State::WritingCaret, &hold, false),
            (State::WritingRow, &drag, false),
        ];
        for (state, gesture, his) in table {
            assert_eq!(wants(&state, gesture), his, "{state:?} {gesture:?}");
        }
    }

    #[test]
    fn he_offers_edit_move_and_done_and_nothing_else() {
        let labels: Vec<&str> = Choice::ALL.iter().map(|c| c.label()).collect();
        assert_eq!(labels, ["Edit", "Move", "Done"]);
    }

    #[test]
    fn a_tap_off_his_bubble_is_never_mind() {
        use crate::gesture::Gesture;
        let asking = State::Asking {
            task_id: "t1".into(),
            title: "Lab report".into(),
        };
        let drag = Gesture::Drag {
            pos: (0.0, 0.0),
            vel: (0.0, 0.0),
        };
        // (state, gesture, on his bubble, never mind)
        let table = [
            (&asking, &Gesture::Tap, false, true),
            (&asking, &Gesture::Tap, true, false),
            (&asking, &Gesture::Hold, false, false),
            (&asking, &drag, false, false),
            (&asking, &Gesture::Drop { pos: (0.0, 0.0) }, false, false),
            (&State::Home, &Gesture::Tap, false, false),
            (&State::Dazed, &Gesture::Tap, false, false),
        ];
        for (state, gesture, on_bubble, dismissed) in table {
            assert_eq!(
                asking_dismissed_by(state, gesture, on_bubble),
                dismissed,
                "{state:?} {gesture:?} on bubble: {on_bubble}"
            );
        }
        // Never mind sends him home.
        assert_eq!(transition(&asking, Event::Return), State::Home);
    }

    #[test]
    fn only_a_hold_while_dazed_draws_him_over_to_write() {
        use crate::gesture::Gesture;
        let gestures = [
            Gesture::Tap,
            Gesture::Hold,
            Gesture::Drag {
                pos: (40.0, 60.0),
                vel: (0.2, 0.0),
            },
            Gesture::Drop { pos: (40.0, 60.0) },
            Gesture::Cancel,
        ];
        let states = [
            State::Home,
            State::Dragging,
            State::Dazed,
            State::Inking,
            State::Splatting,
            State::Skittering,
            State::Flying,
            State::WritingCaret,
            State::WritingRow,
            State::Asking {
                task_id: "t1".into(),
                title: "Lab report".into(),
            },
        ];
        for state in &states {
            for gesture in &gestures {
                let drawn = drawn_over(state, gesture);
                let expected = *state == State::Dazed && matches!(gesture, Gesture::Hold);
                assert_eq!(drawn.is_some(), expected, "{state:?} {gesture:?}");
                // What draws him over is a gesture he wants, and it leaves him writing.
                if let Some(event) = drawn {
                    assert!(wants(state, gesture));
                    assert_eq!(transition(state, event), State::Inking);
                }
            }
        }
    }

    #[test]
    fn a_leaf_closing_on_him_out_on_the_page_traps_him_and_he_runs_home_undazed() {
        for out_on_the_page in [State::Dazed, State::Inking, State::Splatting] {
            assert_eq!(transition(&out_on_the_page, Event::Covered), State::Covered);
        }
        let asking = State::Asking {
            task_id: "t".into(),
            title: "x".into(),
        };
        assert_eq!(transition(&asking, Event::Covered), State::Covered);
        assert_eq!(
            transition(&State::Covered, Event::Uncovered),
            State::Home,
            "out from under it he is not dazed: he hops home, not skitters"
        );
        // Not on a page: in the nest, in hand, or in transit.
        for elsewhere in [
            State::Home,
            State::Dragging,
            State::Flying,
            State::WritingRow,
        ] {
            assert_eq!(transition(&elsewhere, Event::Covered), elsewhere);
        }
        // Stuck: he cannot be picked up from under the leaf.
        assert_eq!(transition(&State::Covered, Event::Grab), State::Covered);
        assert!(is_off_nest(&State::Covered));
    }
}

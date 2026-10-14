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
    /// Dropped on the page: he stays where he landed and does not listen.
    Dazed,
    /// A dazed Rambo being dragged: he writes on the page and does not listen.
    Inking,
    /// Dropped hard (fast release): ink splat + wobble, then dazed.
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
    /// Put back in his nest.
    DropNest,
    /// The splat is over (dazed), the daze is over (he skitters), or the
    /// skitter ends (home).
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
}

/// Milliseconds he stays dazed, untouched, before he skitters home on his own.
pub const DAZED_MS: u32 = 8000;
/// Milliseconds his hop home takes.
pub const SKITTER_MS: u32 = 900;
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
/// Minimum drop velocity (px/ms) to trigger a hard-drop splat.
pub const HARD_DROP_VELOCITY: f64 = 0.5;

pub fn transition(state: &State, event: Event) -> State {
    match (state, event) {
        // From home: grab or talk starts dragging; writing starts flying.
        (State::Home, Event::Grab | Event::Talk) => State::Dragging,
        (State::Home, Event::WriteCaret) => State::WritingCaret,
        (State::Home, Event::WriteRow) => State::Flying,
        (State::Home, Event::FlyHome) => State::Home,

        // Dragging: release or the nest goes home; drop on task asks; drop on
        // the page dazes him; drop hard splats first.
        (State::Dragging, Event::Release | Event::DropNest) => State::Home,
        (State::Dragging, Event::DropEmpty) => State::Dazed,
        (State::Dragging, Event::DropHard) => State::Splatting,
        (State::Dragging, Event::DropTask { task_id, title }) => State::Asking { task_id, title },

        // Splatting → Dazed → Skittering → Home.
        (State::Splatting, Event::Return) => State::Dazed,
        (State::Dazed, Event::Return) => State::Skittering,
        (State::Skittering, Event::Return) => State::Home,

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

/// Whether going from `before` to `after` is the user putting a dazed Rambo
/// back in his nest, which he forgives them a little for.
pub fn put_back(before: &State, after: &State) -> bool {
    matches!(before, State::Dazed | State::Inking) && *after == State::Home
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
            | State::Dazed
            | State::Inking
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
        State::Dazed => " is-dazed",
        State::Inking => " is-inking",
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
        State::Flying => Pose::Flying,
        State::Asking { .. } => Pose::Teasing,
        // Vacant: the nearest drawing to a daze until one is drawn.
        State::Dazed => Pose::Lobotomized,
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
    fn a_drop_on_the_page_dazes_him_where_he_lands_until_he_skitters_home() {
        let state = transition(&State::Home, Event::Grab);
        let state = transition(&state, Event::DropEmpty);
        assert_eq!(state, State::Dazed);
        assert!(is_off_nest(&state), "he stays where he landed");
        let state = transition(&state, Event::Return);
        assert_eq!(state, State::Skittering);
        assert_eq!(transition(&state, Event::Return), State::Home);
    }

    #[test]
    fn hard_drop_splats_then_dazes_him() {
        let state = transition(&State::Home, Event::Grab);
        let state = transition(&state, Event::DropHard);
        assert_eq!(state, State::Splatting);
        assert_eq!(transition(&state, Event::Return), State::Dazed);
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
    fn edit_in_the_bubble_listens_for_the_change() {
        let source = include_str!("rambo.rs");
        let (before, _) = source
            .split_once(">\"Edit\"</button>")
            .expect("the bubble has an Edit button");
        let handler = &before[before.rfind("<button").expect("Edit is a button")..];
        assert!(handler.contains("listen_for_edit("), "{handler}");
    }

    #[test]
    fn edit_in_the_ask_bubble_opens_the_edit_sheet() {
        // QA on 39eb0d9: "Edit" closed the bubble and bumped the task's priority
        // instead of opening the sheet. The button must ask the list for its sheet.
        let source = include_str!("rambo.rs");
        let (before, _) = source
            .split_once(">\"Edit\"</button>")
            .expect("the bubble has an Edit button");
        let handler = &before[before.rfind("<button").expect("Edit is a button")..];
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
    fn every_state_of_the_button_keeps_him_where_he_is() {
        // He is a fixed layer placed by --pen-x/--pen-y alone: a transform
        // without them sends him to the top-left corner of the window.
        let found = rambo_rules(include_str!("../theme.css"), "transform");
        assert!(
            found.len() > 5,
            "the parser found the Rambo rules: {found:?}"
        );
        let lost: Vec<_> = found
            .iter()
            .filter(|(_, value)| !value.contains("--pen-x"))
            .collect();
        assert!(lost.is_empty(), "moved off his place: {lost:?}");
    }

    #[test]
    fn only_a_flight_eases_him_from_place_to_place() {
        // A transform transition on the button also runs when a state's
        // animation ends, and Chrome starts it near the window's corner: every
        // drop sent him to the top-left and back. Only a flight may ease.
        let eased: Vec<_> = rambo_rules(include_str!("../theme.css"), "transition")
            .into_iter()
            .filter(|(selector, value)| {
                (value.contains("transform") || value.contains("all"))
                    && !selector.contains(".is-flying")
            })
            .collect();
        assert!(eased.is_empty(), "eases outside a flight: {eased:?}");
    }
}

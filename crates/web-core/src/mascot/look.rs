//! How he looks: the drawing for a state, mood and pen, and the class the element carries.

use super::*;

/// The folder a tool's drawings live in. The fountain pen is `fountain-pen` in
/// the settings and in `data-pen`, and `fountain` on disk.
pub(super) fn art_folder(pen: Pen) -> &'static str {
    match pen {
        Pen::FountainPen => "fountain",
        other => other.slug(),
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
        State::Inking => " is-inking",
        State::Splatting => " is-splatting",
        State::Skittering => " is-skittering",
        State::Asking { .. } => " is-asking",
        State::Noting { .. } => " is-noting",
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

    /// The drawing of Rambo in this pose, holding this tool, as the page shows it:
    /// `<svg><use href=this></use></svg>`.
    ///
    /// An `<img>` loads the SVG as a document of its own, so it never sees the
    /// page's tokens and Rambo follows the operating system instead of the
    /// theme. Content pulled in through `<use>` inherits them, and `#art` is the
    /// drawing's root group. Spec: docs/FRONTEND-PLAN.md FE-2.
    pub fn art(self, pen: Pen) -> String {
        format!("{}#art", self.art_file(pen))
    }

    /// The file the drawing lives in, for a caller that wants the path and not
    /// the reference (`<use>` takes the reference, everything else the path).
    pub fn art_file(self, pen: Pen) -> String {
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

    /// The `<use>` reference: the picture is drawn into the page, so it takes the page's
    /// tokens (`--rambo-fx`) and follows the theme, not the operating system.
    pub fn art(self) -> String {
        format!("{}#art", self.art_file())
    }

    /// The file the picture lives in.
    pub fn art_file(self) -> String {
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
        State::Dragging | State::Noting { .. } => Pose::Listening,
        State::WritingCaret | State::WritingRow | State::Inking => Pose::Writing,
        State::Flying | State::Thrown => Pose::Flying,
        State::Asking { .. } => Pose::Teasing,
        // Vacant: the nearest drawing to a daze until one is drawn.
        State::Dazed => Pose::Lobotomized,
        // Stuck and pushing, not dazed.
        State::Covered => Pose::Worried,
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

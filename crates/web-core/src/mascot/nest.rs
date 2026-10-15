//! Where the nest is and where he rests, hops or rides: plain geometry.

use super::*;

/// How far outside the nest's edge a drop still counts as putting him back.
pub const NEST_SLACK_PX: f64 = 16.0;

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
        | State::Asking { .. }
        | State::Noting { .. } => false,
        State::Home => !settled_home,
        State::Flying | State::WritingCaret | State::Skittering => true,
    }
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

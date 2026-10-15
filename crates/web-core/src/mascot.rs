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

/// The empty planner hour Rambo was dropped on: its date phrase, which the capture box
/// shows, and the instant a task noted there starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotedHour {
    pub phrase: String,
    pub at: chrono::DateTime<chrono::Utc>,
}

mod ask;
mod look;
mod nest;
mod state;
#[cfg(test)]
mod tests;

pub use ask::*;
pub use look::*;
pub use nest::*;
pub use state::*;

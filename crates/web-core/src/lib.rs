//! The web page's core (ARCHITECTURE 8A): the rules and the numbers the page runs on, with
//! no DOM, so `cargo test` covers them natively. `rambl-web` holds the views over it and
//! re-exports these modules under their old paths.

pub mod body;
pub mod clock;
#[cfg(test)]
mod contrast;
pub mod gesture;
pub mod hop;
pub mod ink;
pub mod mascot;
pub mod placing;
pub mod planner;
pub mod row;
pub mod settings;
pub mod store;
pub mod tuning;

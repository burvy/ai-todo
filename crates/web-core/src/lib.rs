//! The web page's core (ARCHITECTURE 8A): the rules and the numbers the page runs on, with
//! no DOM, so `cargo test` covers them natively. `rambl-web` holds the views over it and
//! re-exports these modules under their old paths.

pub mod body;
pub mod clock;
#[cfg(test)]
mod contrast;
pub mod dabs;
pub mod events;
pub mod gesture;
pub mod hop;
pub mod ink;
pub mod mascot;
pub mod pen;
pub mod placing;
pub mod planner;
pub mod rambo;
pub mod row;
pub mod sched;
pub mod settings;
pub mod spring;
pub mod store;
pub mod task_list;
pub mod tuning;
pub mod week;

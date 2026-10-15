//! The web page's core (ARCHITECTURE 8A): the rules and the numbers the page runs on, with
//! no DOM, so `cargo test` covers them natively. `rambl-web` holds the views over it and
//! re-exports these modules under their old paths.

pub mod clock;
pub mod gesture;
pub mod hop;
pub mod tuning;

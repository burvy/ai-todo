//! Deterministic scheduling over local working hours.

mod hours;
mod plan;
mod series;

pub use hours::{TimeWindow, WorkingHours};
pub use plan::{plan, usual_time};
pub use series::expand;

#[cfg(test)]
mod tests;

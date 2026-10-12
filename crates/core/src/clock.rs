//! The only place that reads the wall clock. Everything else takes a [`Clock`].

use chrono::{DateTime, Utc};

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// The real clock. Used by the server only.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock frozen at one instant. Tests use it so day math is repeatable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedClock(pub DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn fixed_clock_returns_its_instant() {
        let at = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        assert_eq!(FixedClock(at).now(), at);
    }
}

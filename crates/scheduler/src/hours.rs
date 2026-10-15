//! The working day: when tasks may be placed, and local clock times in a zone.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use std::collections::HashMap;
use todo_core::tuning::{WORKDAY_END, WORKDAY_START};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeWindow {
    pub start: NaiveTime,
    pub end: NaiveTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkingHours {
    pub timezone: Tz,
    pub days: HashMap<Weekday, Vec<TimeWindow>>,
    /// Where on its day a day-only task goes when the user has no timed tasks to
    /// learn a usual time from (`DaySettings::placement`).
    pub day_only_time: NaiveTime,
}

impl WorkingHours {
    pub fn every_day(timezone: Tz, start: NaiveTime, end: NaiveTime) -> Self {
        let days = Weekday::iter()
            .map(|day| (day, vec![TimeWindow { start, end }]))
            .collect();
        Self {
            timezone,
            days,
            day_only_time: todo_core::DaySettings::default().placement,
        }
    }
}

impl Default for WorkingHours {
    fn default() -> Self {
        Self::every_day(chrono_tz::UTC, WORKDAY_START, WORKDAY_END)
    }
}

pub(crate) fn local_to_utc(
    timezone: Tz,
    date: NaiveDate,
    time: NaiveTime,
) -> Option<DateTime<Utc>> {
    match timezone.from_local_datetime(&date.and_time(time)) {
        chrono::LocalResult::Single(value) | chrono::LocalResult::Ambiguous(value, _) => {
            Some(value.with_timezone(&Utc))
        }
        chrono::LocalResult::None => None,
    }
}

trait WeekdayIter {
    fn iter() -> std::array::IntoIter<Weekday, 7>;
}

impl WeekdayIter for Weekday {
    fn iter() -> std::array::IntoIter<Weekday, 7> {
        [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ]
        .into_iter()
    }
}

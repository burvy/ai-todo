//! Expanding a repeating series into the occurrences inside a window.

use chrono::{Duration, NaiveDate};
use chrono_tz::Tz;
use std::str::FromStr;
use todo_core::{Occurrence, OccurrenceStatus, When};

/// The weekly occurrences of one series between `window_start` (inclusive) and
/// `window_end` (exclusive), with the exceptions applied (ARCHITECTURE.md 7.1).
///
/// Pure and deterministic: no wall clock, no store. Each occurrence's instant is
/// derived in the zone stored on the series, so a series keeps its local time
/// across DST: 07:00 stays 07:00 while the UTC instant shifts. An undone past
/// occurrence is never carried forward — it stays a fixed block on its own day,
/// and only one-shot tasks can become overdue.
pub fn expand(
    series: &todo_core::Series,
    window_start: NaiveDate,
    window_end: NaiveDate,
    exceptions: &[todo_core::SeriesException],
    ctx: &todo_core::LocalContext,
) -> Vec<Occurrence> {
    // A zone that will not parse (a hand-edited row) falls back to the zone the
    // request came in; an occurrence at a slightly shifted time beats no data.
    let zone = Tz::from_str(&series.zone).unwrap_or(ctx.tz);
    let local = todo_core::LocalContext {
        now: ctx.now,
        tz: zone,
    };
    let at = |date: NaiveDate| match series.time_of_day {
        Some(time) => When::At(local.at(date, time)),
        None => When::Anytime,
    };
    (0..)
        .map(|offset| window_start + Duration::days(offset))
        .take_while(|date| *date < window_end)
        .filter(|date| series.rule.covers(series.start_day, *date))
        .filter_map(|date| {
            let exception = exceptions.iter().find(|held| held.date == date);
            let (status, when, due_tz) = match exception.map(|held| &held.kind) {
                Some(todo_core::ExceptionKind::Skipped) => return None,
                Some(todo_core::ExceptionKind::Done) => (OccurrenceStatus::Done, at(date), None),
                Some(todo_core::ExceptionKind::Moved { to, to_tz }) => {
                    (OccurrenceStatus::Moved, *to, to_tz.clone())
                }
                None => (OccurrenceStatus::Open, at(date), None),
            };
            Some(Occurrence {
                series: series.id.clone(),
                date,
                when,
                status,
                due_tz,
            })
        })
        .collect()
}

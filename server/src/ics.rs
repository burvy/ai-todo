//! Turning an exported `.ics` file into the busy blocks the scheduler reasons
//! about.
//!
//! Everything is converted to UTC, and getting that wrong is not a rounding
//! error: Outlook writes `DTSTART;TZID=America/New_York:20261012T100000`, so
//! the zone lives in a *parameter* and not in the value. Read it as if it were
//! UTC and every event shifts by four hours, which is far enough for the
//! planner to book a two-hour block straight over the top of a lecture.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, TimeZone, Utc};
use icalendar::{CalendarDateTime, Component, DatePerhapsTime, Event};

use crate::db::BusyRow;

/// A calendar we could not read. A malformed upload is the caller's doing, so
/// this is a 400 rather than a 500.
#[derive(Debug)]
pub struct ImportError(pub String);

impl IntoResponse for ImportError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": self.0 })),
        )
            .into_response()
    }
}

/// Parse the events of an `.ics` export into busy blocks, in UTC, oldest
/// first.
///
/// Events that cannot be placed are skipped rather than guessed at: a missing
/// block is easy to spot in the planner, a wrongly-timed one silently ruins
/// the schedule.
pub fn parse_ics(text: &str) -> Result<Vec<BusyRow>, ImportError> {
    let calendar: icalendar::Calendar = text
        .parse()
        .map_err(|error| ImportError(format!("that is not an iCalendar file: {error}")))?;

    let mut busy: Vec<BusyRow> = calendar
        .events()
        .filter_map(|event| {
            let start = to_utc(&date_of(event, "DTSTART")?)?;
            // `DTEND` is the norm, but `DURATION` is legal and Google uses it.
            // A missing `DTEND` must not drop the event, so no `?` here.
            let end = date_of(event, "DTEND")
                .and_then(|value| to_utc(&value))
                .or_else(|| duration_after(event, start))
                .unwrap_or(start);
            if end < start {
                return None;
            }
            Some(BusyRow {
                start,
                end,
                title: summary_of(event),
            })
        })
        .collect();

    busy.sort_by_key(|block| block.start);
    Ok(busy)
}

fn date_of(event: &Event, key: &str) -> Option<DatePerhapsTime> {
    DatePerhapsTime::from_property(event.properties().get(key)?)
}

fn summary_of(event: &Event) -> String {
    event
        .properties()
        .get("SUMMARY")
        .map(|property| property.value().trim())
        .filter(|summary| !summary.is_empty())
        .unwrap_or("Busy")
        .to_string()
}

/// Convert any of the three iCalendar date forms to UTC.
fn to_utc(value: &DatePerhapsTime) -> Option<DateTime<Utc>> {
    match value {
        DatePerhapsTime::DateTime(CalendarDateTime::Utc(date_time)) => Some(*date_time),
        DatePerhapsTime::DateTime(CalendarDateTime::WithTimezone { date_time, tzid }) => {
            let zone: chrono_tz::Tz = tzid.parse().ok()?;
            // `.single()` gives up when a DST change makes this local time
            // ambiguous or nonexistent. `.earliest()` keeps the event, which
            // beats silently dropping a lecture because it happens to fall on
            // the spring-forward morning.
            let local = zone.from_local_datetime(date_time).earliest()?;
            Some(local.with_timezone(&Utc))
        }
        // A floating time names no zone at all. UTC is the only reading that
        // is not a guess about where the reader happens to be.
        DatePerhapsTime::DateTime(CalendarDateTime::Floating(date_time)) => {
            Some(Utc.from_utc_datetime(date_time))
        }
        // An all-day event (`VALUE=DATE`) has no time. Midnight UTC is a
        // placeholder; the scheduler only reads start and end.
        DatePerhapsTime::Date(date) => date
            .and_hms_opt(0, 0, 0)
            .map(|naive| Utc.from_utc_datetime(&naive)),
    }
}

/// `DURATION` is the alternative to `DTEND`, written like `PT1H30M`.
fn duration_after(event: &Event, start: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let raw = event.properties().get("DURATION")?.value().trim();
    let mut seconds: i64 = 0;
    let mut digits = String::new();
    for character in raw.trim_start_matches('P').trim_start_matches('T').chars() {
        match character {
            '0'..='9' => digits.push(character),
            'H' | 'M' | 'S' | 'D' | 'W' => {
                let value = digits.parse::<i64>().ok()?;
                seconds += value
                    * match character {
                        'H' => 3600,
                        'M' => 60,
                        'D' => 86_400,
                        'W' => 604_800,
                        _ => 1,
                    };
                digits.clear();
            }
            _ => return None,
        }
    }
    Some(start + chrono::Duration::seconds(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEEK: &str = include_str!("../../demo/outlook/week.ics");

    #[test]
    fn the_demo_week_has_nine_events_spanning_the_work_week() {
        let busy = parse_ics(WEEK).expect("the Outlook fixture parses");

        assert_eq!(busy.len(), 9, "expected 9 events in demo/outlook/week.ics");

        let first = busy.first().expect("at least one event").start;
        let last = busy.last().expect("at least one event").start;
        assert_eq!(
            first
                .with_timezone(&chrono_tz::Tz::America__New_York)
                .date_naive()
                .to_string(),
            "2026-10-12",
            "week should open on Monday 12 October"
        );
        assert_eq!(
            last.with_timezone(&chrono_tz::Tz::America__New_York)
                .date_naive()
                .to_string(),
            "2026-10-16",
            "week should close on Friday 16 October"
        );
    }

    #[test]
    fn new_york_times_are_converted_not_read_as_utc() {
        let busy = parse_ics(WEEK).expect("the Outlook fixture parses");

        // DTSTART;TZID=America/New_York:20261012T100000 is EDT (UTC-4).
        let lecture = busy
            .iter()
            .find(|block| block.title.contains("BIO 214"))
            .expect("the first lecture is in the file");
        assert_eq!(
            lecture.start.to_rfc3339(),
            "2026-10-12T14:00:00+00:00",
            "10:00 New York in October is 14:00 UTC"
        );
        assert_eq!(
            lecture.end.to_rfc3339(),
            "2026-10-12T15:15:00+00:00",
            "a 75 minute lecture stays 75 minutes"
        );
    }

    #[test]
    fn events_come_back_in_start_order() {
        let busy = parse_ics(WEEK).expect("the Outlook fixture parses");
        let sorted = busy.windows(2).all(|pair| pair[0].start <= pair[1].start);
        assert!(sorted, "blocks should be oldest first");
    }

    #[test]
    fn a_utc_time_is_kept_exactly_as_written() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\n\
            SUMMARY:Lab\n\
            DTSTART:20261012T140000Z\n\
            DTEND:20261012T150000Z\n\
            END:VEVENT\nEND:VCALENDAR\n";
        let busy = parse_ics(ics).expect("parses");
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].start.to_rfc3339(), "2026-10-12T14:00:00+00:00");
        assert_eq!(busy[0].end.to_rfc3339(), "2026-10-12T15:00:00+00:00");
    }

    #[test]
    fn a_floating_time_is_treated_as_utc() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\n\
            SUMMARY:Drop in\n\
            DTSTART:20261012T090000\n\
            DTEND:20261012T100000\n\
            END:VEVENT\nEND:VCALENDAR\n";
        let busy = parse_ics(ics).expect("parses");
        assert_eq!(busy[0].start.to_rfc3339(), "2026-10-12T09:00:00+00:00");
    }

    #[test]
    fn a_duration_is_used_when_there_is_no_dtend() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\n\
            SUMMARY:Seminar\n\
            DTSTART:20261012T140000Z\n\
            DURATION:PT1H30M\n\
            END:VEVENT\nEND:VCALENDAR\n";
        let busy = parse_ics(ics).expect("parses");
        assert_eq!(busy[0].end.to_rfc3339(), "2026-10-12T15:30:00+00:00");
    }

    #[test]
    fn an_all_day_event_still_blocks_the_day() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\n\
            SUMMARY:Reading week\n\
            DTSTART;VALUE=DATE:20261013\n\
            DTEND;VALUE=DATE:20261014\n\
            END:VEVENT\nEND:VCALENDAR\n";
        let busy = parse_ics(ics).expect("parses");
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].start.to_rfc3339(), "2026-10-13T00:00:00+00:00");
    }

    #[test]
    fn an_event_with_no_summary_is_still_busy() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\n\
            DTSTART:20261012T140000Z\nDTEND:20261012T150000Z\n\
            END:VEVENT\nEND:VCALENDAR\n";
        let busy = parse_ics(ics).expect("parses");
        assert_eq!(busy[0].title, "Busy");
    }

    #[test]
    fn junk_is_an_error_not_an_empty_calendar() {
        // An empty busy list would look like "you have a free week", which is
        // worse than being told the upload was not a calendar at all.
        let error = parse_ics("this is not a calendar").expect_err("should fail");
        assert!(error.0.contains("iCalendar"), "got: {}", error.0);
    }

    #[test]
    fn an_event_that_ends_before_it_starts_is_dropped() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\n\
            SUMMARY:Backwards\n\
            DTSTART:20261012T150000Z\nDTEND:20261012T140000Z\n\
            END:VEVENT\n\
            BEGIN:VEVENT\n\
            SUMMARY:Fine\n\
            DTSTART:20261012T160000Z\nDTEND:20261012T170000Z\n\
            END:VEVENT\nEND:VCALENDAR\n";
        let busy = parse_ics(ics).expect("parses");
        assert_eq!(busy.len(), 1, "only the sane event should survive");
        assert_eq!(busy[0].title, "Fine");
    }
}

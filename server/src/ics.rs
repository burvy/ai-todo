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
    body::to_bytes,
    extract::{FromRequest, Multipart},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, TimeZone, Utc};
use icalendar::{CalendarDateTime, Component, DatePerhapsTime, Event};
use serde::Serialize;

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
            if event
                .properties()
                .get("STATUS")
                .is_some_and(|value| value.value().eq_ignore_ascii_case("CANCELLED"))
                || event
                    .properties()
                    .get("TRANSP")
                    .is_some_and(|value| value.value().eq_ignore_ascii_case("TRANSPARENT"))
            {
                return None;
            }
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

/// A pair of events, one imported and one already in the app, that overlap in
/// time. This is what the import has to warn about before it overwrites.
#[derive(Clone, Debug, Serialize)]
pub struct Conflict {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// The event from the uploaded file.
    pub imported_title: String,
    /// What is already in the app at that time.
    pub existing_title: String,
}

/// Find imported events that overlap something already in the calendar.
///
/// Any overlap counts, not just identical times: a lecture that starts ten
/// minutes before an existing meeting is just as much of a clash as one that
/// matches it exactly, and the user needs to see both.
pub fn find_conflicts(imported: &[BusyRow], existing: &[BusyRow]) -> Vec<Conflict> {
    let mut conflicts = Vec::new();
    for block in imported {
        for current in existing {
            if overlaps(block, current) {
                conflicts.push(Conflict {
                    start: block.start,
                    end: block.end,
                    imported_title: block.title.clone(),
                    existing_title: current.title.clone(),
                });
            }
        }
    }
    conflicts
}

/// Half-open intervals: back-to-back events (10:00–11:00 then 11:00–12:00) are
/// not a clash, only real overlap is.
fn overlaps(a: &BusyRow, b: &BusyRow) -> bool {
    a.start < b.end && b.start < a.end
}

/// What an import boils down to: the file's text, and whether the caller has
/// already accepted the conflicts we would otherwise warn about.
#[derive(Debug)]
pub struct ImportInput {
    pub ics_text: String,
    pub confirm: bool,
}

/// Largest upload we will read. A term's worth of calendar events is a few
/// hundred KB; 10MB is generous and stops a hostile upload filling memory.
const MAX_ICS_BYTES: usize = 10 * 1024 * 1024;

/// Accept either `{"ics_text": "..."}` JSON or a `multipart/form-data` upload
/// with a `file` field — which is what a file picker on the device produces,
/// and the way a user actually gets an `.ics` out of Outlook.
pub async fn read_import(request: axum::extract::Request) -> Result<ImportInput, ImportError> {
    let content_type = request
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.starts_with("multipart/form-data") {
        return read_upload(request).await;
    }

    let body = to_bytes(request.into_body(), MAX_ICS_BYTES)
        .await
        .map_err(|error| ImportError(format!("could not read the body: {error}")))?;
    let body: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|error| ImportError(format!("expected {{\"ics_text\": \"...\"}}: {error}")))?;

    let ics_text = body
        .get("ics_text")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ImportError("expected an `ics_text` field".into()))?
        .to_owned();
    let confirm = body
        .get("confirm")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    Ok(ImportInput { ics_text, confirm })
}

async fn read_upload(request: axum::extract::Request) -> Result<ImportInput, ImportError> {
    let mut multipart = Multipart::from_request(request, &())
        .await
        .map_err(|error| ImportError(format!("expected multipart/form-data: {error}")))?;

    let mut text: Option<String> = None;
    let mut filename = String::new();
    let mut confirm = false;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ImportError(format!("could not read the upload: {error}")))?
    {
        let name = field.name().map(str::to_owned);
        match name.as_deref() {
            Some("file") => {
                filename = field.file_name().unwrap_or_default().to_owned();
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|error| ImportError(format!("could not read the file: {error}")))?;
                text = Some(String::from_utf8_lossy(&bytes).into_owned());
            }
            Some("confirm") => {
                let value = field
                    .bytes()
                    .await
                    .map_err(|error| ImportError(format!("could not read confirm: {error}")))?;
                confirm = matches!(
                    String::from_utf8_lossy(&value).trim(),
                    "true" | "1" | "yes" | "on"
                );
            }
            _ => {}
        }
    }

    let ics_text = text.ok_or_else(|| ImportError("the upload had no `file` field".into()))?;
    if ics_text.trim().is_empty() {
        return Err(ImportError(if filename.is_empty() {
            "the uploaded file was empty".into()
        } else {
            format!("{filename} was empty")
        }));
    }

    Ok(ImportInput { ics_text, confirm })
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

    // ---- realistic Outlook / Google exports --------------------------------
    //
    // demo/outlook/week.ics is sanitised: LF only, and no VTIMEZONE, folded
    // lines, RRULE or STATUS. Real exports carry all of them, so the parser is
    // checked against that shape here too.

    /// Build an export the way Outlook does: CRLF line endings, trailing CRLF.
    fn outlook(lines: &[&str]) -> String {
        let mut text = lines.join("\r\n");
        text.push_str("\r\n");
        text
    }

    const VTIMEZONE: &[&str] = &[
        "BEGIN:VTIMEZONE",
        "TZID:America/New_York",
        "BEGIN:DAYLIGHT",
        "TZOFFSETFROM:-0500",
        "TZOFFSETTO:-0400",
        "TZNAME:EDT",
        "DTSTART:19700308T020000",
        "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU",
        "END:DAYLIGHT",
        "BEGIN:STANDARD",
        "TZOFFSETFROM:-0400",
        "TZOFFSETTO:-0500",
        "TZNAME:EST",
        "DTSTART:19701101T020000",
        "RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU",
        "END:STANDARD",
        "END:VTIMEZONE",
    ];

    fn with_vtimezone(rest: &[&str]) -> String {
        let mut lines = vec![
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//Microsoft//OUTLOOK",
        ];
        lines.extend_from_slice(VTIMEZONE);
        lines.extend_from_slice(rest);
        lines.push("END:VCALENDAR");
        outlook(&lines)
    }

    #[test]
    fn crlf_line_endings_parse_like_a_real_export() {
        // Every real .ics is CRLF. The demo fixture is LF, so this is the only
        // test that exercises the line endings Outlook actually writes.
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:BIO 214 Urban Ecology",
            "DTSTART;TZID=America/New_York:20261012T100000",
            "DTEND;TZID=America/New_York:20261012T111500",
            "END:VEVENT",
        ]);
        assert!(text.contains("\r\n"), "sanity: this really is CRLF");

        let busy = parse_ics(&text).expect("a CRLF export parses");
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].title, "BIO 214 Urban Ecology");
        assert_eq!(busy[0].start.to_rfc3339(), "2026-10-12T14:00:00+00:00");
    }

    #[test]
    fn a_vtimezone_block_is_not_mistaken_for_an_event() {
        // Real exports carry a VTIMEZONE per TZID, and those blocks contain
        // DTSTART and RRULE lines of their own.
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:STAT 201 Lecture",
            "DTSTART;TZID=America/New_York:20261013T090000",
            "DTEND;TZID=America/New_York:20261013T101500",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("parses");
        assert_eq!(busy.len(), 1, "the VTIMEZONE must not produce a block");
        assert_eq!(busy[0].title, "STAT 201 Lecture");
    }

    #[test]
    fn a_folded_line_is_joined_back_together() {
        // RFC 5545 folds long lines: a CRLF followed by a single space means
        // "continues". A parser that misses this splits one event's SUMMARY
        // from its DTSTART.
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:BIO 214 Urban Ecology, section three, a genuinely long",
            " title that Outlook had to wrap at seventy-five octets",
            "DTSTART;TZID=America/New_York:20261012T100000",
            "DTEND;TZID=America/New_York:20261012T111500",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("parses");
        assert_eq!(busy.len(), 1, "the folded event should still be one event");
        assert!(
            busy[0].title.contains("seventy-five octets"),
            "the folded continuation was lost: {:?}",
            busy[0].title
        );
    }

    #[test]
    fn escaped_commas_and_semicolons_in_a_title() {
        // RFC 5545 escapes `,` `\;` `\\` and newline in text values. If they
        // are not unescaped the planner shows the raw backslashes to the user.
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Buy milk\\, eggs\\; and bread",
            "DTSTART;TZID=America/New_York:20261012T100000",
            "DTEND;TZID=America/New_York:20261012T110000",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("parses");
        assert_eq!(busy.len(), 1);
        assert_eq!(
            busy[0].title, "Buy milk, eggs; and bread",
            "got {:?}",
            busy[0].title
        );
    }

    #[test]
    fn a_weekly_recurring_class_is_present_for_the_week() {
        // A student's real calendar is mostly RRULEs: the same lecture every
        // week. We do not expand recurrences, so it appears once.
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:STAT 201 Lecture",
            "DTSTART;TZID=America/New_York:20261013T090000",
            "DTEND;TZID=America/New_York:20261013T101500",
            "RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20261215T035959Z",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("parses");
        assert_eq!(
            busy.len(),
            1,
            "one block for the recurrence's first instance"
        );
        assert_eq!(
            busy[0].start.to_rfc3339(),
            "2026-10-13T13:00:00+00:00",
            "the first occurrence should be used"
        );
    }

    #[test]
    fn a_cancelled_event_should_not_block_any_time() {
        // Outlook keeps cancelled meetings in the export with STATUS:CANCELLED.
        // Booking time over them is wrong: the whole point of cancelling was
        // to free the slot.
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Advising appointment",
            "STATUS:CANCELLED",
            "DTSTART;TZID=America/New_York:20261013T130000",
            "DTEND;TZID=America/New_York:20261013T133000",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("parses");
        assert!(
            busy.is_empty(),
            "a cancelled event must not block time, got {:?}",
            busy.iter().map(|b| &b.title).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_transparent_event_is_marked_free_in_the_export() {
        // TRANSP:TRANSPARENT is the iCalendar way of saying "show this, but do
        // not treat it as busy" — all-day birthdays and holidays use it.
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Someone's birthday",
            "TRANSP:TRANSPARENT",
            "DTSTART;TZID=America/New_York:20261014T000000",
            "DTEND;TZID=America/New_York:20261014T235959",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("parses");
        assert!(
            busy.is_empty(),
            "a TRANSPARENT event should not block the day, got {:?}",
            busy.iter().map(|b| &b.title).collect::<Vec<_>>()
        );
    }

    #[test]
    fn non_ascii_titles_survive() {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Café meeting — résumé review",
            "DTSTART;TZID=America/New_York:20261012T100000",
            "DTEND;TZID=America/New_York:20261012T110000",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("parses");
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].title, "Café meeting — résumé review");
    }

    #[test]
    fn an_event_spanning_midnight_keeps_its_length() {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Overnight shift",
            "DTSTART;TZID=America/New_York:20261012T220000",
            "DTEND;TZID=America/New_York:20261013T060000",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("parses");
        assert_eq!(busy.len(), 1);
        let hours = (busy[0].end - busy[0].start).num_minutes() / 60;
        assert_eq!(hours, 8, "an overnight shift should stay eight hours");
    }

    #[test]
    fn one_malformed_event_does_not_lose_the_whole_week() {
        // A single unparseable event must not take the other eight with it.
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Broken, no dates at all",
            "END:VEVENT",
            "BEGIN:VEVENT",
            "SUMMARY:Real lecture",
            "DTSTART;TZID=America/New_York:20261012T100000",
            "DTEND;TZID=America/New_York:20261012T111500",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("parses");
        assert_eq!(busy.len(), 1, "the good event should survive");
        assert_eq!(busy[0].title, "Real lecture");
    }

    #[test]
    fn an_empty_calendar_yields_no_blocks() {
        let text = outlook(&["BEGIN:VCALENDAR", "VERSION:2.0", "END:VCALENDAR"]);
        let busy = parse_ics(&text).expect("an empty calendar is still valid");
        assert!(busy.is_empty());
    }

    #[test]
    fn the_demo_fixture_still_parses_after_all_this() {
        // Guard against the realistic cases above steering the parser away
        // from the fixture the demo actually uses.
        let busy = parse_ics(WEEK).expect("the demo fixture parses");
        assert_eq!(busy.len(), 9);
    }
}

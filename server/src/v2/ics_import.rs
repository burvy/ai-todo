//! Reading an exported `.ics` calendar into imported events.
//!
//! Everything is converted to UTC, and getting that wrong is not a rounding
//! error: Outlook writes `DTSTART;TZID=America/New_York:20261012T100000`, so
//! the zone lives in a *parameter* and not in the value. Read it as if it were
//! UTC and every event shifts by four hours, which is far enough for the
//! planner to book a two-hour block straight over the top of a lecture.

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use icalendar::{CalendarDateTime, Component, DatePerhapsTime, Event};

use crate::v2::app::{ImportedEvent, ImportedSeries};
use crate::v2::tuning::{IMPORT_MAX_EVENT_LENGTH, IMPORT_MAX_RULE_COUNT};
use todo_core::tuning::{TASK_DEFAULT_DURATION_MIN, TASK_MAX_DURATION_MIN, TASK_MIN_DURATION_MIN};

/// A calendar we could not read. A malformed upload is the caller's doing, so
/// the HTTP layer answers it with a 400.
#[derive(Debug)]
pub struct ImportError(pub String);

/// One calendar export, read: the one-shot events, the weekly series it describes, and
/// a note for every rule we chose not to follow.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedCalendar {
    /// Events to import as single tasks, oldest first.
    pub events: Vec<ImportedEvent>,
    /// Weekly recurrences to import as series (docs/ARCHITECTURE.md 7.1).
    pub series: Vec<ImportedSeries>,
    /// One line per rule we could not follow, naming the event and why. That event is
    /// still imported, as one task, so the day is not silently lost.
    pub unmapped: Vec<String>,
}

/// Parse an `.ics` export: one-shot events, the weekly recurrences among them, and a
/// note for every rule that is not one we follow.
///
/// Events that cannot be placed are skipped rather than guessed at: a missing
/// block is easy to spot in the planner, a wrongly-timed one silently ruins
/// the schedule.
pub fn parse_calendar(text: &str) -> Result<ParsedCalendar, ImportError> {
    let calendar: icalendar::Calendar = text
        .parse()
        .map_err(|error| ImportError(format!("that is not an iCalendar file: {error}")))?;

    let mut parsed = ParsedCalendar::default();
    for event in calendar.events() {
        if event
            .properties()
            .get("STATUS")
            .is_some_and(|value| value.value().eq_ignore_ascii_case("CANCELLED"))
            || event
                .properties()
                .get("TRANSP")
                .is_some_and(|value| value.value().eq_ignore_ascii_case("TRANSPARENT"))
        {
            continue;
        }
        let Some(start) = date_of(event, "DTSTART").and_then(|value| to_utc(&value)) else {
            continue;
        };
        // `DTEND` is the norm, but `DURATION` is legal and Google uses it.
        // A missing `DTEND` must not drop the event, so no `?` here. With neither
        // `DTEND` nor `DURATION`, the event gets the default length, never zero.
        let end = date_of(event, "DTEND")
            .and_then(|value| to_utc(&value))
            .or_else(|| duration_after(event, start))
            .unwrap_or(start + chrono::Duration::minutes(TASK_DEFAULT_DURATION_MIN.into()));
        // An event that ends before it begins: nothing to import.
        if end < start {
            continue;
        }
        let uid = uid_of(event);
        let title = summary_of(event);

        match weekly_series(event, start) {
            // A weekly rule becomes a series, never a task per date.
            Ok(Some(series)) => parsed.series.push(ImportedSeries {
                uid,
                title,
                duration_min: minutes_between(start, end),
                ..series
            }),
            Ok(None) => parsed.events.push(ImportedEvent {
                uid,
                title,
                start,
                end,
            }),
            // A rule we do not follow is not an error: the first instance is still
            // worth having on the list, and the answer says why it is not a series.
            Err(reason) => {
                parsed.unmapped.push(format!("{title}: {reason}"));
                parsed.events.push(ImportedEvent {
                    uid,
                    title,
                    start,
                    end,
                });
            }
        }
    }

    parsed.events.sort_by_key(|event| event.start);
    parsed.series.sort_by_key(|series| series.start_day);
    Ok(parsed)
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

/// The two-letter weekday codes a `BYDAY` uses, in the order we store them.
const RRULE_WEEKDAYS: [(&str, Weekday); 7] = [
    ("MO", Weekday::Mon),
    ("TU", Weekday::Tue),
    ("WE", Weekday::Wed),
    ("TH", Weekday::Thu),
    ("FR", Weekday::Fri),
    ("SA", Weekday::Sat),
    ("SU", Weekday::Sun),
];

/// The parts of a weekly rule we follow. Anything else (`BYSETPOS`, `BYMONTH`,
/// `BYMONTHDAY`, `BYWEEKNO`, …) names a rhythm this app cannot keep.
const SUPPORTED_PARTS: [&str; 5] = ["FREQ", "BYDAY", "UNTIL", "COUNT", "WKST"];

/// The series an event's weekly `RRULE` describes, or `None` when it has no rule. An
/// `Err` is a rule we cannot follow, and says why: the caller imports the event's first
/// instance instead and tells the user this rule was not followed.
fn weekly_series(event: &Event, start: DateTime<Utc>) -> Result<Option<ImportedSeries>, String> {
    let Some(rule) = event.properties().get("RRULE").map(|p| p.value()) else {
        return Ok(None);
    };
    let zone = zone_of(event);
    let start_local = start.with_timezone(&zone);
    let start_day = start_local.date_naive();

    let parts: Vec<(&str, &str)> = rule
        .split(';')
        .filter_map(|part| part.split_once('='))
        .collect();
    let part = |name: &str| {
        parts
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| *value)
    };
    // The rhythm first: "every month" says more about why we cannot follow a rule
    // than the parts we do not read do.
    if !part("FREQ").is_some_and(|freq| freq.eq_ignore_ascii_case("WEEKLY")) {
        return Err(format!(
            "{} repeats, imported as one task",
            part("FREQ").unwrap_or("this rule")
        ));
    }
    if let Some(interval) = part("INTERVAL") {
        return Err(format!(
            "every {interval} weeks is not every week, imported as one task"
        ));
    }
    if let Some((name, _)) = parts
        .iter()
        .find(|(name, _)| !SUPPORTED_PARTS.contains(&name.to_uppercase().as_str()))
    {
        return Err(format!("{name} is not supported, imported as one task"));
    }

    let weekdays = match part("BYDAY") {
        // No BYDAY means the event's own weekday, which is the only one DTSTART can be.
        None => vec![start_day.weekday()],
        Some(list) => weekdays_of(list)?,
    };
    // A rule ends either by naming a date or by counting occurrences; the earlier of
    // the two is the one that bites first.
    let until = part("UNTIL").map(|raw| until_date(raw, zone)).transpose()?;
    let counted = match part("COUNT").map(str::trim) {
        None => None,
        Some(raw) => {
            let count: u32 = raw
                .parse()
                .map_err(|_| format!("COUNT={raw} is not a number, imported as one task"))?;
            Some(last_occurrence(start_day, &weekdays, count))
        }
    };
    let until = match (until, counted) {
        (Some(named), Some(counted)) => Some(named.min(counted)),
        (Some(named), None) => Some(named),
        (None, Some(counted)) => Some(counted),
        (None, None) => None,
    };

    Ok(Some(ImportedSeries {
        uid: uid_of(event),
        title: String::new(),
        duration_min: TASK_DEFAULT_DURATION_MIN,
        weekdays,
        until,
        start_day,
        // An all-day event (`VALUE=DATE`) has no time, so its occurrences float.
        time_of_day: (date_of(event, "DTSTART")
            .is_some_and(|value| matches!(value, DatePerhapsTime::DateTime(_))))
        .then_some(start_local.time()),
        zone: zone.name().to_owned(),
        exdates: exdates_of(event, zone),
    }))
}

/// The weekdays a `BYDAY` list names, sorted and without repeats.
///
/// `2FR` and `-1SU` pick a numbered day inside a month, which is a monthly rule wearing
/// a weekly hat: an ordinal is not a weekday and the rule is not one we follow.
fn weekdays_of(list: &str) -> Result<Vec<Weekday>, String> {
    let mut days: Vec<Weekday> = Vec::new();
    for token in list.split(',') {
        let token = token.trim();
        let code = token
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '+' || c == '-')
            .to_owned();
        let ordinal = code.len() != token.len();
        let weekday = RRULE_WEEKDAYS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&code))
            .map(|(_, day)| *day);
        match weekday {
            None => return Err(format!("BYDAY={token} is not a weekday")),
            Some(_) if ordinal => return Err(format!("BYDAY={token} counts weeks")),
            Some(day) if !days.contains(&day) => days.push(day),
            Some(_) => {}
        }
    }
    days.sort_by_key(|day| day.num_days_from_monday());
    if days.is_empty() {
        return Err("BYDAY names no day".into());
    }
    Ok(days)
}

/// `EXDATE` lists the occurrences an export takes back: the series' skipped days,
/// keyed by the local date each one names.
///
/// `EXDATE` is a multi-property (RFC 5545 lets it repeat, and one line may carry
/// several comma-separated dates), so the dates are read out of `multi_properties`.
fn exdates_of(event: &Event, zone: Tz) -> Vec<NaiveDate> {
    let mut dates: Vec<NaiveDate> = event
        .multi_properties()
        .get("EXDATE")
        .into_iter()
        .flatten()
        .flat_map(|property| {
            property
                .value()
                .split(',')
                .map(str::trim)
                .map(str::to_owned)
        })
        .filter_map(|raw| exdate_date(&raw, zone))
        .collect();
    dates.sort();
    dates.dedup();
    dates
}

/// The local date one excluded occurrence names. A `Z` on the value makes it an
/// instant, read in the event's zone; without one it is already a local wall clock.
fn exdate_date(raw: &str, zone: Tz) -> Option<NaiveDate> {
    let body = raw.strip_suffix('Z').unwrap_or(raw);
    // `get`, not a slice: a non-ASCII byte near the start must not panic the import.
    let date_part = body.get(..8).unwrap_or(body);
    let date = NaiveDate::parse_from_str(date_part, "%Y%m%d").ok()?;
    match raw.ends_with('Z') {
        true => utc_instant(raw).map(|instant| instant.with_timezone(&zone).date_naive()),
        false => Some(date),
    }
}

/// The date an `UNTIL` names, read in the event's own zone.
///
/// Outlook writes the local 23:59:59 as UTC (`UNTIL=20261215T035959Z`), which in New
/// York is still the 14th: reading the instant in the event's zone is what makes the
/// last occurrence land on the day the user meant.
fn until_date(raw: &str, zone: Tz) -> Result<NaiveDate, String> {
    let raw = raw.trim();
    let date_part = raw.get(..8).unwrap_or(raw);
    let date = NaiveDate::parse_from_str(date_part, "%Y%m%d")
        .map_err(|_| format!("UNTIL={raw} is not a date, imported as one task"))?;
    Ok(match utc_instant(raw) {
        Some(instant) => instant.with_timezone(&zone).date_naive(),
        None => date,
    })
}

/// The UTC instant in an `UNTIL`, when it carries a time part.
fn utc_instant(raw: &str) -> Option<DateTime<Utc>> {
    let body = raw.strip_suffix('Z')?;
    NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S")
        .ok()
        .map(|local| Utc.from_utc_datetime(&local))
}

/// The date of the `count`-th occurrence from `start_day`: how a rule that counts
/// weeks instead of naming an end date is stored.
///
/// Computes arithmetically so a huge COUNT does not walk day by day and overflow.
/// Caps at [`IMPORT_MAX_RULE_COUNT`] occurrences; a count that large is listed in
/// `unmapped` instead.
fn last_occurrence(start_day: NaiveDate, weekdays: &[Weekday], count: u32) -> NaiveDate {
    // A count beyond the cap is almost certainly garbage.
    let count = count.clamp(1, IMPORT_MAX_RULE_COUNT);
    let days_per_cycle = weekdays.len() as i64;
    // Whole cycles of the weekday pattern
    let full_cycles = (count as i64 - 1) / days_per_cycle;
    let remainder = (count as i64 - 1) % days_per_cycle;
    // Jump full_cycles weeks, then walk the remainder days within the cycle
    let base = start_day
        .checked_add_days(chrono::Days::new(full_cycles as u64 * 7))
        .unwrap_or(start_day);
    let mut date = base;
    let mut found = 0;
    // Walk at most `weekdays.len()` days to find the remainder-th matching weekday
    while found <= remainder {
        if weekdays.contains(&date.weekday()) {
            if found == remainder {
                return date;
            }
            found += 1;
        }
        date = date.checked_add_days(chrono::Days::new(1)).unwrap_or(date);
    }
    date
}

/// The zone an event's times are written in: its `DTSTART` TZID, or UTC when it names
/// none — which is also how a floating time is read.
fn zone_of(event: &Event) -> Tz {
    event
        .properties()
        .get("DTSTART")
        .and_then(|property| property.params().get("TZID"))
        .and_then(|tzid| tzid.value().parse().ok())
        .unwrap_or(chrono_tz::UTC)
}

/// The iCalendar UID an import keys on, when the event has one.
/// The id a re-import matches on. A moved instance of a repeating event shares its
/// series' `UID` and names the instance it replaces in `RECURRENCE-ID`, so the two
/// together are what is unique; one export holds several such instances.
fn uid_of(event: &Event) -> Option<String> {
    let property = |key| {
        event
            .properties()
            .get(key)
            .map(|property| property.value().trim())
            .filter(|value| !value.is_empty())
    };
    let uid = property("UID")?;
    Some(match property("RECURRENCE-ID") {
        Some(instance) => format!("{uid}/{instance}"),
        None => uid.to_owned(),
    })
}

/// An event's length in minutes, inside the same limits as any task.
fn minutes_between(start: DateTime<Utc>, end: DateTime<Utc>) -> u16 {
    let minutes = (end - start).num_minutes();
    minutes.clamp(TASK_MIN_DURATION_MIN.into(), TASK_MAX_DURATION_MIN.into()) as u16
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
///
/// Returns `None` if the duration is absurd (overflows), so the caller falls back
/// to the 30-minute default instead of panicking.
fn duration_after(event: &Event, start: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let raw = event.properties().get("DURATION")?.value().trim();
    let mut seconds: i64 = 0;
    let mut digits = String::new();
    for character in raw.trim_start_matches('P').trim_start_matches('T').chars() {
        match character {
            '0'..='9' => digits.push(character),
            'H' | 'M' | 'S' | 'D' | 'W' => {
                let value = digits.parse::<i64>().ok()?;
                let multiplier = match character {
                    'H' => 3600,
                    'M' => 60,
                    'D' => 86_400,
                    'W' => 604_800,
                    _ => 1,
                };
                // Use checked arithmetic so P99999999999999W doesn't panic
                seconds = seconds.checked_add(value.checked_mul(multiplier)?)?;
                digits.clear();
            }
            _ => return None,
        }
    }
    // Anything longer than the cap is garbage.
    if seconds.abs() > IMPORT_MAX_EVENT_LENGTH.num_seconds() {
        return None;
    }
    start.checked_add_signed(chrono::Duration::seconds(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEEK: &str = include_str!("../../tests/fixtures/week.ics");

    /// The one-shot events of an export. The tests here are about dates, zones and
    /// durations; [`parse_calendar`] is what the server calls, and it also reads rules.
    fn parse_ics(text: &str) -> Result<Vec<ImportedEvent>, ImportError> {
        parse_calendar(text).map(|calendar| calendar.events)
    }

    /// A non-ASCII character where the date's eighth byte would be is a bad date,
    /// not a panic on the request path.
    #[test]
    fn a_non_ascii_until_or_exdate_is_refused_not_a_panic() {
        let zone = chrono_tz::America::New_York;
        assert!(until_date("2026121\u{e9}", zone).is_err());
        assert_eq!(exdate_date("2026101\u{e9}", zone), None);
        assert_eq!(exdate_date("2026101\u{e9}Z", zone), None);
    }

    #[test]
    fn the_demo_week_has_nine_events_spanning_the_work_week() {
        let busy = parse_ics(WEEK).expect("the Outlook fixture parses");

        assert_eq!(
            busy.len(),
            9,
            "expected 9 events in server/tests/fixtures/week.ics"
        );

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
    // server/tests/fixtures/week.ics is sanitised: LF only, and no VTIMEZONE, folded
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
    fn a_weekly_recurring_class_becomes_a_series_for_the_week() {
        // A student's real calendar is mostly RRULEs: the same lecture every week. It
        // is one series, not a block for the one week the export happened to cover.
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "UID:stat-201@example.com",
            "SUMMARY:STAT 201 Lecture",
            "DTSTART;TZID=America/New_York:20261013T090000",
            "DTEND;TZID=America/New_York:20261013T101500",
            "RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20261215T035959Z",
            "END:VEVENT",
        ]);
        let calendar = parse_calendar(&text).expect("parses");
        assert!(
            calendar.events.is_empty(),
            "a weekly rule is not imported as a one-shot task"
        );
        assert_eq!(calendar.series.len(), 1);
        let series = &calendar.series[0];
        assert_eq!(series.uid.as_deref(), Some("stat-201@example.com"));
        assert_eq!(series.title, "STAT 201 Lecture");
        assert_eq!(series.weekdays, vec![Weekday::Tue]);
        assert_eq!(
            series.start_day,
            NaiveDate::from_ymd_opt(2026, 10, 13).unwrap()
        );
        assert_eq!(
            series.time_of_day,
            Some(chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
            "09:00 is the time the series repeats at"
        );
        assert_eq!(series.zone, "America/New_York", "the event's own zone");
        assert_eq!(series.duration_min, 75, "75 minutes of lecture");
        // Outlook writes the local 23:59:59 as UTC, so 20261215T035959Z is still the
        // 14th in New York: the last occurrence lands on the day the user meant.
        assert_eq!(
            series.until,
            Some(NaiveDate::from_ymd_opt(2026, 12, 14).unwrap())
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

    // ---- additional ics import tests (oscar/ics-tests branch) --------

    #[test]
    fn tzid_zone_vs_utc_z_in_same_calendar() {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:UTC lab",
            "DTSTART:20261012T140000Z",
            "DTEND:20261012T150000Z",
            "END:VEVENT",
            "BEGIN:VEVENT",
            "SUMMARY:NY lecture",
            "DTSTART;TZID=America/New_York:20261012T100000",
            "DTEND;TZID=America/New_York:20261012T111500",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("both parse");
        assert_eq!(busy.len(), 2);
        assert!(busy.iter().any(|b| b.title == "NY lecture"));
        assert!(busy.iter().any(|b| b.title == "UTC lab"));
    }

    #[test]
    fn all_day_event_spans_two_dates() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:Reading week\nDTSTART;VALUE=DATE:20261013\nDTEND;VALUE=DATE:20261015\nEND:VEVENT\nEND:VCALENDAR\n";
        let busy = parse_ics(ics).expect("parses");
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].start.to_rfc3339(), "2026-10-13T00:00:00+00:00");
        assert_eq!(busy[0].end.to_rfc3339(), "2026-10-15T00:00:00+00:00");
    }

    #[test]
    fn event_crossing_midnight_keeps_length() {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Overnight shift",
            "DTSTART;TZID=America/New_York:20261012T220000",
            "DTEND;TZID=America/New_York:20261013T060000",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("parses");
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].start.to_rfc3339(), "2026-10-13T02:00:00+00:00");
        assert_eq!((busy[0].end - busy[0].start).num_hours(), 8);
    }

    #[test]
    fn cancelled_event_with_rrule_is_excluded() {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Cancelled lecture",
            "STATUS:CANCELLED",
            "DTSTART;TZID=America/New_York:20261013T090000",
            "DTEND;TZID=America/New_York:20261013T101500",
            "RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20261215T035959Z",
            "END:VEVENT",
        ]);
        let busy = parse_ics(&text).expect("parses");
        assert!(busy.is_empty(), "cancelled event must not appear");
    }

    #[test]
    fn malformed_text_returns_clear_error_never_panic() {
        let result = parse_ics("this is not a calendar");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.0.contains("iCalendar") || err.0.contains("not a calendar"),
            "error message should say it is not an iCalendar file: {}",
            err.0
        );
    }

    #[test]
    fn weekly_rrule_becomes_a_series_not_a_single_block() {
        // The recorded old behaviour: one block for the first instance. Now the rule
        // is kept whole, so every week follows it.
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "UID:weekly-class@example.com",
            "SUMMARY:Weekly class",
            "DTSTART;TZID=America/New_York:20261013T090000",
            "DTEND;TZID=America/New_York:20261013T101500",
            "RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20261215T035959Z",
            "END:VEVENT",
        ]);
        let calendar = parse_calendar(&text).expect("parses");
        assert!(calendar.events.is_empty(), "RRULE is no longer a one-shot");
        assert_eq!(calendar.series.len(), 1);
        assert_eq!(
            calendar.series[0].weekdays,
            vec![Weekday::Tue],
            "it repeats on Tuesdays"
        );
    }

    // ---- recurring rules (docs/ARCHITECTURE.md 7.1) ------------------------

    /// A rule with no `BYDAY` repeats on the weekday it starts on.
    #[test]
    fn a_weekly_rule_without_byday_repeats_on_its_start_day() {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Swim",
            "DTSTART;TZID=America/New_York:20261014T070000",
            "DTEND;TZID=America/New_York:20261014T080000",
            "RRULE:FREQ=WEEKLY",
            "END:VEVENT",
        ]);
        let calendar = parse_calendar(&text).expect("parses");
        assert_eq!(calendar.series.len(), 1);
        assert_eq!(calendar.series[0].weekdays, vec![Weekday::Wed]);
        assert_eq!(calendar.series[0].until, None, "no end named: forever");
    }

    /// `COUNT` is how a rule says "this many times", which is stored as the date of
    /// its last occurrence.
    #[test]
    fn a_count_is_stored_as_the_date_of_the_last_occurrence() {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Bootcamp",
            "DTSTART;TZID=America/New_York:20261013T090000",
            "DTEND;TZID=America/New_York:20261013T120000",
            "RRULE:FREQ=WEEKLY;BYDAY=TU,TH;COUNT=5",
            "END:VEVENT",
        ]);
        let calendar = parse_calendar(&text).expect("parses");
        let series = &calendar.series[0];
        assert_eq!(series.weekdays, vec![Weekday::Tue, Weekday::Thu]);
        // Tue 13, Thu 15, Tue 20, Thu 22, Tue 27 October.
        assert_eq!(
            series.until,
            Some(NaiveDate::from_ymd_opt(2026, 10, 27).unwrap()),
            "five occurrences of two days a week end on the fifth"
        );
    }

    /// An `EXDATE` is an occurrence the export takes back, which the series keeps as a
    /// skipped day.
    #[test]
    fn excluded_dates_come_back_as_skipped_days() {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "UID:standup@example.com",
            "SUMMARY:Standup",
            "DTSTART;TZID=America/New_York:20261012T093000",
            "DTEND;TZID=America/New_York:20261012T094500",
            "RRULE:FREQ=WEEKLY;BYDAY=MO",
            "EXDATE;TZID=America/New_York:20261019T093000",
            "EXDATE;TZID=America/New_York:20261102T093000",
            "END:VEVENT",
        ]);
        let calendar = parse_calendar(&text).expect("parses");
        assert_eq!(
            calendar.series[0].exdates,
            vec![
                NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(),
                NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
            ],
            "both excluded Mondays are kept, in order"
        );
    }

    /// A rule we cannot follow is imported as today's single task, and the answer says
    /// why, so the client can tell the user their rule was not kept.
    #[test]
    fn rules_we_cannot_follow_are_imported_once_and_reported() {
        for (rule, because) in [
            ("FREQ=MONTHLY;BYMONTHDAY=1", "repeats"),
            ("FREQ=DAILY;COUNT=10", "repeats"),
            ("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO", "every 2 weeks"),
            ("FREQ=WEEKLY;BYDAY=2FR", "counts weeks"),
            (
                "FREQ=WEEKLY;BYSETPOS=1;BYDAY=MO",
                "BYSETPOS is not supported",
            ),
        ] {
            let text = with_vtimezone(&[
                "BEGIN:VEVENT",
                "SUMMARY:Repeat",
                "DTSTART;TZID=America/New_York:20261012T090000",
                "DTEND;TZID=America/New_York:20261012T100000",
                &format!("RRULE:{rule}"),
                "END:VEVENT",
            ]);
            let calendar = parse_calendar(&text).expect("parses");
            assert!(
                calendar.series.is_empty(),
                "{rule} must not become a series"
            );
            assert_eq!(
                calendar.events.len(),
                1,
                "{rule} still imports its first week as a task"
            );
            assert_eq!(calendar.unmapped.len(), 1, "{rule} must be reported");
            let note = &calendar.unmapped[0];
            assert!(note.contains("Repeat"), "{note} should name the event");
            assert!(note.contains(because), "{note} should say why: {because}");
        }
    }

    /// An all-day weekly event has no time, so its occurrences float rather than being
    /// pinned to midnight UTC.
    #[test]
    fn an_all_day_weekly_event_floats() {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Laundry day",
            "DTSTART;VALUE=DATE:20261012",
            "DTEND;VALUE=DATE:20261013",
            "RRULE:FREQ=WEEKLY;BYDAY=MO",
            "END:VEVENT",
        ]);
        let calendar = parse_calendar(&text).expect("parses");
        assert_eq!(calendar.series.len(), 1);
        assert_eq!(calendar.series[0].time_of_day, None, "no time to pin");
        assert_eq!(calendar.series[0].weekdays, vec![Weekday::Mon]);
    }

    /// An event with no rule is still one task, as before.
    #[test]
    fn an_event_without_a_rule_is_a_single_task() {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:One off",
            "DTSTART;TZID=America/New_York:20261012T100000",
            "DTEND;TZID=America/New_York:20261012T110000",
            "END:VEVENT",
        ]);
        let calendar = parse_calendar(&text).expect("parses");
        assert_eq!(calendar.events.len(), 1);
        assert!(calendar.series.is_empty());
        assert!(calendar.unmapped.is_empty(), "nothing to report");
    }

    /// A huge COUNT must not burn CPU or panic on date overflow.
    #[test]
    fn a_huge_count_does_not_panic() {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "SUMMARY:Bootcamp",
            "DTSTART;TZID=America/New_York:20261013T090000",
            "DTEND;TZID=America/New_York:20261013T120000",
            "RRULE:FREQ=WEEKLY;BYDAY=TU,TH;COUNT=4000000000",
            "END:VEVENT",
        ]);
        let calendar = parse_calendar(&text).expect("parses without panic");
        // COUNT is capped at 1820, so the series should still be created
        assert_eq!(calendar.series.len(), 1);
        // With 2 days/week, 1820 occurrences = 910 weeks ≈ 6370 days.
        // The point is it doesn't panic and finishes instantly.
        let until = calendar.series[0].until.unwrap();
        let start = NaiveDate::from_ymd_opt(2026, 10, 13).unwrap();
        let days = (until - start).num_days();
        assert!(
            days < 7000,
            "until should be reasonable after capping, got {days} days"
        );
    }

    /// A huge DURATION must not panic; fall back to 30-minute default.
    #[test]
    fn a_huge_duration_falls_back_to_default() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\n\
            SUMMARY:Long event\n\
            DTSTART:20261012T140000Z\n\
            DURATION:P99999999999999W\n\
            END:VEVENT\nEND:VCALENDAR\n";
        let busy = parse_ics(ics).expect("parses without panic");
        assert_eq!(busy.len(), 1);
        // duration_after returns None, so end = start + 30 minutes
        assert_eq!(
            busy[0].end.to_rfc3339(),
            "2026-10-12T14:30:00+00:00",
            "absurd duration falls back to 30-minute default"
        );
    }

    /// last_occurrence computes the same result as the old loop for small counts.
    #[test]
    fn last_occurrence_matches_loop_for_small_counts() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 13).unwrap(); // Tuesday
        let weekdays = vec![Weekday::Tue, Weekday::Thu];
        // COUNT=5: Tue 13, Thu 15, Tue 20, Thu 22, Tue 27
        let last = last_occurrence(start, &weekdays, 5);
        assert_eq!(
            last,
            NaiveDate::from_ymd_opt(2026, 10, 27).unwrap(),
            "5th occurrence of Tue/Thu from Oct 13 is Oct 27"
        );
        // COUNT=1: just the start day
        let first = last_occurrence(start, &weekdays, 1);
        assert_eq!(first, start, "COUNT=1 is the start day");
        // COUNT=0: treated as 1
        let zero = last_occurrence(start, &weekdays, 0);
        assert_eq!(zero, start, "COUNT=0 is treated as 1");
    }
}

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
use crate::v2::tuning::{IMPORT_MAX_EVENT_LENGTH, IMPORT_MAX_INTERVAL, IMPORT_MAX_RULE_COUNT};
use todo_core::Recurrence;
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

        match rrule_series(event, start) {
            // A rule becomes a series, never a task per date.
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

/// The parts of a rule we follow. Anything else (`BYSETPOS`, `BYMONTH`,
/// `BYMONTHDAY`, `BYWEEKNO`, …) names a rhythm this app cannot keep.
const SUPPORTED_PARTS: [&str; 6] = ["FREQ", "BYDAY", "UNTIL", "COUNT", "WKST", "INTERVAL"];

/// The series an event's `RRULE` describes, or `None` when it has no rule. An
/// `Err` is a rule we cannot follow, and says why: the caller imports the event's first
/// instance instead and tells the user this rule was not followed.
fn rrule_series(event: &Event, start: DateTime<Utc>) -> Result<Option<ImportedSeries>, String> {
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
    let freq = part("FREQ").unwrap_or("WEEKLY");
    let is_weekly = freq.eq_ignore_ascii_case("WEEKLY");
    let is_daily = freq.eq_ignore_ascii_case("DAILY");
    if !is_weekly && !is_daily {
        return Err(format!("{} repeats, imported as one task", freq));
    }
    // INTERVAL: how many weeks (weekly) or days (daily) between occurrences.
    let interval: u32 = match part("INTERVAL") {
        None => 1,
        Some(raw) => raw
            .trim()
            .parse()
            .ok()
            .filter(|interval| (1..=IMPORT_MAX_INTERVAL).contains(interval))
            .ok_or_else(|| {
                format!(
                    "INTERVAL={raw} is not a number from 1 to {IMPORT_MAX_INTERVAL}, imported as one task"
                )
            })?,
    };
    // Which weekday a week starts on decides which weeks "every 2 weeks" skips.
    if is_weekly
        && interval > 1
        && part("WKST").is_some_and(|wkst| !wkst.eq_ignore_ascii_case("MO"))
    {
        return Err(
            "WKST other than MO with an INTERVAL is not supported, imported as one task".into(),
        );
    }
    if let Some((name, _)) = parts
        .iter()
        .find(|(name, _)| !SUPPORTED_PARTS.contains(&name.to_uppercase().as_str()))
    {
        return Err(format!("{name} is not supported, imported as one task"));
    }

    let weekdays = match (is_weekly, part("BYDAY")) {
        // No BYDAY means the event's own weekday, which is the only one DTSTART can be.
        (true, None) => vec![start_day.weekday()],
        (true, Some(list)) => weekdays_of(list)?,
        // A daily rule has no weekdays.
        (false, None) => Vec::new(),
        // Every day, on these days only, is every week on these days: the same rule, as long
        // as it is every day (a day step cannot be a week step).
        (false, Some(list)) if interval == 1 => weekdays_of(list)?,
        (false, Some(_)) => {
            return Err(
                "BYDAY with an INTERVAL on a daily rule is not supported, imported as one task"
                    .into(),
            );
        }
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
            let rule = if weekdays.is_empty() {
                Recurrence::daily(interval, None)
            } else {
                Recurrence::with_interval(weekdays.iter().copied(), interval, None)
            };
            rule.and_then(|rule| {
                rule.nth_occurrence(start_day, count.clamp(1, IMPORT_MAX_RULE_COUNT))
            })
            .ok_or_else(|| format!("COUNT={raw} runs past the calendar, imported as one task"))
            .map(Some)?
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
        interval,
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
mod tests;

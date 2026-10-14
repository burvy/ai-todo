//! What the edit dialog's fields mean: dates, times, repeats.

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Timelike, Utc, Weekday};
use todo_core::When;

use crate::api;

/// The dialog's day buttons, Monday first, each weekday with its short name.
pub(crate) const WEEKDAY_CHOICES: [(Weekday, &str); 7] = [
    (Weekday::Mon, "Mon"),
    (Weekday::Tue, "Tue"),
    (Weekday::Wed, "Wed"),
    (Weekday::Thu, "Thu"),
    (Weekday::Fri, "Fri"),
    (Weekday::Sat, "Sat"),
    (Weekday::Sun, "Sun"),
];

/// The days a series repeats on, Monday-first without duplicates — what the
/// wire wants, whatever order the buttons were clicked in.
fn sorted_weekdays(days: &[Weekday]) -> Vec<Weekday> {
    let mut days: Vec<Weekday> = days.to_vec();
    days.sort_by_key(|day| day.num_days_from_monday());
    days.dedup();
    days
}

/// Picking a picked day unpicks it.
pub(crate) fn toggled_day(days: Vec<Weekday>, day: Weekday) -> Vec<Weekday> {
    let mut days = days;
    if days.contains(&day) {
        days.retain(|picked| *picked != day);
    } else {
        days.push(day);
    }
    days
}

/// `HH:MM` from the dialog's time field, or none when left empty; an
/// unparsable value is treated as empty, and the field never offers anything
/// else its type cannot produce.
pub(crate) fn parse_hhmm(value: &str) -> Option<String> {
    chrono::NaiveTime::parse_from_str(value, "%H:%M")
        .ok()
        .map(|time| time.format("%H:%M").to_string())
}

/// `YYYY-MM-DD` from the dialog's end-date field, or none when left empty.
fn parse_ymd(value: &str) -> Option<String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .map(|date| date.to_string())
}

/// The series the dialog's fields describe, or `None` when they name no rule
/// yet (no title, no length, no day picked): the caller leaves the dialog
/// open, the way it already does for an ordinary edit that is not filled in.
pub(crate) fn series_from_dialog(
    title: &str,
    duration: Option<u16>,
    days: &[Weekday],
    until: &str,
    time: &str,
) -> Option<api::SeriesBody> {
    let title = title.trim();
    if title.is_empty() || duration.is_none() || days.is_empty() {
        return None;
    }
    Some(api::SeriesBody {
        title: title.to_owned(),
        duration_min: duration.unwrap_or(1),
        weekdays: sorted_weekdays(days),
        time_of_day: parse_hhmm(time),
        until: parse_ymd(until),
    })
}

/// The series edit the dialog's fields describe. The dialog opens holding the
/// series' rule, so an emptied end date or time is sent as `null` and clears it.
pub(crate) fn series_patch_from_dialog(
    title: &str,
    duration: Option<u16>,
    days: &[Weekday],
    until: &str,
    time: &str,
) -> Option<api::SeriesPatchBody> {
    let title = title.trim();
    if title.is_empty() || duration.is_none() || days.is_empty() {
        return None;
    }
    Some(api::SeriesPatchBody {
        title: Some(title.to_owned()),
        duration_min: duration,
        weekdays: Some(sorted_weekdays(days)),
        time_of_day: parse_hhmm(time),
        until: parse_ymd(until),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditWhen {
    Anytime,
    DueBy,
    At,
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn local_input_value(time: DateTime<Utc>) -> String {
    let date = js_sys::Date::new(&(time.timestamp_millis() as f64).into());
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes()
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn local_input_value(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%dT%H:%M").to_string()
}

#[cfg(target_arch = "wasm32")]
fn parse_local_input(value: &str) -> Option<DateTime<Utc>> {
    // An empty or bad field parses to NaN, and `NaN as i64` is 0: 1 January 1970.
    let millis = js_sys::Date::parse(&format!("{value}:00"));
    if millis.is_nan() {
        return None;
    }
    // The browser quietly moves a wall clock that falls in a spring-forward gap
    // (02:30 becomes 03:30); read the instant back and refuse it if it moved.
    DateTime::from_timestamp_millis(millis as i64)
        .filter(|instant| wall_clock_kept(value, &local_input_value(*instant)))
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_local_input(value: &str) -> Option<DateTime<Utc>> {
    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
        .ok()
        .map(|time| DateTime::<Utc>::from_naive_utc_and_offset(time, Utc))
}

/// Whether the instant a browser built from `asked` still reads as `asked` on the wall:
/// false when the browser moved it out of a clock-change gap.
#[cfg(any(target_arch = "wasm32", test))]
fn wall_clock_kept(asked: &str, read_back: &str) -> bool {
    asked == read_back
}

/// A deadline's clock time as it reads in its own zone, for the edit dialog. Unlike
/// `local_input_value` this does not depend on the browser's zone, which is exactly
/// the point of `due_tz` (docs/ARCHITECTURE.md 4.1): a deadline set as Friday 17:00
/// New York reads Friday 17:00 no matter where the browser is.
fn due_input_value(time: DateTime<Utc>, tz: chrono_tz::Tz) -> String {
    let local = time.with_timezone(&tz);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        local.year(),
        local.month(),
        local.day(),
        local.hour(),
        local.minute()
    )
}

/// How the edit dialog shows a deadline: in the zone it was set in when the task
/// has one, else in the browser's zone (tasks created before `due_tz` existed).
pub(crate) fn due_display(task: &todo_core::Task, time: DateTime<Utc>) -> String {
    match task.due_tz.as_deref().and_then(|name| name.parse().ok()) {
        Some(tz) => due_input_value(time, tz),
        None => local_input_value(time),
    }
}

/// Parse a deadline typed into the edit dialog as a time in `tz`, so a deadline
/// shown in its original zone saves as the same local time even after a move.
/// Fall-back makes a wall clock ambiguous (the hour repeats); `.earliest()` picks
/// the first of the two, where `.single()` used to give up silently. A time in a
/// spring-forward gap genuinely does not exist and still fails.
fn parse_due_input(value: &str, tz: chrono_tz::Tz) -> Option<DateTime<Utc>> {
    let naive = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").ok()?;
    tz.from_local_datetime(&naive)
        .earliest()
        .map(|local| local.with_timezone(&Utc))
}

/// The deadline the edited task already has, parsed in the zone that set it (so the
/// shown value round-trips), or in the browser's zone for a task without `due_tz`.
fn parse_edited_due(value: &str, task: &todo_core::Task) -> Option<When> {
    let due = match task.due_tz.as_deref().and_then(|name| name.parse().ok()) {
        Some(tz) => parse_due_input(value, tz),
        None => parse_local_input(value),
    };
    due.map(When::DueBy)
}

/// The `when` a save sends, or the message to show when the typed clock time names no
/// instant. Three error messages distinguish the cause: empty field, unrecognised
/// format, or a wall-clock time inside a spring-forward gap (02:30 when the clocks jump
/// 02:00 to 03:00). The dialog shows the message and stays open so a Save that does
/// nothing quietly does not look broken. `deadline` is the edited task, which carries
/// the zone its due time is read in.
pub(crate) fn typed_when(
    kind: EditWhen,
    value: &str,
    deadline: Option<&todo_core::Task>,
) -> Result<When, &'static str> {
    if let EditWhen::Anytime = kind {
        return Ok(When::Anytime);
    }
    if value.trim().is_empty() {
        return Err("Pick a time");
    }
    // Check the format before resolving: `NaiveDateTime::parse_from_str` fails on
    // bad format, while the time-zone resolver fails on spring-forward gaps.
    if chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").is_err() {
        return Err("That is not a time");
    }
    let when = match kind {
        EditWhen::DueBy => deadline.and_then(|task| parse_edited_due(value, task)),
        EditWhen::At => parse_local_input(value).map(When::At),
        _ => None,
    };
    when.ok_or("That time does not exist on this day (clocks change)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_datetime_input() {
        assert_eq!(
            parse_local_input("2026-10-12T14:30")
                .expect("valid datetime")
                .format("%Y-%m-%dT%H:%M")
                .to_string(),
            "2026-10-12T14:30"
        );
    }
    /// docs/ARCHITECTURE.md 4.1: a deadline is edited in the zone it was set in.
    /// The value shown and the value parsed on save must round-trip in that zone,
    /// on both sides of the November DST change.
    #[test]
    fn a_due_deadline_round_trips_in_its_own_zone_across_dst() {
        let ny = chrono_tz::America::New_York;
        // Friday 2026-10-30 23:59 New York is still EDT (UTC-4).
        let before = Utc.with_ymd_and_hms(2026, 10, 31, 3, 59, 0).unwrap();
        assert_eq!(due_input_value(before, ny), "2026-10-30T23:59");
        assert_eq!(parse_due_input("2026-10-30T23:59", ny), Some(before));

        // Monday 2026-11-02 23:59 New York is EST (UTC-5): a wall clock set after
        // the fall-back is 04:59:00Z, not 03:59:00Z.
        let after = Utc.with_ymd_and_hms(2026, 11, 3, 4, 59, 0).unwrap();
        assert_eq!(due_input_value(after, ny), "2026-11-02T23:59");
        assert_eq!(parse_due_input("2026-11-02T23:59", ny), Some(after));

        // The fall-back itself: 01:30 New York on 2026-11-01 happens twice — 01:30
        // EDT and again 01:30 EST after the clocks fall back. `.earliest()` picks
        // the first one; `.single()` used to reject the whole time silently.
        let fold_earliest = Utc.with_ymd_and_hms(2026, 11, 1, 5, 30, 0).unwrap();
        assert_eq!(parse_due_input("2026-11-01T01:30", ny), Some(fold_earliest));

        // A spring-forward gap is not a time at all: 02:30 on 2026-03-08 New York
        // (the clocks jump 02:00 to 03:00) must not invent an instant.
        assert_eq!(parse_due_input("2026-03-08T02:30", ny), None);
    }

    /// A deadline set as 17:00 New York reads 17:00 New York even when the browser
    /// lives somewhere else — that is the whole point of `due_tz`.
    #[test]
    fn a_due_task_is_shown_in_the_zone_it_was_set_in() {
        let deadline = Utc.with_ymd_and_hms(2026, 10, 16, 21, 0, 0).unwrap();
        let mut task = api::fixture_snapshot().tasks[0].clone();
        task.when = When::DueBy(deadline);
        task.due_tz = Some("America/New_York".into());

        assert_eq!(due_display(&task, deadline), "2026-10-16T17:00");
        // Saving without touching the field keeps the same instant: 17:00 in New
        // York is not 17:00 in Los Angeles.
        assert_eq!(
            parse_edited_due("2026-10-16T17:00", &task),
            Some(When::DueBy(deadline))
        );
        // ... and the same deadline reads 14:00 only when shown in Los Angeles.
        assert_eq!(
            due_input_value(deadline, chrono_tz::America::Los_Angeles),
            "2026-10-16T14:00"
        );
    }

    /// Three different error messages for three different problems: empty field,
    /// unrecognised format, and a time that never happens (spring-forward gap).
    #[test]
    fn typed_when_distinguishes_empty_bad_format_and_clock_skip() {
        let mut task = api::fixture_snapshot().tasks[0].clone();
        task.when = When::DueBy(Utc.with_ymd_and_hms(2027, 3, 13, 7, 0, 0).unwrap());
        task.due_tz = Some("America/New_York".into());

        // Empty field asks the user to pick.
        assert_eq!(
            typed_when(EditWhen::DueBy, "", Some(&task)),
            Err("Pick a time")
        );
        // Whitespace-only is treated as empty.
        assert_eq!(
            typed_when(EditWhen::DueBy, "   ", Some(&task)),
            Err("Pick a time")
        );
        // Text that is not a datetime format gets a format message.
        assert_eq!(
            typed_when(EditWhen::DueBy, "tomorrow", Some(&task)),
            Err("That is not a time")
        );
        assert_eq!(
            typed_when(EditWhen::DueBy, "14:30", Some(&task)),
            Err("That is not a time")
        );
        // 02:30 on 2027-03-14 New York: clocks jump 02:00 to 03:00 that day.
        assert_eq!(
            typed_when(EditWhen::DueBy, "2027-03-14T02:30", Some(&task)),
            Err("That time does not exist on this day (clocks change)")
        );
        // An ordinary time saves correctly.
        assert_eq!(
            typed_when(EditWhen::DueBy, "2027-03-14T09:30", Some(&task)),
            Ok(When::DueBy(
                Utc.with_ymd_and_hms(2027, 3, 14, 13, 30, 0).unwrap()
            ))
        );
        // "Anytime" ignores the field entirely.
        assert_eq!(typed_when(EditWhen::Anytime, "", None), Ok(When::Anytime));
    }

    /// The browser turns 02:30 in a spring-forward gap into 03:30; the save path must
    /// notice the wall clock moved, so the sheet shows the clocks-change message.
    #[test]
    fn a_gap_time_the_browser_moved_is_not_the_time_asked_for() {
        assert!(!wall_clock_kept("2027-03-14T02:30", "2027-03-14T03:30"));
        assert!(wall_clock_kept("2027-03-14T09:30", "2027-03-14T09:30"));
    }

    /// The paper pickers write a lone day or a lone time when only one is picked;
    /// the save path still answers with the BUG-7 messages.
    #[test]
    fn typed_when_reads_what_the_paper_pickers_write() {
        use crate::pickers::join_datetime;
        assert_eq!(
            typed_when(EditWhen::At, &join_datetime("", ""), None),
            Err("Pick a time")
        );
        assert_eq!(
            typed_when(EditWhen::At, &join_datetime("2026-10-14", ""), None),
            Err("That is not a time")
        );
        assert_eq!(
            typed_when(EditWhen::At, &join_datetime("", "09:30"), None),
            Err("That is not a time")
        );
        assert!(typed_when(EditWhen::At, &join_datetime("2026-10-14", "09:30"), None).is_ok());
    }

    /// The same three messages apply to the `At` kind (no deadline task needed).
    #[test]
    fn typed_when_at_kind_has_same_error_messages() {
        assert_eq!(typed_when(EditWhen::At, "", None), Err("Pick a time"));
        assert_eq!(
            typed_when(EditWhen::At, "noon", None),
            Err("That is not a time")
        );
        assert!(matches!(
            typed_when(EditWhen::At, "2027-06-01T10:00", None),
            Ok(When::At(_))
        ));
    }

    /// A task from before `due_tz` existed has no zone: the edit dialog falls back
    /// to the browser's own reading (UTC on the host test target), so old behaviour
    /// is untouched.
    #[test]
    fn a_task_without_due_tz_falls_back_to_the_browser_zone() {
        let deadline = Utc.with_ymd_and_hms(2026, 10, 16, 21, 0, 0).unwrap();
        let mut task = api::fixture_snapshot().tasks[0].clone();
        task.when = When::DueBy(deadline);
        task.due_tz = None;
        assert_eq!(due_display(&task, deadline), "2026-10-16T21:00");
        assert_eq!(
            parse_edited_due("2026-10-16T21:00", &task),
            Some(When::DueBy(deadline))
        );
    }

    #[test]
    fn the_day_buttons_sort_monday_first_and_toggle() {
        assert_eq!(
            sorted_weekdays(&[Weekday::Sun, Weekday::Mon, Weekday::Wed, Weekday::Mon]),
            vec![Weekday::Mon, Weekday::Wed, Weekday::Sun]
        );
        let picked = toggled_day(Vec::new(), Weekday::Thu);
        assert_eq!(
            toggled_day(picked.clone(), Weekday::Thu),
            Vec::<Weekday>::new(),
            "picking a picked day unpicks it"
        );
        assert_eq!(
            toggled_day(picked, Weekday::Fri),
            vec![Weekday::Thu, Weekday::Fri]
        );
    }

    #[test]
    fn the_dialog_builds_a_series_body_from_its_fields() {
        let body = series_from_dialog(
            "  Gym  ",
            Some(30),
            &[Weekday::Thu, Weekday::Mon],
            "2026-12-15",
            "07:00",
        )
        .expect("every field names a rule");
        assert_eq!(body.title, "Gym");
        assert_eq!(body.duration_min, 30);
        assert_eq!(body.weekdays, vec![Weekday::Mon, Weekday::Thu]);
        assert_eq!(body.until.as_deref(), Some("2026-12-15"));
        assert_eq!(body.time_of_day.as_deref(), Some("07:00"));
        // An empty end date or time is simply absent on the wire.
        let floating = series_from_dialog("Gym", Some(30), &[Weekday::Mon], "", "")
            .expect("no end and no time is a valid rule");
        assert_eq!(floating.until, None);
        assert_eq!(floating.time_of_day, None);
        // No title, no length, or no day picked: nothing to save.
        assert!(series_from_dialog("  ", Some(30), &[Weekday::Mon], "", "").is_none());
        assert!(series_from_dialog("Gym", None, &[Weekday::Mon], "", "").is_none());
        assert!(series_from_dialog("Gym", Some(30), &[], "", "").is_none());
    }

    #[test]
    fn a_series_edit_sends_only_the_fields_the_dialog_holds() {
        let patch = series_patch_from_dialog("Gym", Some(45), &[Weekday::Fri], "", "08:30")
            .expect("a usable edit");
        assert_eq!(patch.title.as_deref(), Some("Gym"));
        assert_eq!(patch.duration_min, Some(45));
        assert_eq!(patch.weekdays, Some(vec![Weekday::Fri]));
        assert_eq!(patch.until, None);
        assert_eq!(patch.time_of_day.as_deref(), Some("08:30"));
        // An emptied end date goes as null, which clears it (7.1); a missing key
        // would keep the old one.
        let wire = serde_json::to_value(&patch).unwrap();
        assert_eq!(wire["until"], serde_json::Value::Null);
        assert!(wire.as_object().unwrap().contains_key("until"));
        assert!(series_patch_from_dialog("Gym", Some(45), &[], "", "").is_none());
    }
}

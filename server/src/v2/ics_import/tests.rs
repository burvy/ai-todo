use super::*;

const WEEK: &str = include_str!("../../../tests/fixtures/week.ics");

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

/// FREQ=WEEKLY;INTERVAL=2 becomes a series with interval=2.
#[test]
fn weekly_with_interval_becomes_a_series() {
    let text = with_vtimezone(&[
        "BEGIN:VEVENT",
        "UID:biweekly@example.com",
        "SUMMARY:Biweekly sync",
        "DTSTART;TZID=America/New_York:20261012T090000",
        "DTEND;TZID=America/New_York:20261012T100000",
        "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO",
        "END:VEVENT",
    ]);
    let calendar = parse_calendar(&text).expect("parses");
    assert_eq!(
        calendar.series.len(),
        1,
        "INTERVAL=2 weekly becomes a series"
    );
    let series = &calendar.series[0];
    assert_eq!(series.weekdays, vec![Weekday::Mon]);
    assert_eq!(series.interval, 2);
    assert!(calendar.unmapped.is_empty(), "no unmapped for INTERVAL=2");
}

/// FREQ=DAILY becomes a daily series (empty weekdays).
#[test]
fn daily_becomes_a_series() {
    let text = with_vtimezone(&[
        "BEGIN:VEVENT",
        "UID:daily@example.com",
        "SUMMARY:Standup",
        "DTSTART;TZID=America/New_York:20261012T090000",
        "DTEND;TZID=America/New_York:20261012T091500",
        "RRULE:FREQ=DAILY;COUNT=10",
        "END:VEVENT",
    ]);
    let calendar = parse_calendar(&text).expect("parses");
    assert_eq!(calendar.series.len(), 1, "DAILY becomes a series");
    let series = &calendar.series[0];
    assert!(series.weekdays.is_empty(), "daily has no weekdays");
    assert_eq!(series.interval, 1);
    // COUNT=10: start Oct 12 + 9 days = Oct 21
    assert_eq!(
        series.until,
        Some(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap()),
        "10 daily occurrences end on the 10th day"
    );
    assert!(calendar.unmapped.is_empty(), "no unmapped for DAILY");
}

/// FREQ=DAILY;INTERVAL=2 becomes a series that fires every 2 days.
#[test]
fn daily_with_interval_becomes_a_series() {
    let text = with_vtimezone(&[
        "BEGIN:VEVENT",
        "UID:every2days@example.com",
        "SUMMARY:Water plants",
        "DTSTART;TZID=America/New_York:20261012T080000",
        "DTEND;TZID=America/New_York:20261012T081500",
        "RRULE:FREQ=DAILY;INTERVAL=2;COUNT=5",
        "END:VEVENT",
    ]);
    let calendar = parse_calendar(&text).expect("parses");
    assert_eq!(calendar.series.len(), 1);
    let series = &calendar.series[0];
    assert!(series.weekdays.is_empty());
    assert_eq!(series.interval, 2);
    // COUNT=5: Oct 12, 14, 16, 18, 20
    assert_eq!(
        series.until,
        Some(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()),
        "5 occurrences every 2 days end on Oct 20"
    );
}

/// FREQ=WEEKLY;INTERVAL=2;COUNT computes the last occurrence correctly.
#[test]
fn weekly_interval_with_count() {
    let text = with_vtimezone(&[
        "BEGIN:VEVENT",
        "UID:biweekly-count@example.com",
        "SUMMARY:Biweekly review",
        "DTSTART;TZID=America/New_York:20261012T140000",
        "DTEND;TZID=America/New_York:20261012T150000",
        "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO;COUNT=3",
        "END:VEVENT",
    ]);
    let calendar = parse_calendar(&text).expect("parses");
    assert_eq!(calendar.series.len(), 1);
    let series = &calendar.series[0];
    assert_eq!(series.weekdays, vec![Weekday::Mon]);
    assert_eq!(series.interval, 2);
    // COUNT=3: Oct 12, Oct 26, Nov 9
    assert_eq!(
        series.until,
        Some(NaiveDate::from_ymd_opt(2026, 11, 9).unwrap()),
        "3 occurrences every 2 weeks end on Nov 9"
    );
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

/// One event with the given RRULE, DTSTART Wed 14 Oct 2026 09:00 New York.
fn rule_on_a_wednesday(rule: &str) -> ParsedCalendar {
    let rule = format!("RRULE:{rule}");
    let text = with_vtimezone(&[
        "BEGIN:VEVENT",
        "UID:rule@example.com",
        "SUMMARY:Rule",
        "DTSTART;TZID=America/New_York:20261014T090000",
        "DTEND;TZID=America/New_York:20261014T100000",
        &rule,
        "END:VEVENT",
    ]);
    parse_calendar(&text).expect("parses")
}

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// COUNT with INTERVAL and several BYDAY days counts calendar weeks from the start's
/// own week: Wed 14, Mon 26, Wed 28, Mon 9 Nov is the 4th.
#[test]
fn count_with_interval_and_several_days_ends_on_the_right_day() {
    let calendar = rule_on_a_wednesday("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=4");
    assert_eq!(calendar.series.len(), 1);
    assert_eq!(calendar.series[0].until, Some(ymd(2026, 11, 9)));
    // ...and it agrees with the rule's own coverage.
    let rule = Recurrence::with_interval([Weekday::Mon, Weekday::Wed], 2, None).unwrap();
    let start = calendar.series[0].start_day;
    assert!(rule.covers(start, ymd(2026, 11, 9)));
    assert!(
        !rule.covers(start, ymd(2026, 11, 2)),
        "the week between is skipped"
    );
}

/// A weekday-only daily rule is a weekly rule on those days, never every day.
#[test]
fn daily_on_some_days_is_a_weekly_series_on_those_days() {
    let calendar = rule_on_a_wednesday("FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR;COUNT=7");
    assert!(calendar.unmapped.is_empty());
    let series = &calendar.series[0];
    assert_eq!(
        series.weekdays,
        vec![
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri
        ]
    );
    assert_eq!(series.interval, 1);
    // Wed 14, Thu 15, Fri 16, Mon 19, Tue 20, Wed 21, Thu 22: no weekend.
    assert_eq!(series.until, Some(ymd(2026, 10, 22)));
}

/// BYDAY on a daily rule with a day step has no weekly twin: imported once and reported.
#[test]
fn daily_on_some_days_with_a_step_is_reported() {
    let calendar = rule_on_a_wednesday("FREQ=DAILY;INTERVAL=2;BYDAY=MO,WE");
    assert!(calendar.series.is_empty());
    assert_eq!(calendar.unmapped.len(), 1);
    assert!(
        calendar.unmapped[0].contains("BYDAY with an INTERVAL"),
        "{:?}",
        calendar.unmapped
    );
}

/// An interval that is zero, not a number or beyond the cap falls back per event; the
/// rest of the import is untouched.
#[test]
fn a_bad_interval_falls_back_for_that_event_only() {
    for interval in ["0", "x", "-1", "1001", "4294967295"] {
        let text = with_vtimezone(&[
            "BEGIN:VEVENT",
            "UID:bad@example.com",
            "SUMMARY:Bad",
            "DTSTART;TZID=America/New_York:20261014T090000",
            "DTEND;TZID=America/New_York:20261014T100000",
            &format!("RRULE:FREQ=DAILY;INTERVAL={interval}"),
            "END:VEVENT",
            "BEGIN:VEVENT",
            "UID:good@example.com",
            "SUMMARY:Good",
            "DTSTART;TZID=America/New_York:20261014T110000",
            "DTEND;TZID=America/New_York:20261014T120000",
            "RRULE:FREQ=DAILY;INTERVAL=2",
            "END:VEVENT",
        ]);
        let calendar = parse_calendar(&text).expect("parses");
        assert_eq!(
            calendar.series.len(),
            1,
            "INTERVAL={interval}: only the good one"
        );
        assert_eq!(calendar.series[0].interval, 2);
        assert_eq!(
            calendar.events.len(),
            1,
            "INTERVAL={interval}: the bad one is one task"
        );
        assert_eq!(calendar.unmapped.len(), 1, "INTERVAL={interval}");
        assert!(
            calendar.unmapped[0].contains("INTERVAL"),
            "{:?}",
            calendar.unmapped
        );
    }
    // The cap itself is fine.
    let calendar = rule_on_a_wednesday("FREQ=WEEKLY;INTERVAL=1000;COUNT=2");
    assert_eq!(calendar.series[0].interval, 1000);
    assert_eq!(
        calendar.series[0].until,
        Some(ymd(2026, 10, 14) + chrono::Days::new(7000))
    );
}

/// A week that starts on another day changes which weeks "every 2 weeks" skips: not
/// followed, so reported, unless it is the default Monday.
#[test]
fn a_week_start_other_than_monday_with_an_interval_is_reported() {
    let calendar = rule_on_a_wednesday("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;WKST=SU");
    assert!(calendar.series.is_empty());
    assert!(
        calendar.unmapped[0].contains("WKST"),
        "{:?}",
        calendar.unmapped
    );
    let monday = rule_on_a_wednesday("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;WKST=MO");
    assert_eq!(monday.series.len(), 1);
    // Every week, the week start does not matter.
    let every = rule_on_a_wednesday("FREQ=WEEKLY;BYDAY=MO,WE;WKST=SU");
    assert_eq!(every.series.len(), 1);
}

/// COUNT with a day step counts every second day from the start: Wed 14, Fri 16, Sun 18.
#[test]
fn daily_with_interval_and_count_ends_on_the_right_day() {
    let calendar = rule_on_a_wednesday("FREQ=DAILY;INTERVAL=2;COUNT=3");
    assert!(calendar.unmapped.is_empty(), "{:?}", calendar.unmapped);
    let series = &calendar.series[0];
    assert_eq!(series.interval, 2);
    assert_eq!(series.until, Some(ymd(2026, 10, 18)));
}

/// UNTIL with an interval: the named day is the last one, and the next step after it is
/// gone. UNTIL is UTC midnight 1 Nov, which is still Sat 31 Oct in New York.
#[test]
fn until_with_interval_stops_on_the_named_day() {
    let calendar = rule_on_a_wednesday("FREQ=WEEKLY;INTERVAL=2;BYDAY=WE;UNTIL=20261101T000000Z");
    assert!(calendar.unmapped.is_empty(), "{:?}", calendar.unmapped);
    let series = &calendar.series[0];
    assert_eq!(series.until, Some(ymd(2026, 10, 31)));
    let rule = Recurrence::with_interval([Weekday::Wed], 2, series.until).unwrap();
    assert!(
        rule.covers(series.start_day, ymd(2026, 10, 28)),
        "two weeks on"
    );
    assert!(
        !rule.covers(series.start_day, ymd(2026, 11, 11)),
        "four weeks on is past UNTIL"
    );
}

/// BYDAY days that fall before DTSTART in its week (Mon 12 Oct, DTSTART Wed 14) are not
/// occurrences, and the week after the step is skipped: Fri 16, then Mon 26 and Fri 30.
#[test]
fn by_day_before_the_start_with_an_interval_is_not_an_occurrence() {
    let calendar = rule_on_a_wednesday("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,FR;COUNT=3");
    assert!(calendar.unmapped.is_empty(), "{:?}", calendar.unmapped);
    let series = &calendar.series[0];
    assert_eq!(series.until, Some(ymd(2026, 10, 30)));
    let rule = Recurrence::with_interval([Weekday::Mon, Weekday::Fri], 2, series.until).unwrap();
    let start = series.start_day;
    assert!(
        !rule.covers(start, ymd(2026, 10, 12)),
        "Mon 12 is before the start"
    );
    assert!(rule.covers(start, ymd(2026, 10, 16)), "Fri 16 is the first");
    assert!(
        !rule.covers(start, ymd(2026, 10, 19)),
        "the week between is skipped"
    );
    assert!(
        rule.covers(start, ymd(2026, 10, 26)),
        "Mon 26 is two weeks on"
    );
    assert!(
        !rule.covers(start, ymd(2026, 11, 2)),
        "COUNT=3 ends on Fri 30"
    );
}

/// BYDAY takes a list, not a range: `MO-FR` names no weekday, so the rule falls back to one
/// task and says why.
#[test]
fn daily_by_day_range_falls_back_to_one_task() {
    let calendar = rule_on_a_wednesday("FREQ=DAILY;BYDAY=MO-FR");
    assert!(calendar.series.is_empty());
    assert_eq!(calendar.events.len(), 1);
    assert_eq!(calendar.unmapped.len(), 1);
    assert!(
        calendar.unmapped[0].contains("not a weekday"),
        "{:?}",
        calendar.unmapped
    );
}

/// INTERVAL of 0 and of 5000 (past the cap) fall back to one task, for a daily and a
/// weekly rule alike.
#[test]
fn interval_zero_and_5000_fall_back_to_one_task() {
    for freq in ["DAILY", "WEEKLY"] {
        for interval in ["0", "5000"] {
            let calendar = rule_on_a_wednesday(&format!("FREQ={freq};INTERVAL={interval}"));
            assert!(calendar.series.is_empty(), "{freq} INTERVAL={interval}");
            assert_eq!(calendar.events.len(), 1, "{freq} INTERVAL={interval}");
            assert_eq!(calendar.unmapped.len(), 1, "{freq} INTERVAL={interval}");
            assert!(
                calendar.unmapped[0].contains("INTERVAL"),
                "{:?}",
                calendar.unmapped
            );
        }
    }
}

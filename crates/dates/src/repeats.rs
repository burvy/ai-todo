//! Recurrence phrases (docs/ARCHITECTURE.md 7.1, Capture): what one clause says about
//! repeating, read on its own before `candidates` sees the words.
//!
//! A marker decides that a clause repeats, which is why this runs first: "gym every
//! Monday and Thursday" is one series, not two dated tasks. Nothing here stores a
//! series or knows `core`'s types; it only says what the words meant.

use std::ops::Range;

use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Weekday};
use todo_core::LocalContext;

use crate::phrases;
use crate::{MAX_WINDOW, Token, tokenize};

/// Monday to Friday are the first rows of [`phrases::WEEKDAYS`]: "every weekday".
const WEEKDAY_ROWS: usize = 5;

/// What a clause says about repeating, and the phrases the caller must leave out of the
/// title. Plain types, because `core` owns the stored `Recurrence` (7.1) and this
/// scanner is built before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceMatch {
    /// Byte spans of every phrase read here: the marker, the time, the end and the
    /// start day. The title is the clause minus these, the way `capture` already drops
    /// date phrases.
    pub title_spans: Vec<Range<usize>>,
    /// The days it repeats on, Monday first, without repeats. Never empty: a marker
    /// naming no day is no series.
    ///
    /// A `Vec` and not the `BTreeSet` 7.1 sketches, because `chrono::Weekday` is `Eq`
    /// and `Hash` but not `Ord`, so it cannot go in one. [`days_from`] sorts and dedups
    /// it into set behaviour until `core` settles on its own representation.
    pub weekdays: Vec<Weekday>,
    /// "at 6pm": the time every occurrence takes. None floats and is placed later.
    pub time: Option<NaiveTime>,
    /// "until December 15": the last day an occurrence exists, inclusive.
    pub until: Option<NaiveDate>,
    /// "starting October 30": the first week the series can appear in.
    pub start_day: Option<NaiveDate>,
}

/// The recurrence in `clause`, or `None` when it is a one-shot phrase.
pub fn recurrence(clause: &str, ctx: &LocalContext) -> Option<RecurrenceMatch> {
    let tokens = tokenize(clause);
    let marker = (0..tokens.len()).find_map(|index| marker_at(&tokens, index))?;
    let mut matched = RecurrenceMatch {
        title_spans: vec![span(&tokens, marker.first, marker.last)],
        weekdays: marker.weekdays,
        time: None,
        until: None,
        start_day: None,
    };
    let mut index = marker.last + 1;
    while index < tokens.len() {
        index += read_tail(&tokens, index, ctx, &mut matched);
    }
    Some(matched)
}

/// The words that make a clause repeat: a marker and the days it names.
struct Marker {
    weekdays: Vec<Weekday>,
    first: usize,
    last: usize,
}

/// The marker starting at `index`, with the days it names and the last word it used.
/// "every …" and "weekly on …" take the days that follow; a plural weekday is a marker
/// on its own, but can still head a list ("mondays and thursdays").
fn marker_at(tokens: &[Token], index: usize) -> Option<Marker> {
    let (days, last) = match word_at(tokens, index)? {
        "every" => {
            let (days, used) = days_from(&words_from(tokens, index + 1))?;
            (days, index + used)
        }
        "weekly" if word_at(tokens, index + 1) == Some("on") => {
            // One word past the days: the "on" between.
            let (days, used) = days_from(&words_from(tokens, index + 2))?;
            (days, index + used + 1)
        }
        // "mondays" repeats; a bare "Monday" is a date, and stays with `phrases::day`.
        // The list starts at the name itself, so its last word sits one before the count.
        plural if plural.ends_with('s') => {
            let (days, used) = days_from(&words_from(tokens, index))?;
            (days, index + used - 1)
        }
        _ => return None,
    };
    Some(Marker {
        weekdays: days,
        first: index,
        last,
    })
}

/// The days a marker names, with how many words they took: "monday", "weekday", "day",
/// or several of them ("monday and thursday", "monday, thursday, friday").
fn days_from(words: &[&str]) -> Option<(Vec<Weekday>, usize)> {
    let (mut days, mut used) = one_day(words)?;
    loop {
        let next = match words.get(used) {
            // "monday and thursday": the joiner is a word. A comma is not one — `tokenize`
            // drops it — so "monday, thursday" is the two words side by side.
            Some(&"and") => used + 1,
            Some(_) => used,
            None => break,
        };
        match one_day(&words[next..]) {
            Some((more, taken)) => {
                days.extend(more);
                used = next + taken;
            }
            // A conjunction with no day after it is an unfinished list, not a series.
            None if next > used => return None,
            None => break,
        }
    }
    days.sort_by_key(|day| day.num_days_from_monday());
    days.dedup();
    Some((days, used))
}

/// One named day, or "weekday" (Monday to Friday) or "day" (all seven).
fn one_day(words: &[&str]) -> Option<(Vec<Weekday>, usize)> {
    let days: Vec<Weekday> = match *words.first()? {
        "day" | "days" => phrases::WEEKDAYS.iter().map(|(_, day)| *day).collect(),
        "weekday" | "weekdays" => phrases::WEEKDAYS[..WEEKDAY_ROWS]
            .iter()
            .map(|(_, day)| *day)
            .collect(),
        name => vec![phrases::weekday(name)?],
    };
    Some((days, 1))
}

/// One phrase from the words after the marker.
enum Tail {
    Time(NaiveTime),
    End(NaiveDate),
    Start(NaiveDate),
}

/// How many words to step over at `index`: the phrase read there, or 1 when those words
/// are title text. The words a phrase used are not read again.
fn read_tail(
    tokens: &[Token],
    index: usize,
    ctx: &LocalContext,
    matched: &mut RecurrenceMatch,
) -> usize {
    let Some((used, tail)) = tail_at(tokens, index, ctx) else {
        return 1;
    };
    // The first phrase of a kind wins: a clause that names two times has one series.
    match tail {
        Tail::Time(time) => {
            matched.time.get_or_insert(time);
        }
        Tail::End(day) => {
            matched.until.get_or_insert(day);
        }
        Tail::Start(day) => {
            matched.start_day.get_or_insert(day);
        }
    }
    matched
        .title_spans
        .push(span(tokens, index, index + used - 1));
    used
}

/// The tail phrase starting at `index`, with the words it uses. A day phrase needs its
/// marker ("until", "starting") to mean anything to a series; a bare clock time does
/// not, which is where the one-shot path picks it up too.
fn tail_at(tokens: &[Token], index: usize, ctx: &LocalContext) -> Option<(usize, Tail)> {
    let words = words_from(tokens, index);
    if let Some((used, day)) = end_of_the_month(&words, ctx) {
        return Some((used, Tail::End(day)));
    }
    let first = *words.first()?;
    let (used, tail) = match first {
        "until" => {
            let (used, day) = end_after(&words[1..], ctx)?;
            (used + 1, Tail::End(day))
        }
        "starting" | "beginning" | "from" => {
            // "starting on October 30" spends a word on "on"; "from October 30" does not.
            let on = usize::from(first != "from" && words.get(1) == Some(&"on"));
            let (used, day) = day_after(&words[1 + on..], ctx)?;
            (used + 1 + on, Tail::Start(day))
        }
        _ => {
            let (used, time) = time_after(&words)?;
            return Some((used, Tail::Time(time)));
        }
    };
    Some((used, tail))
}

/// The last day an occurrence exists: "end of the month", a day phrase after "until", or
/// a bare month read as the month before it ("until December" ends with November).
fn end_after(words: &[&str], ctx: &LocalContext) -> Option<(usize, NaiveDate)> {
    if let Some(end) = end_of_the_month(words, ctx) {
        return Some(end);
    }
    // A day phrase beats a bare month: "until December 15" names a day, not a month.
    day_after(words, ctx).or_else(|| month_before_end(words, ctx))
}

/// "until December": the last day of the month before, in the coming year once that
/// month has gone by.
fn month_before_end(words: &[&str], ctx: &LocalContext) -> Option<(usize, NaiveDate)> {
    let month = words.first().copied().and_then(phrases::month_of)?;
    let today = ctx.today();
    let end = [today.year(), today.year() + 1]
        .into_iter()
        .find_map(|year| month_before(year, month).filter(|day| *day >= today))?;
    Some((1, end))
}

/// "end of the month": the last day of the month in hand. Four words, longer than any
/// other phrase here, so it is spelled out rather than window-matched.
fn end_of_the_month(words: &[&str], ctx: &LocalContext) -> Option<(usize, NaiveDate)> {
    if !words.starts_with(&["end", "of", "the", "month"]) {
        return None;
    }
    let today = ctx.today();
    Some((4, last_day_of(today.year(), today.month())?))
}

/// The longest day phrase in `words`, with the words it took, the way `candidates`
/// reads one ("until October 30, 2026" beats "until October 30").
fn day_after(words: &[&str], ctx: &LocalContext) -> Option<(usize, NaiveDate)> {
    (1..=MAX_WINDOW).rev().find_map(|len| {
        let window = words.get(..len)?;
        phrases::day(window, ctx).map(|day| (len, day))
    })
}

/// The longest clock time in `words`, with the words it took.
fn time_after(words: &[&str]) -> Option<(usize, NaiveTime)> {
    (1..=MAX_WINDOW).rev().find_map(|len| {
        let window = words.get(..len)?;
        phrases::time(window).map(|time| (len, time))
    })
}

/// The last day of the month before `month` in `year`. January reaches back into the
/// year before, since a year has no month zero.
fn month_before(year: i32, month: u32) -> Option<NaiveDate> {
    let (year, month) = if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    };
    last_day_of(year, month)
}

/// The last day of `month` in `year`: the first of the next month, less a day.
fn last_day_of(year: i32, month: u32) -> Option<NaiveDate> {
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    };
    next.map(|first| first - Duration::days(1))
}

/// The byte span of tokens `first..=last`, from the first word's start to the last
/// word's end.
fn span(tokens: &[Token], first: usize, last: usize) -> Range<usize> {
    tokens[first].start..tokens[last].end
}

fn word_at(tokens: &[Token], index: usize) -> Option<&str> {
    tokens.get(index).map(|token| token.word.as_str())
}

fn words_from(tokens: &[Token], from: usize) -> Vec<&str> {
    tokens
        .iter()
        .skip(from)
        .map(|token| token.word.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Weekday::{Fri, Mon, Sat, Thu, Tue, Wed};
    use chrono::{TimeZone, Utc};

    /// Sunday 2026-10-11 21:00 in New York, the crate's usual clock.
    fn ctx() -> LocalContext {
        LocalContext {
            now: Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
            tz: chrono_tz::America::New_York,
        }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn time(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    /// The days a series repeats on, in the order the scanner returns them.
    fn days(list: &[Weekday]) -> Vec<Weekday> {
        let mut days = list.to_vec();
        days.sort_by_key(|day| day.num_days_from_monday());
        days
    }

    const EVERY_DAY: [Weekday; 7] = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ];
    const WEEKDAYS: [Weekday; 5] = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
    ];

    /// The span of `phrase` inside `clause`, which every row below can point at.
    fn span_of(clause: &str, phrase: &str) -> Range<usize> {
        let start = clause
            .find(phrase)
            .unwrap_or_else(|| panic!("{phrase:?} is not in {clause:?}"));
        start..start + phrase.len()
    }

    /// Every phrase in 7.1's Capture list, from Sunday 2026-10-11: the marker and its
    /// days, "at" a time, "until" an end and "starting" a day.
    #[test]
    fn the_recurrence_phrases_of_the_design() {
        let rows = [
            (
                "gym every Monday and Thursday",
                RecurrenceMatch {
                    title_spans: vec![span_of(
                        "gym every Monday and Thursday",
                        "every Monday and Thursday",
                    )],
                    weekdays: days(&[Mon, Thu]),
                    time: None,
                    until: None,
                    start_day: None,
                },
            ),
            (
                "every weekday until December",
                RecurrenceMatch {
                    title_spans: vec![
                        span_of("every weekday until December", "every weekday"),
                        span_of("every weekday until December", "until December"),
                    ],
                    weekdays: days(&WEEKDAYS),
                    time: None,
                    // A bare month after "until" ends with the month before it.
                    until: Some(date(2026, 11, 30)),
                    start_day: None,
                },
            ),
            (
                "weekly on Tue and Fri at 6pm",
                RecurrenceMatch {
                    title_spans: vec![
                        span_of("weekly on Tue and Fri at 6pm", "weekly on Tue and Fri"),
                        span_of("weekly on Tue and Fri at 6pm", "at 6pm"),
                    ],
                    weekdays: days(&[Tue, Fri]),
                    time: Some(time(18, 0)),
                    until: None,
                    start_day: None,
                },
            ),
            (
                "standup every day at 9",
                RecurrenceMatch {
                    title_spans: vec![
                        span_of("standup every day at 9", "every day"),
                        span_of("standup every day at 9", "at 9"),
                    ],
                    weekdays: days(&EVERY_DAY),
                    time: Some(time(9, 0)),
                    until: None,
                    start_day: None,
                },
            ),
            (
                "mondays",
                RecurrenceMatch {
                    title_spans: vec![span_of("mondays", "mondays")],
                    weekdays: days(&[Mon]),
                    time: None,
                    until: None,
                    start_day: None,
                },
            ),
            (
                "water the plants on mondays and thursdays",
                RecurrenceMatch {
                    title_spans: vec![span_of(
                        "water the plants on mondays and thursdays",
                        "mondays and thursdays",
                    )],
                    weekdays: days(&[Mon, Thu]),
                    time: None,
                    until: None,
                    start_day: None,
                },
            ),
            (
                "gym every Monday, Wednesday and Friday",
                RecurrenceMatch {
                    title_spans: vec![span_of(
                        "gym every Monday, Wednesday and Friday",
                        "every Monday, Wednesday and Friday",
                    )],
                    weekdays: days(&[Mon, Wed, Fri]),
                    time: None,
                    until: None,
                    start_day: None,
                },
            ),
            (
                "weekly on saturdays",
                RecurrenceMatch {
                    title_spans: vec![span_of("weekly on saturdays", "weekly on saturdays")],
                    weekdays: days(&[Sat]),
                    time: None,
                    until: None,
                    start_day: None,
                },
            ),
        ];
        let ctx = ctx();
        for (clause, expected) in rows {
            assert_eq!(
                recurrence(clause, &ctx).as_ref(),
                Some(&expected),
                "{clause}"
            );
        }
    }

    /// "until" takes anything `day()` already reads, plus "end of the month" on its own
    /// or after "until", and reads the same whichever side of the marker it sits on.
    #[test]
    fn the_ends_a_series_can_have() {
        let ctx = ctx();
        for (clause, until) in [
            ("every Tuesday until December 15", date(2026, 12, 15)),
            ("every Tuesday until Friday", date(2026, 10, 16)),
            ("every Tuesday until 2026-12-15", date(2026, 12, 15)),
            ("every Tuesday until end of the month", date(2026, 10, 31)),
            ("every Tuesday end of the month", date(2026, 10, 31)),
        ] {
            let found = recurrence(clause, &ctx)
                .unwrap_or_else(|| panic!("{clause} should be a series"))
                .until;
            assert_eq!(found, Some(until), "{clause}");
        }
        let both_ways = recurrence("every weekday until December at 9", &ctx).expect("a series");
        assert_eq!(both_ways.until, Some(date(2026, 11, 30)));
        assert_eq!(both_ways.time, Some(time(9, 0)));
        // "until December" ends in November, and rolls to the coming November once that
        // month has gone by.
        let after = LocalContext {
            now: Utc.with_ymd_and_hms(2026, 12, 2, 17, 0, 0).unwrap(),
            tz: chrono_tz::America::New_York,
        };
        assert_eq!(
            recurrence("every weekday until December", &after)
                .unwrap()
                .until,
            Some(date(2027, 11, 30))
        );
    }

    /// A one-shot date beside a recurrence marker is the day the series starts.
    #[test]
    fn a_date_beside_the_marker_is_the_start_day() {
        let ctx = ctx();
        for clause in [
            "submit the report every Friday starting October 30",
            "submit the report every Friday starting on October 30",
            "submit the report every Friday from October 30",
            "submit the report every Friday beginning on October 30",
        ] {
            let found =
                recurrence(clause, &ctx).unwrap_or_else(|| panic!("{clause} should be a series"));
            assert_eq!(found.weekdays, days(&[Fri]), "{clause}");
            assert_eq!(found.start_day, Some(date(2026, 10, 30)), "{clause}");
        }
    }

    /// The clause keeps only its own words: every span the scanner read is a phrase the
    /// title must leave out.
    #[test]
    fn the_spans_are_the_phrases_left_out_of_the_title() {
        let ctx = ctx();
        let clause = "gym every Monday and Thursday at 6pm until December";
        let found = recurrence(clause, &ctx).expect("a series");
        let title: String = {
            let mut out = String::new();
            let mut cursor = 0;
            for span in &found.title_spans {
                out.push_str(&clause[cursor..span.start]);
                cursor = span.end;
            }
            out.push_str(&clause[cursor..]);
            out
        };
        assert_eq!(title.trim(), "gym");
    }

    /// A weekday with no marker is a date, a clause with no days at all is a task, and
    /// a rhythm this scanner does not read is not a weekly series either: 7.1 leaves
    /// monthly and biweekly rules out of scope.
    #[test]
    fn one_shot_clauses_are_not_series() {
        let ctx = ctx();
        for clause in [
            "a thing next Monday",
            "call mom on Thursday",
            "buy milk",
            "every",
            "every week",
            "every Monday and",
            "every other Monday",
            "pay the rent every month",
            "always take the bins out",
            "weekly",
        ] {
            assert_eq!(recurrence(clause, &ctx), None, "{clause}");
        }
    }

    /// A phrase that reaches out of the clause is not one: a weekday with nothing
    /// before it, or "until" with no end.
    #[test]
    fn phrases_that_run_out_of_words_are_not_series() {
        let ctx = ctx();
        assert_eq!(recurrence("every", &ctx), None);
        assert_eq!(recurrence("every until December", &ctx), None);
        assert_eq!(recurrence("every Monday until", &ctx).unwrap().until, None);
    }
}

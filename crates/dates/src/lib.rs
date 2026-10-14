//! Text to dates, times and durations (ARCHITECTURE.md section 5, step 2).
//!
//! Code finds the phrases and resolves each one against a [`LocalContext`], so every
//! candidate already carries its value. Jev only chooses among these candidates.

mod phrases;
mod repeats;
pub mod tuning;

pub use repeats::{RecurrenceMatch, recurrence};

use std::ops::Range;

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, SubsecRound, Timelike, Utc};
use todo_core::{LocalContext, When};

/// Longest phrase the resolvers accept, in words ("in half an hour", "October 30, 2026").
const MAX_WINDOW: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Day(NaiveDate),
    Time(NaiveTime),
    /// Minutes.
    Duration(u16),
    /// A time that many minutes after now: "in 10 minutes", "in an hour". It is an
    /// instant, so it needs no day: [`after`] puts it on the clock.
    After(u32),
}

/// The instant `minutes` after now, to the minute: "in 10 minutes" asked at 10:03:27 is
/// 10:13, so the task does not carry the seconds the user happened to speak at.
pub fn after(minutes: u32, ctx: &LocalContext) -> DateTime<Utc> {
    let now = ctx.now.trunc_subsecs(0);
    let on_the_minute = now - Duration::seconds(i64::from(now.second()));
    on_the_minute + Duration::minutes(i64::from(minutes))
}

/// A phrase found in the text, with its byte span and resolved value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub span: Range<usize>,
    pub text: String,
    pub value: Value,
}

/// Day, time and duration from one text. Each is the first candidate of its kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub day: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    pub duration_min: Option<u16>,
}

/// Every date, time and duration phrase in `text`, left to right, with byte spans
/// into `text`. Phrases do not overlap.
pub fn candidates(text: &str, ctx: &LocalContext) -> Vec<Candidate> {
    let tokens = tokenize(text);
    let mut found = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let longest = MAX_WINDOW.min(tokens.len() - index);
        let matched = (1..=longest).rev().find_map(|len| {
            let window: Vec<&str> = tokens[index..index + len]
                .iter()
                .map(|token| token.word.as_str())
                .collect();
            resolve_window(&window, ctx).map(|value| (len, value))
        });
        match matched {
            Some((len, value)) => {
                let span = tokens[index].start..tokens[index + len - 1].end;
                found.push(Candidate {
                    text: text[span.clone()].to_owned(),
                    span,
                    value,
                });
                index += len;
            }
            None => index += 1,
        }
    }
    found
}

/// The first day, time and duration in `text`.
pub fn resolve(text: &str, ctx: &LocalContext) -> Resolved {
    let mut resolved = Resolved::default();
    for candidate in candidates(text, ctx) {
        match candidate.value {
            Value::Day(day) => {
                resolved.day.get_or_insert(day);
            }
            Value::Time(time) => {
                resolved.time.get_or_insert(time);
            }
            Value::Duration(minutes) => {
                resolved.duration_min.get_or_insert(minutes);
            }
            // An instant has no day or clock of its own to report here.
            Value::After(_) => {}
        }
    }
    resolved
}

/// The scheduling rule for a capture, from the day and time already chosen: day + time
/// pins the task, a day alone makes it due by `deadline` (the user's day-only deadline
/// time) that day, and anything else floats.
pub fn when_from(
    day: Option<NaiveDate>,
    time: Option<NaiveTime>,
    deadline: NaiveTime,
    ctx: &LocalContext,
) -> When {
    match (day, time) {
        (Some(day), Some(time)) => When::At(ctx.at(day, time)),
        (Some(day), None) => When::DueBy(ctx.at(day, deadline)),
        _ => When::Anytime,
    }
}

fn resolve_window(words: &[&str], ctx: &LocalContext) -> Option<Value> {
    phrases::day(words, ctx)
        .map(Value::Day)
        .or_else(|| phrases::time(words).map(Value::Time))
        .or_else(|| phrases::minutes_from_now(words).map(Value::After))
        .or_else(|| phrases::duration(words).map(Value::Duration))
}

struct Token {
    /// Lowercase, without edge punctuation.
    word: String,
    start: usize,
    end: usize,
}

/// Splits on whitespace and keeps byte offsets into the original text.
fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut begin = None;
    for (index, ch) in text.char_indices().chain([(text.len(), ' ')]) {
        if ch.is_whitespace() {
            if let Some(start) = begin.take() {
                push_token(text, start, index, &mut tokens);
            }
        } else if begin.is_none() {
            begin = Some(index);
        }
    }
    tokens
}

fn push_token(text: &str, begin: usize, end: usize, tokens: &mut Vec<Token>) {
    let raw = &text[begin..end];
    let is_edge = |c: char| !(c.is_alphanumeric() || matches!(c, ':' | '/' | '-'));
    let front = raw.trim_start_matches(is_edge);
    let core = front.trim_end_matches(is_edge);
    if core.is_empty() {
        return;
    }
    let start = begin + (raw.len() - front.len());
    tokens.push(Token {
        word: core.to_lowercase(),
        start,
        end: start + core.len(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    /// A day and a time together: the exact instant. Without both, `None`.
    fn pinned_start(text: &str, ctx: &LocalContext) -> Option<DateTime<Utc>> {
        let resolved = resolve(text, ctx);
        Some(ctx.at(resolved.day?, resolved.time?))
    }
    use chrono::{Datelike, TimeZone, Weekday};

    fn ny(now: DateTime<Utc>) -> LocalContext {
        LocalContext {
            now,
            tz: chrono_tz::America::New_York,
        }
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn time(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    /// Sunday 2026-10-11 21:00 in New York, which is already Monday in UTC.
    fn sunday_night() -> LocalContext {
        ny(utc(2026, 10, 12, 1, 0))
    }

    #[test]
    fn sunday_night_tomorrow_is_monday_not_tuesday() {
        let ctx = sunday_night();
        assert_eq!(phrases::day(&["today"], &ctx), Some(date(2026, 10, 11)));
        assert_eq!(phrases::day(&["tomorrow"], &ctx), Some(date(2026, 10, 12)));
        assert_eq!(phrases::day(&["monday"], &ctx), Some(date(2026, 10, 12)));
    }

    #[test]
    fn every_weekday_from_every_weekday_is_one_to_seven_days_away() {
        for offset in 0..7 {
            let base = date(2026, 10, 11) + chrono::Duration::days(offset);
            let ctx = ny(utc(base.year(), base.month(), base.day(), 16, 0));
            for (name, target) in phrases::WEEKDAYS {
                let found = phrases::day(&[name], &ctx).expect(name);
                assert_eq!(found.weekday(), target, "{name} from {base}");
                let ahead = (found - ctx.today()).num_days();
                // Before 17:00 a same-day name is today; otherwise it is 1 to 7 days ahead.
                assert!(
                    (0..=7).contains(&ahead),
                    "{name} from {base} was {ahead} days"
                );
                assert!(ahead != 0 || found == ctx.today(), "{name} from {base}");
            }
        }
    }
    #[test]
    fn bare_3pm_resolves_to_3pm_without_at() {
        assert_eq!(phrases::time(&["3pm"]), Some(time(15, 0)));
    }

    #[test]
    fn next_tuesday_on_a_tuesday_is_a_week_away() {
        let ctx = ny(utc(2026, 10, 13, 16, 0));
        assert_eq!(ctx.today().weekday(), Weekday::Tue);
        assert_eq!(
            phrases::day(&["next", "tuesday"], &ctx),
            Some(date(2026, 10, 20))
        );
    }

    #[test]
    fn bare_weekday_on_its_own_day_is_today_before_five_pm() {
        // 12:00 local on Tuesday.
        let ctx = ny(utc(2026, 10, 13, 16, 0));
        assert_eq!(phrases::day(&["tuesday"], &ctx), Some(date(2026, 10, 13)));
    }

    #[test]
    fn bare_weekday_on_its_own_day_is_a_week_away_after_five_pm() {
        // 17:00 local on Tuesday exactly.
        let ctx = ny(utc(2026, 10, 13, 21, 0));
        assert_eq!(phrases::day(&["tuesday"], &ctx), Some(date(2026, 10, 20)));
    }

    #[test]
    fn weekend_is_today_on_saturday_before_five_pm_only() {
        let before = ny(utc(2026, 10, 17, 16, 0));
        assert_eq!(
            phrases::day(&["weekend"], &before),
            Some(date(2026, 10, 17))
        );
        let after = ny(utc(2026, 10, 17, 21, 30));
        assert_eq!(phrases::day(&["weekend"], &after), Some(date(2026, 10, 24)));
    }

    #[test]
    fn by_friday_from_monday_is_this_friday() {
        let ctx = ny(utc(2026, 10, 12, 16, 0));
        assert_eq!(
            phrases::day(&["by", "friday"], &ctx),
            Some(date(2026, 10, 16))
        );
    }

    #[test]
    fn next_week_from_wednesday_is_the_coming_monday() {
        let ctx = ny(utc(2026, 10, 14, 16, 0));
        assert_eq!(
            phrases::day(&["next", "week"], &ctx),
            Some(date(2026, 10, 19))
        );
    }

    #[test]
    fn month_and_year_roll_over() {
        let ctx = ny(utc(2026, 12, 30, 17, 0));
        assert_eq!(phrases::day(&["jan", "5"], &ctx), Some(date(2027, 1, 5)));
        assert_eq!(phrases::day(&["dec", "31"], &ctx), Some(date(2026, 12, 31)));
        assert_eq!(phrases::day(&["dec", "30"], &ctx), Some(date(2026, 12, 30)));
        assert_eq!(phrases::day(&["dec", "29"], &ctx), Some(date(2027, 12, 29)));
        assert_eq!(
            phrases::day(&["october", "30", "2026"], &ctx),
            Some(date(2026, 10, 30))
        );
    }

    #[test]
    fn numeric_dates() {
        let ctx = sunday_night();
        assert_eq!(
            phrases::day(&["2026-10-30"], &ctx),
            Some(date(2026, 10, 30))
        );
        assert_eq!(
            phrases::day(&["10/30/2026"], &ctx),
            Some(date(2026, 10, 30))
        );
        assert_eq!(phrases::day(&["2026-02-30"], &ctx), None);
    }

    #[test]
    fn clock_times() {
        assert_eq!(phrases::time(&["at", "3"]), Some(time(15, 0)));
        assert_eq!(phrases::time(&["at", "9"]), Some(time(9, 0)));
        assert_eq!(phrases::time(&["at", "12"]), Some(time(12, 0)));
        assert_eq!(phrases::time(&["at", "3pm"]), Some(time(15, 0)));
        assert_eq!(phrases::time(&["at", "3", "pm"]), Some(time(15, 0)));
        assert_eq!(phrases::time(&["10am"]), Some(time(10, 0)));
        assert_eq!(phrases::time(&["3:30", "pm"]), Some(time(15, 30)));
        assert_eq!(phrases::time(&["15:45"]), Some(time(15, 45)));
        assert_eq!(phrases::time(&["noon"]), Some(time(12, 0)));
        assert_eq!(phrases::time(&["3"]), None);
        assert_eq!(phrases::time(&["at", "13"]), None);
    }

    fn after_of(text: &str, ctx: &LocalContext) -> Option<DateTime<Utc>> {
        candidates(text, ctx)
            .iter()
            .find_map(|candidate| match candidate.value {
                Value::After(minutes) => Some(after(minutes, ctx)),
                _ => None,
            })
    }

    #[test]
    fn relative_times_are_that_long_after_now_to_the_minute() {
        let now = DateTime::parse_from_rfc3339("2026-10-14T14:03:27Z")
            .unwrap()
            .with_timezone(&Utc);
        let ctx = ny(now);
        let at = |h, m| utc(2026, 10, 14, h, m);
        for (text, expected) in [
            ("wake up in 10 minutes", at(14, 13)),
            ("in 10 min", at(14, 13)),
            ("in 10mins", at(14, 13)),
            ("in a minute", at(14, 4)),
            ("in an hour", at(15, 3)),
            ("in 1 hour", at(15, 3)),
            ("in one hour", at(15, 3)),
            ("in half an hour", at(14, 33)),
            ("in half hour", at(14, 33)),
            ("in 90 minutes", at(15, 33)),
            ("in 2 hours", at(16, 3)),
            ("in two hours", at(16, 3)),
            ("in 3hrs", at(17, 3)),
            ("in twenty minutes", at(14, 23)),
            ("in forty five minutes", at(14, 48)),
            ("in forty-five minutes", at(14, 48)),
            ("in fifteen minutes", at(14, 18)),
            ("in sixty minutes", at(15, 3)),
            ("in twenty five minutes", at(14, 28)),
        ] {
            assert_eq!(after_of(text, &ctx), Some(expected), "{text}");
        }
    }

    #[test]
    fn a_relative_phrase_is_one_candidate_that_covers_its_whole_text() {
        let ctx = ny(utc(2026, 10, 14, 14, 0));
        let found = candidates("Wake up in half an hour please", &ctx);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "in half an hour");
        // Without "in" it is still a task's length.
        assert_eq!(resolve("a half an hour walk", &ctx).duration_min, Some(30));
        assert_eq!(resolve("a 90 minutes call", &ctx).duration_min, Some(90));
    }

    #[test]
    fn relative_days_and_weeks_are_dates() {
        let ctx = ny(utc(2026, 10, 14, 14, 0));
        for (text, days) in [
            ("in a day", 1),
            ("in 2 days", 2),
            ("in two days", 2),
            ("in a week", 7),
            ("in 1 week", 7),
            ("in 2 weeks", 14),
            ("in 3 days at 5pm", 3),
        ] {
            assert_eq!(
                resolve(text, &ctx).day,
                Some(date(2026, 10, 14) + Duration::days(days)),
                "{text}"
            );
        }
        assert_eq!(resolve("in 3 days at 5pm", &ctx).time, Some(time(17, 0)));
    }

    #[test]
    fn in_a_month_is_the_same_day_next_month_or_the_last_day_of_a_shorter_one() {
        for (from, text, expected) in [
            (date(2026, 10, 14), "in a month", date(2026, 11, 14)),
            (date(2026, 10, 14), "in 2 months", date(2026, 12, 14)),
            (date(2026, 12, 20), "in a month", date(2027, 1, 20)),
            (date(2026, 1, 31), "in a month", date(2026, 2, 28)),
            (date(2028, 1, 31), "in one month", date(2028, 2, 29)),
        ] {
            let ctx = ny(utc(from.year(), from.month(), from.day(), 15, 0));
            assert_eq!(
                resolve(text, &ctx).day,
                Some(expected),
                "{text} from {from}"
            );
        }
    }

    #[test]
    fn spelled_numbers_up_to_sixty_are_durations_too() {
        let ctx = ny(utc(2026, 10, 14, 14, 0));
        for (text, minutes) in [
            ("a twenty minutes walk", 20),
            ("a forty five minutes call", 45),
            ("thirty minutes", 30),
            ("sixty minutes", 60),
        ] {
            assert_eq!(resolve(text, &ctx).duration_min, Some(minutes), "{text}");
        }
        // Not numbers this code reads: three hundred, twenty-ten, thirty one-tens.
        assert!(resolve("seventy minutes", &ctx).duration_min.is_none());
        assert!(resolve("twenty-ten minutes", &ctx).duration_min.is_none());
    }

    #[test]
    fn relative_phrases_that_are_nonsense_are_left_alone() {
        let ctx = ny(utc(2026, 10, 14, 14, 0));
        for text in [
            "in 0 minutes",
            "in 99999 days",
            "in a while",
            "in the hour",
            "in 5",
            "in time",
        ] {
            assert!(
                candidates(text, &ctx)
                    .iter()
                    .all(|c| !matches!(c.value, Value::After(_) | Value::Day(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn near_midnight_a_relative_time_crosses_the_local_date() {
        // 23:55 in New York on the 14th (03:55Z on the 15th): ten minutes is 00:05 local.
        let ctx = ny(utc(2026, 10, 15, 3, 55));
        let at = after_of("in 10 minutes", &ctx).unwrap();
        assert_eq!(ctx.local(at).date(), date(2026, 10, 15));
        assert_eq!(ctx.local(at).time(), time(0, 5));
    }

    #[test]
    fn a_relative_time_is_exact_across_the_fall_back() {
        // 00:30 EDT on 1 Nov 2026 (04:30Z): two hours later is 01:30 EST, three hours of
        // wall clock earlier than a naive 02:30.
        let ctx = ny(utc(2026, 11, 1, 4, 30));
        let at = after_of("in 2 hours", &ctx).unwrap();
        assert_eq!(at, utc(2026, 11, 1, 6, 30));
        assert_eq!(ctx.local(at).time(), time(1, 30));
    }

    #[test]
    fn a_relative_time_is_exact_across_the_spring_forward() {
        // 01:30 EST on 8 Mar 2026 (06:30Z): one hour later is 03:30 EDT, the gap skipped.
        let ctx = ny(utc(2026, 3, 8, 6, 30));
        let at = after_of("in an hour", &ctx).unwrap();
        assert_eq!(at, utc(2026, 3, 8, 7, 30));
        assert_eq!(ctx.local(at).time(), time(3, 30));
    }

    #[test]
    fn a_relative_day_across_the_fall_back_keeps_the_clock_date() {
        // "in 2 days" from 30 Oct 2026 is 1 Nov, a 25-hour day: a date, not 48 hours.
        let ctx = ny(utc(2026, 10, 30, 14, 0));
        assert_eq!(resolve("in 2 days", &ctx).day, Some(date(2026, 11, 1)));
    }

    #[test]
    fn duration_phrases() {
        assert_eq!(phrases::duration(&["an", "hour"]), Some(60));
        assert_eq!(phrases::duration(&["half", "an", "hour"]), Some(30));
        assert_eq!(phrases::duration(&["90", "minutes"]), Some(90));
        assert_eq!(phrases::duration(&["2", "hours"]), Some(120));
        assert_eq!(phrases::duration(&["two", "hours"]), Some(120));
        assert_eq!(phrases::duration(&["45", "min"]), Some(45));
        assert_eq!(phrases::duration(&["45min"]), Some(45));
        assert_eq!(phrases::duration(&["hour"]), None);
    }

    #[test]
    fn resolve_finds_each_kind_in_a_sentence() {
        let ctx = sunday_night();
        let resolved = resolve("lab report due Thursday at 2, about an hour", &ctx);
        assert_eq!(resolved.day, Some(date(2026, 10, 15)));
        assert_eq!(resolved.time, Some(time(14, 0)));
        assert_eq!(resolved.duration_min, Some(60));
    }

    #[test]
    fn candidates_carry_byte_spans_that_match_their_text() {
        let ctx = sunday_night();
        let text = "café tomorrow at 3, 45 min";
        let found = candidates(text, &ctx);
        assert_eq!(found.len(), 3);
        for candidate in &found {
            assert_eq!(&text[candidate.span.clone()], candidate.text);
        }
        assert_eq!(found[0].text, "tomorrow");
        assert_eq!(found[1].text, "at 3");
        assert_eq!(found[2].text, "45 min");
    }

    #[test]
    fn first_day_wins_when_two_are_named() {
        let ctx = sunday_night();
        assert_eq!(
            resolve("Thursday or Friday", &ctx).day,
            Some(date(2026, 10, 15))
        );
    }

    /// "at 3:30" is the afternoon, like "at 3"; a 24-hour time or an "am" is kept.
    #[test]
    fn a_time_with_minutes_after_at_follows_the_bare_hour_rule() {
        let ctx = sunday_night();
        let time = |text| resolve(text, &ctx).time;
        assert_eq!(time("call mom at 3:30"), NaiveTime::from_hms_opt(15, 30, 0));
        assert_eq!(time("call mom at 3"), NaiveTime::from_hms_opt(15, 0, 0));
        assert_eq!(
            time("call mom at 3:30am"),
            NaiveTime::from_hms_opt(3, 30, 0)
        );
        assert_eq!(time("call mom at 9:15"), NaiveTime::from_hms_opt(9, 15, 0));
        assert_eq!(
            time("call mom at 15:30"),
            NaiveTime::from_hms_opt(15, 30, 0)
        );
        assert_eq!(time("call mom at 0:30"), NaiveTime::from_hms_opt(0, 30, 0));
    }

    /// chrono reads a two-digit `%Y` literally; year 26 is not what anyone meant.
    #[test]
    fn a_two_digit_year_is_not_a_date_in_year_26() {
        let ctx = sunday_night();
        assert_eq!(resolve("pay rent by 10/30/26", &ctx).day, None);
        assert_eq!(
            resolve("pay rent by 10/30/2026", &ctx).day,
            Some(date(2026, 10, 30))
        );
    }

    /// A huge number of hours is not a duration, and never an overflow.
    #[test]
    fn an_absurd_duration_is_ignored_not_an_overflow() {
        let ctx = sunday_night();
        assert_eq!(resolve("nap for 71582789 hours", &ctx).duration_min, None);
        assert_eq!(resolve("nap 71582789h", &ctx).duration_min, None);
    }

    #[test]
    fn spring_forward_pin_moves_the_gap_forward() {
        let ctx = ny(utc(2026, 3, 1, 12, 0));
        assert_eq!(
            pinned_start("2026-03-08 at 2:30am", &ctx),
            Some(utc(2026, 3, 8, 7, 30))
        );
    }

    #[test]
    fn fall_back_pin_takes_the_earlier_instant() {
        let ctx = ny(utc(2026, 10, 1, 12, 0));
        assert_eq!(
            pinned_start("2026-11-01 at 1:30am", &ctx),
            Some(utc(2026, 11, 1, 5, 30))
        );
    }

    fn when(text: &str, deadline: NaiveTime, ctx: &LocalContext) -> When {
        let resolved = resolve(text, ctx);
        when_from(resolved.day, resolved.time, deadline, ctx)
    }

    #[test]
    fn when_follows_the_capture_rule() {
        let ctx = sunday_night();
        let deadline = todo_core::DaySettings::default().deadline;
        assert_eq!(
            when("tomorrow at 3", deadline, &ctx),
            When::At(utc(2026, 10, 12, 19, 0))
        );
        // Day alone: due 23:59 New York time on Monday, 03:59 UTC on Tuesday.
        assert_eq!(
            when("tomorrow", deadline, &ctx),
            When::DueBy(utc(2026, 10, 13, 3, 59))
        );
        assert_eq!(when("at 3", deadline, &ctx), When::Anytime);
        assert_eq!(when("buy milk", deadline, &ctx), When::Anytime);
    }

    #[test]
    fn the_day_only_deadline_follows_the_setting() {
        let ctx = sunday_night();
        let five = NaiveTime::from_hms_opt(17, 0, 0).unwrap();
        assert_eq!(
            when("tomorrow", five, &ctx),
            When::DueBy(utc(2026, 10, 12, 21, 0))
        );
    }

    #[test]
    fn tomorrow_at_23_59_and_00_01() {
        // Sunday 2026-10-11 21:00 NY -> Monday 2026-10-12 01:00 UTC
        let ctx = ny(utc(2026, 10, 12, 1, 0));
        assert_eq!(phrases::day(&["tomorrow"], &ctx), Some(date(2026, 10, 12)));
        // At 23:59 local => Mon 23:59 NY = Tue 03:59 UTC (next day)
        assert_eq!(
            pinned_start("tomorrow at 23:59", &ctx),
            Some(utc(2026, 10, 13, 3, 59))
        );
        // At 00:01 local => Mon 00:01 NY = Mon 04:01 UTC (same day, early)
        let ctx_early = ny(utc(2026, 10, 12, 5, 0)); // Mon 01:00 NY
        assert_eq!(
            pinned_start("tomorrow at 0:01", &ctx_early),
            Some(utc(2026, 10, 13, 4, 1))
        );
    }

    #[test]
    fn next_monday_on_monday_is_next_week() {
        // Monday 2026-10-12 16:00 NY
        let ctx = ny(utc(2026, 10, 12, 16, 0));
        assert_eq!(ctx.today().weekday(), Weekday::Mon);
        assert_eq!(
            phrases::day(&["next", "monday"], &ctx),
            Some(date(2026, 10, 19))
        );
    }

    #[test]
    fn next_monday_on_sunday_is_tomorrow() {
        // Sunday 2026-10-11 21:00 NY -> Mon 01:00 UTC
        let ctx = ny(utc(2026, 10, 12, 1, 0));
        assert_eq!(
            phrases::day(&["next", "monday"], &ctx),
            Some(date(2026, 10, 12))
        );
    }

    #[test]
    fn this_weekend_before_five_pm_is_today() {
        let before = ny(utc(2026, 10, 17, 16, 0));
        assert_eq!(
            phrases::day(&["this", "weekend"], &before),
            Some(date(2026, 10, 17))
        );
    }

    #[test]
    fn this_weekend_after_five_pm_is_next() {
        let after = ny(utc(2026, 10, 17, 21, 30));
        assert_eq!(
            phrases::day(&["this", "weekend"], &after),
            Some(date(2026, 10, 24))
        );
    }

    #[test]
    fn month_rollover_31_dec_to_jan_5() {
        let ctx = ny(utc(2026, 12, 30, 17, 0));
        assert_eq!(phrases::day(&["jan", "5"], &ctx), Some(date(2027, 1, 5)));
        assert_eq!(phrases::day(&["dec", "31"], &ctx), Some(date(2026, 12, 31)));
    }

    #[test]
    fn year_rollover_feb_2028_29_days() {
        // 2028 is a leap year; Feb 29 exists.
        let ctx = ny(utc(2028, 2, 28, 16, 0));
        assert_eq!(phrases::day(&["feb", "29"], &ctx), Some(date(2028, 2, 29)));
    }

    #[test]
    fn year_rollover_feb_2027_28_days() {
        // 2027 is not leap year; Feb 28 exists, Feb 29 invalid -> None
        let ctx = ny(utc(2027, 2, 27, 16, 0));
        assert_eq!(phrases::day(&["feb", "28"], &ctx), Some(date(2027, 2, 28)));
        assert_eq!(phrases::day(&["feb", "29"], &ctx), None);
    }

    #[test]
    fn dst_berlin_2026_10_25_fall_back() {
        // Europe/Berlin: DST ends 25 Oct 2026 03:00 -> 02:00 (fall back).
        // Test that 2:30 resolves correctly to earlier instant.
        let berlin = chrono_tz::Europe::Berlin;
        let ctx = LocalContext {
            now: utc(2026, 10, 25, 2, 30),
            tz: berlin,
        };
        // 2:30 after fall-back is unambiguous (second occurrence; earlier wins per spec)
        let pinned = pinned_start("2026-10-25 at 2:30am", &ctx);
        assert!(
            pinned.is_some(),
            "DST fall-back should resolve 2:30 to earlier instant"
        );
    }

    #[test]
    fn dst_ny_2026_11_01_fall_back() {
        // America/New_York: DST ends 1 Nov 2026 02:00 -> 01:00.
        let ny_tz = chrono_tz::America::New_York;
        let ctx = LocalContext {
            now: utc(2026, 11, 1, 6, 30),
            tz: ny_tz,
        };
        let pinned = pinned_start("2026-11-01 at 1:30am", &ctx);
        assert_eq!(pinned, Some(utc(2026, 11, 1, 5, 30)));
    }
}

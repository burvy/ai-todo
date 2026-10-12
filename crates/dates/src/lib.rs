//! Text to dates, times and durations (ARCHITECTURE.md section 5, step 2).
//!
//! Code finds the phrases and resolves each one against a [`LocalContext`], so every
//! candidate already carries its value. Jev only chooses among these candidates.

mod phrases;

use std::ops::Range;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use todo_core::{LocalContext, When};

/// Longest phrase the resolvers accept, in words ("half an hour", "October 30, 2026").
const MAX_WINDOW: usize = 3;

/// 17:00 local: where a day-only deadline lands.
fn end_of_day() -> NaiveTime {
    NaiveTime::from_hms_opt(17, 0, 0).expect("17:00 is a valid time")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Day(NaiveDate),
    Time(NaiveTime),
    /// Minutes.
    Duration(u16),
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
        }
    }
    resolved
}

/// A day and a time together: the exact instant. Without both, `None`.
pub fn pinned_start(text: &str, ctx: &LocalContext) -> Option<DateTime<Utc>> {
    let resolved = resolve(text, ctx);
    Some(ctx.at(resolved.day?, resolved.time?))
}

/// A day alone: 17:00 local on that day. Without a day, `None`.
pub fn due_by(text: &str, ctx: &LocalContext) -> Option<DateTime<Utc>> {
    let day = resolve(text, ctx).day?;
    Some(ctx.at(day, end_of_day()))
}

/// The scheduling rule for a capture: day + time pins the task, a day alone makes it
/// due by 17:00 that day, and anything else floats.
pub fn when(text: &str, ctx: &LocalContext) -> When {
    let resolved = resolve(text, ctx);
    match (resolved.day, resolved.time) {
        (Some(day), Some(time)) => When::At(ctx.at(day, time)),
        (Some(day), None) => When::DueBy(ctx.at(day, end_of_day())),
        _ => When::Anytime,
    }
}

fn resolve_window(words: &[&str], ctx: &LocalContext) -> Option<Value> {
    phrases::day(words, ctx)
        .map(Value::Day)
        .or_else(|| phrases::time(words).map(Value::Time))
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

    #[test]
    fn spring_forward_pin_moves_the_gap_forward() {
        let ctx = ny(utc(2026, 3, 1, 12, 0));
        assert_eq!(
            pinned_start("2026-03-08 at 2:30", &ctx),
            Some(utc(2026, 3, 8, 7, 30))
        );
    }

    #[test]
    fn fall_back_pin_takes_the_earlier_instant() {
        let ctx = ny(utc(2026, 10, 1, 12, 0));
        assert_eq!(
            pinned_start("2026-11-01 at 1:30", &ctx),
            Some(utc(2026, 11, 1, 5, 30))
        );
    }

    #[test]
    fn when_follows_the_capture_rule() {
        let ctx = sunday_night();
        assert_eq!(
            when("tomorrow at 3", &ctx),
            When::At(utc(2026, 10, 12, 19, 0))
        );
        assert_eq!(
            when("tomorrow", &ctx),
            When::DueBy(utc(2026, 10, 12, 21, 0))
        );
        assert_eq!(when("at 3", &ctx), When::Anytime);
        assert_eq!(when("buy milk", &ctx), When::Anytime);
    }

    #[test]
    fn due_by_is_five_pm_local() {
        let ctx = sunday_night();
        assert_eq!(due_by("tomorrow", &ctx), Some(utc(2026, 10, 12, 21, 0)));
        assert_eq!(due_by("at 3", &ctx), None);
    }
}

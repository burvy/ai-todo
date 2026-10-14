//! Phrase resolvers. Each takes a window of normalized words (lowercase, no edge
//! punctuation) and returns a value only when the whole window is one phrase.

use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Weekday};
use todo_core::LocalContext;

use crate::tuning::SAME_DAY_CUTOFF;

/// The one weekday table. Every weekday rule reads from here.
pub const WEEKDAYS: [(&str, Weekday); 7] = [
    ("monday", Weekday::Mon),
    ("tuesday", Weekday::Tue),
    ("wednesday", Weekday::Wed),
    ("thursday", Weekday::Thu),
    ("friday", Weekday::Fri),
    ("saturday", Weekday::Sat),
    ("sunday", Weekday::Sun),
];

const MONTHS: [(&str, u32); 12] = [
    ("january", 1),
    ("february", 2),
    ("march", 3),
    ("april", 4),
    ("may", 5),
    ("june", 6),
    ("july", 7),
    ("august", 8),
    ("september", 9),
    ("october", 10),
    ("november", 11),
    ("december", 12),
];

fn weekday_of(word: &str) -> Option<Weekday> {
    WEEKDAYS
        .iter()
        .find(|(name, _)| *name == word)
        .map(|(_, day)| *day)
}

/// A weekday name as a recurrence phrase spells it: the full name, its plural "s", or a
/// three-letter prefix ("mondays", "tues" -> Monday), the same prefixes the month table
/// takes. [`day`] keeps the full name alone, so a bare "tue" is still not a date.
pub fn weekday(word: &str) -> Option<Weekday> {
    let name = word.strip_suffix('s').unwrap_or(word);
    let short = name.len() == 3;
    WEEKDAYS
        .iter()
        .find(|(full, _)| *full == name || (short && full.starts_with(name)))
        .map(|(_, day)| *day)
}

/// The coming `target`: today when it is that weekday and the local time is before
/// [`SAME_DAY_CUTOFF`], otherwise the next one, a week away when it is the same weekday
/// after it.
fn upcoming(target: Weekday, ctx: &LocalContext) -> NaiveDate {
    let today = ctx.today();
    let still_today = ctx.local(ctx.now).time() < SAME_DAY_CUTOFF;
    if today.weekday() == target && still_today {
        today
    } else {
        today + Duration::days(days_ahead(today.weekday(), target))
    }
}

/// Days from `from` to the next `target`, never zero: a match on the same weekday is a week away.
fn days_ahead(from: Weekday, target: Weekday) -> i64 {
    let diff = (target.num_days_from_monday() as i64 - from.num_days_from_monday() as i64 + 7) % 7;
    if diff == 0 { 7 } else { diff }
}

pub(crate) fn month_of(word: &str) -> Option<u32> {
    if word == "sept" {
        return Some(9);
    }
    // 3-letter prefixes are intentional ("sep" -> "september", "oct" -> "october").
    MONTHS
        .iter()
        .find(|(name, _)| *name == word || (word.len() == 3 && name.starts_with(word)))
        .map(|(_, month)| *month)
}

/// A date phrase: today, tomorrow, a weekday (bare, "by", "next"), next week,
/// this weekend, an ISO or US numeric date, or a month and day ("Oct 30", "October 30, 2026").
pub fn day(words: &[&str], ctx: &LocalContext) -> Option<NaiveDate> {
    let today = ctx.today();
    match words {
        ["today"] => Some(today),
        ["tomorrow"] => Some(today + Duration::days(1)),
        ["in", amount, unit] if is_day_unit(unit) || is_week_unit(unit) => {
            let count = count_of(amount)?;
            let days = if is_week_unit(unit) {
                count.checked_mul(7)?
            } else {
                count
            };
            // A year out is a typo or a joke, not a plan: leave it to the title.
            (days <= MAX_RELATIVE_DAYS).then(|| today + Duration::days(i64::from(days)))
        }
        // The same day of the month, and the last day when that month is shorter.
        ["in", amount, unit] if is_month_unit(unit) => {
            let months = count_of(amount).filter(|months| *months <= MAX_RELATIVE_MONTHS)?;
            today.checked_add_months(chrono::Months::new(months))
        }
        ["next", "week"] => {
            Some(today + Duration::days(8 - today.weekday().number_from_monday() as i64))
        }
        ["this", "weekend"] | ["weekend"] => Some(upcoming(Weekday::Sat, ctx)),
        [first, name] if matches!(*first, "by" | "next") => {
            let target = weekday_of(name)?;
            Some(today + Duration::days(days_ahead(today.weekday(), target)))
        }
        [name] if weekday_of(name).is_some() => Some(upcoming(weekday_of(name)?, ctx)),
        // chrono's `%Y` takes one to four digits, so "10/30/26" would be year 26.
        [numeric] => NaiveDate::parse_from_str(numeric, "%Y-%m-%d")
            .or_else(|_| NaiveDate::parse_from_str(numeric, "%m/%d/%Y"))
            .ok()
            .filter(|date| date.year() >= 1000),
        [month, number] => month_day(month_of(month)?, number, None, today),
        [month, number, year] => {
            let year = year.parse::<i32>().ok().filter(|y| *y >= 1000)?;
            month_day(month_of(month)?, number, Some(year), today)
        }
        _ => None,
    }
}

/// Month and day. Without a year, the next occurrence on or after today is used.
fn month_day(month: u32, number: &str, year: Option<i32>, today: NaiveDate) -> Option<NaiveDate> {
    let day = number.parse::<u32>().ok()?;
    match year {
        Some(year) => NaiveDate::from_ymd_opt(year, month, day),
        None => {
            let this_year = NaiveDate::from_ymd_opt(today.year(), month, day)?;
            if this_year < today {
                NaiveDate::from_ymd_opt(today.year() + 1, month, day)
            } else {
                Some(this_year)
            }
        }
    }
}

fn is_suffix(word: &str) -> bool {
    matches!(word, "am" | "pm")
}

/// Splits "3pm" into ("3", Some("pm")). A token without a suffix comes back whole.
fn split_suffix(token: &str) -> (&str, Option<&str>) {
    if let Some(number) = token.strip_suffix("am") {
        (number, Some("am"))
    } else if let Some(number) = token.strip_suffix("pm") {
        (number, Some("pm"))
    } else {
        (token, None)
    }
}

/// One clock time. A bare hour is read as a clock time only after "at":
/// 1-6 is afternoon, 7-11 is morning, 12 is noon. "at 3:30" follows the same rule,
/// while "15:30" or "at 3:30am" say exactly what they mean.
fn clock(number: &str, suffix: Option<&str>, after_at: bool) -> Option<NaiveTime> {
    let (hour, minute) = match number.split_once(':') {
        Some((hour, minute)) => (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?),
        None => (number.parse::<u32>().ok()?, 0),
    };
    if minute > 59 {
        return None;
    }
    let hour = match suffix {
        Some(suffix) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            hour % 12 + if suffix == "pm" { 12 } else { 0 }
        }
        None if after_at && (1..=6).contains(&hour) => hour + 12,
        None if number.contains(':') && hour <= 23 => hour,
        None if after_at && (7..=12).contains(&hour) => hour,
        None => return None,
    };
    NaiveTime::from_hms_opt(hour, minute, 0)
}

/// A time phrase: "noon", "at 3", "at 3pm", "3:30 pm", "10am", "15:45".
pub fn time(words: &[&str]) -> Option<NaiveTime> {
    match words {
        ["noon"] | ["at", "noon"] => NaiveTime::from_hms_opt(12, 0, 0),
        ["at", token] => {
            let (number, suffix) = split_suffix(token);
            clock(number, suffix, true)
        }
        ["at", number, suffix] if is_suffix(suffix) => clock(number, Some(suffix), true),
        [token] => {
            let (number, suffix) = split_suffix(token);
            clock(number, suffix, false)
        }
        [number, suffix] if is_suffix(suffix) => clock(number, Some(suffix), false),
        _ => None,
    }
}

/// A number as a digit string or spelled out, up to sixty: "7", "twelve", "twenty",
/// "forty-five". Speech to text spells small numbers out.
fn number_word(word: &str) -> Option<u32> {
    if let Ok(number) = word.parse::<u32>() {
        return Some(number);
    }
    const WORDS: [(&str, u32); 29] = [
        ("one", 1),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("six", 6),
        ("seven", 7),
        ("eight", 8),
        ("nine", 9),
        ("ten", 10),
        ("eleven", 11),
        ("twelve", 12),
        ("thirteen", 13),
        ("fourteen", 14),
        ("fifteen", 15),
        ("sixteen", 16),
        ("seventeen", 17),
        ("eighteen", 18),
        ("nineteen", 19),
        ("twenty", 20),
        ("thirty", 30),
        ("forty", 40),
        ("fifty", 50),
        ("sixty", 60),
        // Common mishearings of the tens.
        ("fourty", 40),
        ("twenny", 20),
        ("thirdy", 30),
        ("fivety", 50),
        ("sixy", 60),
    ];
    let lookup = |name: &str| {
        WORDS
            .iter()
            .find(|(spelled, _)| *spelled == name)
            .map(|(_, number)| *number)
    };
    match word.split_once('-') {
        Some((tens, ones)) => spelled_pair(lookup(tens)?, lookup(ones)?),
        None => lookup(word),
    }
}

/// "forty" and "five" together: a multiple of ten from twenty to fifty, and one to nine.
fn spelled_pair(tens: u32, ones: u32) -> Option<u32> {
    ((20..=50).contains(&tens) && tens.is_multiple_of(10) && (1..=9).contains(&ones))
        .then(|| tens + ones)
}

/// The count in front of a unit, one or two words: "10", "forty-five", "forty five".
/// Returns the count and how many words it took.
fn amount(words: &[&str]) -> Option<(u32, usize)> {
    if let [tens, ones, ..] = words
        && let Some(pair) = spelled_pair(number_word(tens)?, number_word(ones)?)
    {
        return Some((pair, 2));
    }
    Some((count_of(words.first()?)?, 1))
}

/// The most days ahead "in N days" and "in N weeks" reach.
const MAX_RELATIVE_DAYS: u32 = 366;

/// "a", "an" and "one" count one; a digit or a number word counts itself.
fn count_of(word: &str) -> Option<u32> {
    match word {
        "a" | "an" => Some(1),
        other => number_word(other),
    }
}

fn is_day_unit(word: &str) -> bool {
    matches!(word, "day" | "days")
}

/// The most months ahead "in N months" reaches.
const MAX_RELATIVE_MONTHS: u32 = 12;

fn is_month_unit(word: &str) -> bool {
    matches!(word, "month" | "months")
}

fn is_week_unit(word: &str) -> bool {
    matches!(word, "week" | "weeks")
}

fn is_minute_unit(word: &str) -> bool {
    matches!(word, "min" | "mins" | "minute" | "minutes")
}

fn is_hour_unit(word: &str) -> bool {
    matches!(word, "hour" | "hours" | "hr" | "hrs" | "h")
}

/// A duration phrase in minutes: "about an hour", "half an hour", "90 minutes", "2 hours", "45 min".
pub fn duration(words: &[&str]) -> Option<u16> {
    let minutes = match words {
        ["half", "an", "hour"] | ["half", "hour"] => 30,
        [one, unit] if matches!(*one, "an" | "a" | "one") && is_hour_unit(unit) => 60,
        [amount, unit] if is_minute_unit(unit) => number_word(amount)?,
        [amount, unit] if is_hour_unit(unit) => number_word(amount)?.checked_mul(60)?,
        [tens, ones, unit] if is_minute_unit(unit) => {
            spelled_pair(number_word(tens)?, number_word(ones)?)?
        }
        [compact] => {
            let (number, unit) = split_compact_unit(compact)?;
            if is_minute_unit(unit) {
                number.parse::<u32>().ok()?
            } else if is_hour_unit(unit) {
                number.parse::<u32>().ok()?.checked_mul(60)?
            } else {
                return None;
            }
        }
        _ => return None,
    };
    u16::try_from(minutes).ok()
}

/// How many minutes from now a relative time phrase is: "in 10 minutes", "in an hour",
/// "in half an hour", "in 90 minutes", "in 2 hours", "in 45min". Days and weeks ("in 2
/// days", "in a week") are dates, not minutes: see [`day`].
pub fn minutes_from_now(words: &[&str]) -> Option<u32> {
    let minutes = match words {
        ["in", "half", "an", "hour"] | ["in", "half", "hour"] => 30,
        ["in", compact] => {
            let (number, unit) = split_compact_unit(compact)?;
            let number = number.parse::<u32>().ok()?;
            if is_minute_unit(unit) {
                number
            } else if is_hour_unit(unit) {
                number.checked_mul(60)?
            } else {
                return None;
            }
        }
        ["in", counted @ .., unit] => {
            let (count, used) = amount(counted)?;
            if used != counted.len() {
                return None;
            }
            if is_minute_unit(unit) {
                count
            } else if is_hour_unit(unit) {
                count.checked_mul(60)?
            } else {
                return None;
            }
        }
        _ => return None,
    };
    // Past a year it is a typo, not a wait: leave it to the title.
    (minutes > 0 && minutes <= MAX_RELATIVE_DAYS * 24 * 60).then_some(minutes)
}

/// "45min" -> ("45", "min"); "2hrs" -> ("2", "hrs").
fn split_compact_unit(token: &str) -> Option<(&str, &str)> {
    let split = token.find(|c: char| c.is_ascii_alphabetic())?;
    let (number, unit) = token.split_at(split);
    (!number.is_empty()).then_some((number, unit))
}

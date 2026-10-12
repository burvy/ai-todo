use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};
use regex::Regex;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";

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

const MONTHS_SHORT: [(&str, u32); 12] = [
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("may", 5),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
];

#[allow(dead_code)]
const WEEKDAYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Judgment {
    pub is_task: f64,
    pub deadline: Option<String>,
    pub priority: u8,
    pub duration_min: u32,
    pub confidence: f64,
    pub label: Option<String>,
    pub title: Option<String>,
}

const LABELS: [&str; 6] = ["School", "Work", "Errands", "Health", "Personal", "Other"];

/// Classify a conversational message with one Jev Choice question.
pub async fn classify_intent(text: &str) -> Result<String> {
    let key = std::env::var("TYPESAFE_API_KEY").context("TYPESAFE_API_KEY is not set")?;
    let body = json!({
        "state": text,
        "model": "jev-latest",
        "questions": {"intent": {
            "type": "choice",
            "instructions": "What does this conversational message intend to do?",
            "criteria": {
                "add_tasks": "Add or create one or more tasks",
                "cancel_event": "Cancel an existing calendar event",
                "complete_task": "Mark an existing task complete",
                "query_day": "Ask what the day or calendar looks like",
                "other": "Anything else"
            }
        }}
    });
    let response: Response = reqwest::Client::new()
        .post(ENDPOINT)
        .header(AUTHORIZATION, format!("Bearer {key}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    response
        .answers
        .get("intent")
        .and_then(|answer| answer.get("choice"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .context("missing intent choice")
}

/// Let Jev select one of the supplied candidates, or none when no candidate fits.
pub async fn choose(text: &str, choices: &[String]) -> Result<Option<String>> {
    let key = std::env::var("TYPESAFE_API_KEY").context("TYPESAFE_API_KEY is not set")?;
    let mut criteria = serde_json::Map::new();
    for (index, choice) in choices.iter().enumerate() {
        criteria.insert(format!("candidate_{index}"), Value::String(choice.clone()));
    }
    criteria.insert("none".into(), Value::String("None of these".into()));
    let body = json!({
        "state": text,
        "model": "jev-latest",
        "questions": {"selection": {
            "type": "choice",
            "instructions": "Select the single best candidate, or none if no candidate matches.",
            "criteria": criteria
        }}
    });
    let response: Response = reqwest::Client::new()
        .post(ENDPOINT)
        .header(AUTHORIZATION, format!("Bearer {key}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let answer = response
        .answers
        .get("selection")
        .context("missing selection")?;
    if answer_confidence(answer) < 0.5 {
        return Ok(None);
    }
    let index = answer
        .get("choice")
        .and_then(Value::as_str)
        .and_then(|choice| choice.strip_prefix("candidate_"))
        .and_then(|index| index.parse::<usize>().ok());
    Ok(index.and_then(|index| choices.get(index).cloned()))
}

#[derive(Serialize)]
struct Request<'a> {
    state: State<'a>,
    model: &'static str,
    questions: Value,
}

#[derive(Serialize)]
struct State<'a> {
    transcript: &'a str,
    parsed_date_candidates: &'a [String],
    now: String,
}

#[derive(Deserialize)]
struct Response {
    answers: serde_json::Map<String, Value>,
}

/// Ask Jev all four questions in one request.
pub async fn understand(
    sentence: &str,
    date_candidates: &[String],
    title_candidates: &[String],
    _now: DateTime<Utc>,
    local_now: &str,
) -> Result<Judgment> {
    let key = std::env::var("TYPESAFE_API_KEY").context("TYPESAFE_API_KEY is not set")?;
    let questions = json!({
        "is_a_task": {
            "type": "noul",
            "instructions": "Does this clause contain a present/future obligation, deadline, assessment to prepare for (quiz, exam, test, presentation, interview), or concrete errand? Do not treat observations, wishes, feelings, past events, or ordering instructions as tasks.",
            "criteria": {"true": "Actionable obligation, assessment preparation, deadline, or errand", "false": "Observation, wish (would be nice), chit-chat, feeling, past event, or instruction to reorder existing work"}
        },
        "deadline_choice": {
            "type": "choice",
            "instructions": "Which parsed date candidate is the intended task deadline?",
            "criteria": deadline_criteria(date_candidates)
        },
        "priority_score": {
            "type": "score",
            "instructions": "What is the task priority from low to critical?",
            "criteria": ["P3 Low", "P2 Medium", "P1 High", "P0 Critical"]
        },
        "effort_duration_score": {
            "type": "score",
            "instructions": "Estimate the required task duration.",
            "criteria": ["Quick (<15 min)", "Short (15–45 min)", "Medium (1–2 hours)", "Half-Day (2–4 hours)", "Full-Day or Multi-Day (>4 hours)"]
        },
        "label": {
            "type": "choice",
            "instructions": "Choose the best label for this task.",
            "criteria": {"School": "School", "Work": "Work", "Errands": "Errands", "Health": "Health", "Personal": "Personal", "Other": "Other"}
        },
        "title": {
            "type": "choice",
            "instructions": "Choose the candidate span that best names the actionable task. Preserve important words from the beginning and omit filler and dates.",
            "criteria": title_criteria(title_candidates)
        }
    });
    let body = Request {
        state: State {
            transcript: sentence,
            parsed_date_candidates: date_candidates,
            now: local_now.to_owned(),
        },
        model: "jev-latest",
        questions,
    };
    let response: Response = reqwest::Client::new()
        .post(ENDPOINT)
        .header(AUTHORIZATION, format!("Bearer {key}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let task_answer = response
        .answers
        .get("is_a_task")
        .context("missing is_a_task answer")?;
    let is_task = number(task_answer, "noul")?.clamp(0.0, 1.0);
    let deadline_choice = response
        .answers
        .get("deadline_choice")
        .context("missing deadline_choice answer")?;
    let choice = deadline_choice
        .get("choice")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let deadline = if choice == "none" {
        None
    } else if let Some(index) = choice
        .strip_prefix("candidate_")
        .and_then(|value| value.parse::<usize>().ok())
    {
        date_candidates.get(index).cloned()
    } else {
        date_candidates
            .iter()
            .find(|candidate| candidate.as_str() == choice)
            .cloned()
    };
    let priority = number(
        response
            .answers
            .get("priority_score")
            .context("missing priority_score answer")?,
        "score",
    )?
    .round()
    .clamp(0.0, 3.0) as u8;
    let duration_level = number(
        response
            .answers
            .get("effort_duration_score")
            .context("missing effort_duration_score answer")?,
        "score",
    )?
    .round()
    .clamp(0.0, 4.0) as usize;
    let duration_min = [10, 30, 90, 180, 360][duration_level];
    let label = response
        .answers
        .get("label")
        .and_then(|answer| answer.get("choice"))
        .and_then(Value::as_str)
        .filter(|choice| LABELS.contains(choice))
        .map(ToOwned::to_owned);
    let title = response
        .answers
        .get("title")
        .and_then(|answer| answer.get("choice"))
        .and_then(Value::as_str)
        .and_then(|choice| choice.strip_prefix("candidate_"))
        .and_then(|index| index.parse::<usize>().ok())
        .and_then(|index| title_candidates.get(index).cloned());
    let confidence = [
        task_answer,
        deadline_choice,
        response.answers.get("priority_score").unwrap(),
        response.answers.get("effort_duration_score").unwrap(),
    ]
    .iter()
    .map(|answer| answer_confidence(answer))
    .sum::<f64>()
        / 4.0;
    Ok(Judgment {
        is_task,
        deadline,
        priority,
        duration_min,
        confidence,
        label,
        title,
    })
}

fn title_criteria(candidates: &[String]) -> Value {
    let mut criteria = serde_json::Map::new();
    for (index, candidate) in candidates.iter().enumerate() {
        criteria.insert(
            format!("candidate_{index}"),
            Value::String(candidate.clone()),
        );
    }
    criteria.insert("none".into(), Value::String("No suitable title".into()));
    Value::Object(criteria)
}

fn deadline_criteria(candidates: &[String]) -> Value {
    let mut criteria = serde_json::Map::new();
    for (index, candidate) in candidates.iter().enumerate() {
        criteria.insert(
            format!("candidate_{index}"),
            Value::String(candidate.clone()),
        );
    }
    criteria.insert("none".into(), Value::String("No deadline specified".into()));
    Value::Object(criteria)
}

fn number(answer: &Value, field: &str) -> Result<f64> {
    answer
        .get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("answer field {field} is missing or not numeric"))
}

fn answer_confidence(answer: &Value) -> f64 {
    answer
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or_else(|| {
            answer
                .get("noul")
                .and_then(Value::as_f64)
                .map(|p| (2.0 * p - 1.0).abs())
                .unwrap_or(0.0)
        })
        .clamp(0.0, 1.0)
}

pub fn split_sentences(text: &str) -> Vec<String> {
    let raw: Vec<String> = text
        .split(['.', '!', '?', '\n'])
        .flat_map(split_compound)
        .collect();
    let mut merged: Vec<String> = Vec::new();
    for clause in raw {
        if is_hedged_self_talk(&clause) {
            continue;
        }
        if is_policy_line(&clause) {
            continue;
        }
        if is_time_fragment(&clause)
            && let Some(previous) = merged.last_mut()
        {
            previous.push(' ');
            previous.push_str(clause.trim());
            continue;
        }
        if (is_pronoun_clause(&clause) || is_pronoun_object_action(&clause))
            && let Some(previous) = merged.last_mut()
        {
            previous.push(' ');
            previous.push_str(clause.trim());
            continue;
        }
        merged.push(clause);
    }
    merged
}

fn is_hedged_self_talk(clause: &str) -> bool {
    let lower = clause.trim().to_ascii_lowercase();
    [
        "i should probably ",
        "i guess ",
        "maybe i'll ",
        "maybe i will ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn is_pronoun_clause(clause: &str) -> bool {
    [
        "it ", "that ", "this ", "them ", "it,", "that,", "this,", "them,",
    ]
    .iter()
    .any(|prefix| clause.trim().to_ascii_lowercase().starts_with(prefix))
}

fn is_pronoun_object_action(clause: &str) -> bool {
    let words: Vec<&str> = clause.split_whitespace().collect();
    let words = words.strip_prefix(&["and"]).unwrap_or(&words);
    words.len() >= 2
        && [
            "upload", "send", "share", "email", "finish", "review", "read", "submit", "return",
        ]
        .contains(
            &words[0]
                .trim_matches(|c: char| !c.is_ascii_alphabetic())
                .to_ascii_lowercase()
                .as_str(),
        )
        && ["it", "that", "this", "them"].contains(
            &words[1]
                .trim_matches(|c: char| !c.is_ascii_alphabetic())
                .to_ascii_lowercase()
                .as_str(),
        )
}

fn is_time_fragment(clause: &str) -> bool {
    let lower = clause.trim().to_ascii_lowercase();
    lower.split_whitespace().count() <= 5
        && [
            "by ",
            "before ",
            "today",
            "tomorrow",
            "next ",
            "this weekend",
            "weekend",
            "just ",
            "only ",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn split_compound(sentence: &str) -> Vec<String> {
    let sentence = sentence.trim();
    let lower = sentence.to_ascii_lowercase();
    let mut candidates = Vec::new();
    for marker in [",", " oh and ", " also ", " and ", " then ", " plus "] {
        let mut offset = 0;
        while let Some(relative) = lower[offset..].find(marker) {
            let index = offset + relative;
            let left = sentence[..index].trim();
            let right = sentence[index + marker.len()..].trim();
            let right_first = right
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_ascii_lowercase();
            let bare_coordination = marker == " and "
                && !is_action_word(&right_first)
                && !["i", "you", "we", "they", "a", "an", "the"].contains(&right_first.as_str());
            if word_count_without_filler(left) >= 2
                && word_count_without_filler(right) >= 2
                && !bare_coordination
            {
                candidates.push((index, marker.len(), left.to_owned(), right.to_owned()));
            }
            offset = index + marker.len();
        }
    }
    if let Some((_, _, left, right)) = candidates.into_iter().min_by_key(|(index, _, _, _)| *index)
    {
        let mut clauses = split_compound(&left);
        clauses.extend(split_compound(&right));
        return clauses;
    }
    if sentence.is_empty() {
        Vec::new()
    } else {
        vec![sentence.to_owned()]
    }
}

fn is_action_word(word: &str) -> bool {
    [
        "buy", "call", "clean", "email", "finish", "find", "make", "read", "review", "schedule",
        "book", "pay", "send", "share", "start", "submit", "upload", "write",
    ]
    .contains(
        &word
            .trim_matches(|c: char| !c.is_ascii_alphabetic())
            .to_ascii_lowercase()
            .as_str(),
    )
}

fn word_count_without_filler(sentence: &str) -> usize {
    sentence
        .split_whitespace()
        .map(|word| word.trim_matches(|character: char| !character.is_ascii_alphanumeric()))
        .filter(|word| {
            !["ugh", "uh", "um", "like", "okay", "so"].contains(&word.to_ascii_lowercase().as_str())
        })
        .filter(|word| !word.is_empty())
        .count()
}

pub fn find_date_candidates(text: &str, now: DateTime<Utc>) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut found = Vec::new();
    for phrase in ["today", "tomorrow", "next week", "this weekend", "weekend"] {
        if lower.contains(phrase) {
            found.push(phrase.to_owned());
        }
    }
    for name in [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ] {
        if lower.contains(name) {
            let label = if lower.contains(&format!("by {name}")) {
                let current = now.weekday().number_from_monday() as i64;
                let target = match name {
                    "monday" => 1,
                    "tuesday" => 2,
                    "wednesday" => 3,
                    "thursday" => 4,
                    "friday" => 5,
                    "saturday" => 6,
                    _ => 7,
                };
                let days = (target - current + 7) % 7;
                let days = if days == 0 { 7 } else { days };
                (now.date_naive() + Duration::days(days))
                    .format("%Y-%m-%d")
                    .to_string()
            } else if lower.contains(&format!("next {name}")) {
                let current = now.with_timezone(&Local).weekday().number_from_monday() as i64;
                let target = match name {
                    "monday" => 1,
                    "tuesday" => 2,
                    "wednesday" => 3,
                    "thursday" => 4,
                    "friday" => 5,
                    "saturday" => 6,
                    _ => 7,
                };
                let mut days = (target - current + 7) % 7;
                if days == 0 {
                    days = 7;
                }
                (now.date_naive() + Duration::days(days))
                    .format("%Y-%m-%d")
                    .to_string()
            } else {
                name.to_owned()
            };
            if !found.contains(&label) {
                found.push(label);
            }
        }
    }
    for token in lower.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| !c.is_ascii_digit() && c != '-' && c != '/');
        if ((cleaned.len() == 10 && cleaned.chars().nth(4) == Some('-'))
            || cleaned.matches('/').count() == 2)
            && !found.contains(&cleaned.to_owned())
        {
            found.push(cleaned.to_owned());
        }
    }
    // Keep the parameter meaningful for callers that use relative dates.
    let _ = (now.weekday(), now + Duration::days(1));

    // Absolute date patterns
    found.extend(find_absolute_dates(text, now));

    found
}

fn find_absolute_dates(text: &str, now: DateTime<Utc>) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut found = Vec::new();
    let now_local = now.with_timezone(&Local);
    let current_year = now_local.year();
    let current_month = now_local.month();
    let current_day = now_local.day();

    // Pattern 1: "December 11, 2026" or "Dec 11, 2026" or "December 11" or "Dec 11"
    let full_month_re =
        Regex::new(r"\b(january|february|march|april|may|june|july|august|september|october|november|december)\s+(\d{1,2}),?\s*(\d{4})?\b").unwrap();
    for cap in full_month_re.captures_iter(&lower) {
        let month_str = &cap[1];
        let day: u32 = cap[2].parse().unwrap_or(1);
        let year = cap
            .get(3)
            .and_then(|m| m.as_str().parse::<i32>().ok())
            .unwrap_or_else(|| {
                let month_num = MONTHS
                    .iter()
                    .find(|(n, _)| *n == month_str)
                    .map(|(_, m)| *m)
                    .unwrap_or(1);
                if month_num < current_month || (month_num == current_month && day < current_day) {
                    current_year + 1
                } else {
                    current_year
                }
            });
        if let Some(date) = NaiveDate::from_ymd_opt(
            year,
            MONTHS
                .iter()
                .find(|(n, _)| *n == month_str)
                .map(|(_, m)| *m)
                .unwrap_or(1),
            day,
        ) {
            let dt = date
                .and_hms_opt(17, 0, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap();
            found.push(dt.with_timezone(&Utc).to_rfc3339());
        }
    }

    // Pattern 2: "Oct 30" or "Oct 30, 2026" (short month)
    let short_month_re = Regex::new(
        r"\b(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\s+(\d{1,2}),?\s*(\d{4})?\b",
    )
    .unwrap();
    for cap in short_month_re.captures_iter(&lower) {
        let month_str = &cap[1];
        let day: u32 = cap[2].parse().unwrap_or(1);
        let year = cap
            .get(3)
            .and_then(|m| m.as_str().parse::<i32>().ok())
            .unwrap_or_else(|| {
                let month_num = MONTHS_SHORT
                    .iter()
                    .find(|(n, _)| *n == month_str)
                    .map(|(_, m)| *m)
                    .unwrap_or(1);
                if month_num < current_month || (month_num == current_month && day < current_day) {
                    current_year + 1
                } else {
                    current_year
                }
            });
        if let Some(date) = NaiveDate::from_ymd_opt(
            year,
            MONTHS_SHORT
                .iter()
                .find(|(n, _)| *n == month_str)
                .map(|(_, m)| *m)
                .unwrap_or(1),
            day,
        ) {
            let dt = date
                .and_hms_opt(17, 0, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap();
            found.push(dt.with_timezone(&Utc).to_rfc3339());
        }
    }

    // Pattern 3: "10/30" or "10/30/2026"
    let slash_re = Regex::new(r"\b(\d{1,2})/(\d{1,2})(?:/(\d{2,4}))?\b").unwrap();
    for cap in slash_re.captures_iter(&lower) {
        let month: u32 = cap[1].parse().unwrap_or(1);
        let day: u32 = cap[2].parse().unwrap_or(1);
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            continue;
        }
        let year = cap
            .get(3)
            .and_then(|m| m.as_str().parse::<i32>().ok())
            .unwrap_or_else(|| {
                if month < current_month || (month == current_month && day < current_day) {
                    current_year + 1
                } else {
                    current_year
                }
            });
        let year = if year < 100 { year + 2000 } else { year };
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            let dt = date
                .and_hms_opt(17, 0, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap();
            found.push(dt.with_timezone(&Utc).to_rfc3339());
        }
    }

    // Pattern 4: "Friday, October 30" or "Friday, October 30, 2026"
    let weekday_month_re = Regex::new(r"\b(monday|tuesday|wednesday|thursday|friday|saturday|sunday),\s+(january|february|march|april|may|june|july|august|september|october|november|december)\s+(\d{1,2}),?\s*(\d{4})?\b").unwrap();
    for cap in weekday_month_re.captures_iter(&lower) {
        let _weekday = &cap[1];
        let month_str = &cap[2];
        let day: u32 = cap[3].parse().unwrap_or(1);
        let year = cap
            .get(4)
            .and_then(|m| m.as_str().parse::<i32>().ok())
            .unwrap_or_else(|| {
                let month_num = MONTHS
                    .iter()
                    .find(|(n, _)| *n == month_str)
                    .map(|(_, m)| *m)
                    .unwrap_or(1);
                if month_num < current_month || (month_num == current_month && day < current_day) {
                    current_year + 1
                } else {
                    current_year
                }
            });
        if let Some(date) = NaiveDate::from_ymd_opt(
            year,
            MONTHS
                .iter()
                .find(|(n, _)| *n == month_str)
                .map(|(_, m)| *m)
                .unwrap_or(1),
            day,
        ) {
            let dt = date
                .and_hms_opt(17, 0, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap();
            found.push(dt.with_timezone(&Utc).to_rfc3339());
        }
    }

    // Deduplicate
    found.sort();
    found.dedup();
    found
}

/// Check if a clause is a policy line (not a task).
/// Policy lines contain: late work, grading, attendance, "unless", percent penalties.
pub fn is_policy_line(clause: &str) -> bool {
    let lower = clause.to_lowercase();
    let policy_keywords = [
        "late work",
        "grading",
        "attendance",
        "unless",
        "percent",
        "%",
        "penalty",
        "penalties",
        "late penalty",
        "extension",
        "arranged in advance",
        "drops",
        "drop policy",
        "late loses",
        "loses percent",
    ];
    policy_keywords.iter().any(|kw| lower.contains(kw))
}

/// Extract title and deadline from patterns like "Final synthesis essay - December 11, 2026"
/// Returns (title, deadline_string) if found, otherwise (original_clause, None)
pub fn extract_title_and_deadline(clause: &str) -> (String, Option<String>) {
    // Pattern: "Title - Date" or "Title – Date" (em dash)
    let dash_re = Regex::new(r"^(.+?)\s*[-–—]\s*(.+)$").unwrap();
    if let Some(cap) = dash_re.captures(clause) {
        let title = cap[1].trim().to_string();
        let date_part = cap[2].trim().to_string();
        // Try to parse the date part as an absolute date
        let now = Utc::now();
        let dates = find_absolute_dates(&date_part, now);
        if !dates.is_empty() {
            return (title, Some(dates[0].clone()));
        }
    }
    (clause.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn splits_punctuation_and_newlines() {
        assert_eq!(
            split_sentences("One. Two!\nThree?"),
            vec!["One", "Two", "Three"]
        );
        assert_eq!(
            split_sentences("finish X and email Y"),
            vec!["finish X", "email Y"]
        );
        let clauses = split_sentences(
            "Ugh okay so I have a calc quiz Thursday, um, also mom called, and I gotta pick up my prescription today",
        );
        assert!(clauses.len() >= 2);
        assert!(
            clauses
                .iter()
                .all(|clause| clause.split_whitespace().count() <= 20)
        );
    }

    #[test]
    fn demo_rambles_split_into_short_clauses() {
        let rambles = include_str!("../../../demo/rambles.md");
        for paragraph in rambles.split("## ").skip(1) {
            let clauses = split_sentences(paragraph);
            assert!(clauses.len() >= 2);
            assert!(
                clauses
                    .iter()
                    .all(|clause| clause.split_whitespace().count() <= 20)
            );
        }
    }

    #[test]
    fn merges_pronoun_upload_and_drops_hedged_self_talk() {
        let clauses = split_sentences(
            "I have to finish the history response paper, it is two pages, and upload it by Friday. I should probably start the response before I panic.",
        );
        assert_eq!(clauses.len(), 1, "{clauses:?}");
        assert!(clauses[0].contains("finish the history response paper"));
        assert!(clauses[0].contains("upload it by Friday"));
        assert!(!clauses.iter().any(|clause| clause.contains("panic")));
    }

    #[test]
    fn keeps_coordinated_names_in_one_clause() {
        let clauses = split_sentences("I should schedule a meeting with Maya and Luis");
        assert_eq!(
            clauses,
            vec!["I should schedule a meeting with Maya and Luis"]
        );
    }

    #[test]
    fn keeps_bare_number_coordination() {
        assert_eq!(
            split_sentences("Review chapters six and seven"),
            vec!["Review chapters six and seven"]
        );
        assert!(
            split_sentences("Please make a task to review chapters six and seven, maybe an hour.")
                .iter()
                .any(|clause| clause.contains("chapters six and seven"))
        );
        assert!(
            split_sentences(
                "I should schedule a meeting with Maya and Luis, maybe Friday afternoon."
            )
            .iter()
            .any(|clause| clause.contains("Maya and Luis"))
        );
        assert_eq!(
            split_sentences("book a dentist appointment and pay rent by Friday"),
            vec!["book a dentist appointment", "pay rent by Friday"]
        );
    }

    #[test]
    fn finds_relative_weekday_and_numeric_dates() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        let candidates =
            find_date_candidates("Finish today, by Friday, next week, or 2026-10-31", now);
        assert!(candidates.contains(&"today".into()));
        assert!(candidates.contains(&"2026-10-16".into()));
        assert!(candidates.contains(&"next week".into()));
        assert!(candidates.contains(&"2026-10-31".into()));
    }

    #[test]
    fn finds_absolute_dates_full_month() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        let candidates = find_date_candidates("Final synthesis essay - December 11, 2026", now);
        assert!(candidates.iter().any(|c| c.contains("2026-12-11")));
    }

    #[test]
    fn finds_absolute_dates_short_month() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        let candidates = find_date_candidates("Quiz on Dec 11", now);
        assert!(candidates.iter().any(|c| c.contains("2026-12-11")));
        let candidates2 = find_date_candidates("Oct 30 deadline", now);
        assert!(candidates2.iter().any(|c| c.contains("2026-10-30")));
    }

    #[test]
    fn finds_absolute_dates_slash_format() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        let candidates = find_date_candidates("Due 10/30", now);
        assert!(candidates.iter().any(|c| c.contains("2026-10-30")));
        let candidates2 = find_date_candidates("Due 10/30/2026", now);
        assert!(candidates2.iter().any(|c| c.contains("2026-10-30")));
    }

    #[test]
    fn finds_absolute_dates_weekday_month() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        let candidates = find_date_candidates("Friday, October 30", now);
        assert!(candidates.iter().any(|c| c.contains("2026-10-30")));
    }

    #[test]
    fn filters_policy_lines() {
        let clauses = split_sentences(
            "Late work loses 10 percent per day unless an extension is arranged in advance. Finish the essay by Friday.",
        );
        assert_eq!(clauses.len(), 1);
        assert!(clauses[0].contains("Finish the essay"));
    }

    #[test]
    fn filters_policy_lines_grading_attendance() {
        let clauses = split_sentences(
            "Grading is based on participation. Attendance is mandatory. Submit homework by Monday.",
        );
        assert_eq!(clauses.len(), 1);
        assert!(clauses[0].contains("Submit homework"));
    }

    #[test]
    fn extracts_title_and_deadline() {
        let _now = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        let (title, deadline) =
            extract_title_and_deadline("Final synthesis essay - December 11, 2026");
        assert_eq!(title, "Final synthesis essay");
        assert!(deadline.is_some());
        assert!(deadline.unwrap().contains("2026-12-11"));
    }

    #[test]
    fn extracts_title_and_deadline_em_dash() {
        let _now = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        let (title, deadline) =
            extract_title_and_deadline("Final synthesis essay – December 11, 2026");
        assert_eq!(title, "Final synthesis essay");
        assert!(deadline.is_some());
        assert!(deadline.unwrap().contains("2026-12-11"));
    }

    #[test]
    fn ignores_text_without_dates() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        assert!(find_date_candidates("Review the notes", now).is_empty());
    }

    #[test]
    fn processes_syllabus_text() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        let syllabus_text = r#"INTRODUCTION TO URBAN ECOLOGY - FALL 2026
Instructor: Dr. Lena Ortiz    Course: BIO 214
Meeting: Tuesday and Thursday, 10:00-11:15
This course studies how plants, animals, and people shape city ecosystems.
Read the assigned chapter before each discussion and bring one observation
from your neighborhood.
ASSIGNMENTS AND DUE DATES
1. Neighborhood observation journal - September 18, 2026
Three field observations with photographs and a 500-word reflection.
2. Species profile - October 2, 2026
Research one urban species and cite four reliable sources in two pages.
3. Midterm data memo - October 23, 2026
Analyze the class temperature dataset; submit methods and results memo.
4. Team habitat proposal - November 13, 2026
Propose a habitat improvement with a map, budget, and presentation.
5. Final synthesis essay - December 11, 2026
Submit a 1,500-word essay connecting two themes to course evidence.
Late work loses 10 percent per day unless an extension is arranged in advance."#;
        let sentences = split_sentences(syllabus_text);
        println!("Sentences: {:?}", sentences);
        for (i, s) in sentences.iter().enumerate() {
            println!("{}: {}", i, s);
            let candidates = find_date_candidates(s, now);
            if !candidates.is_empty() {
                println!("  DATE CANDIDATES: {:?}", candidates);
            }
            let (title, deadline) = extract_title_and_deadline(s);
            if deadline.is_some() {
                println!("  EXTRACTED: title='{}', deadline={:?}", title, deadline);
            }
        }
        // The policy line should be filtered out
        assert!(!sentences.iter().any(|s| s.contains("Late work loses")));
        // Each assignment should be a clause with a date
        assert!(sentences.iter().any(|s| s.contains("September 18")));
        assert!(sentences.iter().any(|s| s.contains("October 2")));
        assert!(sentences.iter().any(|s| s.contains("October 23")));
        assert!(sentences.iter().any(|s| s.contains("November 13")));
        assert!(sentences.iter().any(|s| s.contains("December 11")));
    }

    #[tokio::test]
    #[ignore = "requires TYPESAFE_API_KEY and network access"]
    async fn live_jev_request() {
        let _ = understand(
            "File taxes by Friday",
            &[],
            &[],
            Utc::now(),
            "2026-10-11T20:00:00-04:00",
        )
        .await
        .unwrap();
    }
}

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Datelike, Duration, Utc};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";

#[derive(Clone, Debug, PartialEq)]
pub struct Judgment {
    pub is_task: f64,
    pub deadline: Option<String>,
    pub priority: u8,
    pub duration_min: u32,
    pub confidence: f64,
}

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
    now: DateTime<Utc>,
) -> Result<Judgment> {
    let key = std::env::var("TYPESAFE_API_KEY").context("TYPESAFE_API_KEY is not set")?;
    let questions = json!({
        "is_a_task": {
            "type": "noul",
            "instructions": "Does this clause contain an obligation, deadline, assessment to prepare for (quiz, exam, test, presentation, interview), or concrete errand? Count implied preparation tasks as actionable.",
            "criteria": {"true": "Actionable obligation, assessment preparation, deadline, or errand", "false": "Chit-chat, feelings, background context, or a past event with no action"}
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
        }
    });
    let body = Request {
        state: State {
            transcript: sentence,
            parsed_date_candidates: date_candidates,
            now: now.to_rfc3339(),
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
    })
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
    text.split(['.', '!', '?', '\n'])
        .flat_map(split_compound)
        .collect()
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
            if word_count_without_filler(left) >= 2 && word_count_without_filler(right) >= 2 {
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
    for phrase in ["today", "tomorrow", "next week"] {
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
    found
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
    fn ignores_text_without_dates() {
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        assert!(find_date_candidates("Review the notes", now).is_empty());
    }

    #[tokio::test]
    #[ignore = "requires TYPESAFE_API_KEY and network access"]
    async fn live_jev_request() {
        let _ = understand("File taxes by Friday", &[], Utc::now())
            .await
            .unwrap();
    }
}

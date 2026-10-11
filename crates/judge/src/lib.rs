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
    pub label: Option<String>,
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
    now: DateTime<Utc>,
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
    let label = response
        .answers
        .get("label")
        .and_then(|answer| answer.get("choice"))
        .and_then(Value::as_str)
        .filter(|choice| LABELS.contains(choice))
        .map(ToOwned::to_owned);
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
    let raw: Vec<String> = text
        .split(['.', '!', '?', '\n'])
        .flat_map(split_compound)
        .collect();
    let mut merged: Vec<String> = Vec::new();
    for clause in raw {
        if is_hedged_self_talk(&clause) {
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

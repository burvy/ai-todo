//! The Jev HTTP client for v2. One request per clause, all sent at once. Each request
//! asks only the questions that have candidates to choose from.

use anyhow::{Context, Result, anyhow};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use todo_core::Label;

use crate::{ClauseAnswer, ClauseQuestion, Judge, LABEL_NAMES, PRIORITY_LEVELS};

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const MODEL: &str = "jev-latest";

#[derive(Clone, Debug)]
pub struct JevJudge {
    key: String,
    client: reqwest::Client,
}

impl JevJudge {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Reads `TYPESAFE_API_KEY`. The key is never logged or stored anywhere else.
    pub fn from_env() -> Result<Self> {
        let key = std::env::var("TYPESAFE_API_KEY").context("TYPESAFE_API_KEY is not set")?;
        Ok(Self::new(key))
    }

    async fn judge_one(&self, question: &ClauseQuestion) -> Result<ClauseAnswer> {
        let body = json!({
            "state": {"transcript": question.clause, "now": question.local_now},
            "model": MODEL,
            "questions": questions(question),
        });
        let response: Response = self
            .client
            .post(ENDPOINT)
            .header(AUTHORIZATION, format!("Bearer {}", self.key))
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        parse_answer(&response.answers)
    }
}

impl Judge for JevJudge {
    async fn judge(&self, questions: &[ClauseQuestion]) -> Result<Vec<ClauseAnswer>> {
        futures::future::try_join_all(questions.iter().map(|question| self.judge_one(question)))
            .await
    }
}

#[derive(Deserialize)]
struct Response {
    answers: Map<String, Value>,
}

const TITLE_INSTRUCTIONS: &str = "Which span is the task itself, as a short title? Words that only ask the app to create the task, such as \"add a task to\", \"remind me to\", \"can you\" or \"please\", are not part of the title. If the clause is already the task, choose the whole clause.";

const IS_TASK_INSTRUCTIONS: &str = "Does this clause name an actionable obligation, deadline, assessment to prepare for, or errand? Observations, wishes, feelings, past events and chit-chat are not tasks.";

/// The questions to ask for one clause. Choice questions with no candidates are left out.
fn questions(question: &ClauseQuestion) -> Value {
    let mut questions = Map::new();
    questions.insert(
        "is_a_task".into(),
        json!({
            "type": "noul",
            "instructions": IS_TASK_INSTRUCTIONS,
            "criteria": {"true": "Actionable task", "false": "Not a task"}
        }),
    );
    questions.insert(
        "title".into(),
        choice(
            TITLE_INSTRUCTIONS,
            indexed(&question.titles, "title_", None),
        ),
    );
    if !question.days.is_empty() {
        questions.insert(
            "day".into(),
            choice(
                "Which date phrase is the task's day? Choose none if the clause names no day.",
                indexed(&question.days, "candidate_", Some("No day")),
            ),
        );
    }
    if !question.times.is_empty() {
        questions.insert(
            "time".into(),
            choice(
                "Which time phrase is the task's start time? Choose none if the clause names no time.",
                indexed(&question.times, "candidate_", Some("No time")),
            ),
        );
    }
    if !question.durations.is_empty() {
        questions.insert(
            "duration".into(),
            choice(
                "Which duration phrase is the task's length? Choose none if the clause gives no length.",
                indexed(&question.durations, "candidate_", Some("No length")),
            ),
        );
    }
    let labels: Map<String, Value> = LABEL_NAMES
        .iter()
        .map(|(name, _)| (name.to_string(), Value::String(name.to_string())))
        .collect();
    questions.insert(
        "label".into(),
        choice("Choose the best label for this task.", labels),
    );
    questions.insert(
        "priority".into(),
        json!({
            "type": "score",
            "instructions": "What is the task priority from low to critical?",
            "criteria": ["P3 Low", "P2 Medium", "P1 High", "P0 Critical"]
        }),
    );
    Value::Object(questions)
}

fn choice(instructions: &str, criteria: Map<String, Value>) -> Value {
    json!({"type": "choice", "instructions": instructions, "criteria": criteria})
}

/// `candidate_0`, `candidate_1`, ... for each item, plus an optional "none" entry.
fn indexed(items: &[String], prefix: &str, none: Option<&str>) -> Map<String, Value> {
    let mut criteria: Map<String, Value> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (format!("{prefix}{index}"), Value::String(item.clone())))
        .collect();
    if let Some(none) = none {
        criteria.insert("none".into(), Value::String(none.into()));
    }
    criteria
}

fn parse_answer(answers: &Map<String, Value>) -> Result<ClauseAnswer> {
    let field = |name: &str| {
        answers
            .get(name)
            .ok_or_else(|| anyhow!("Jev answer missing {name}"))
    };
    let task = field("is_a_task")?;
    let is_task = task
        .get("noul")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let mut confidences = vec![(2.0 * is_task - 1.0).abs()];

    let title = choice_index(field("title")?, "title_", &mut confidences);
    let day = optional(answers, "day", "candidate_", &mut confidences);
    let time = optional(answers, "time", "candidate_", &mut confidences);
    let duration = optional(answers, "duration", "candidate_", &mut confidences);

    let label_answer = field("label")?;
    let label = label_answer
        .get("choice")
        .and_then(Value::as_str)
        .and_then(label_named)
        .unwrap_or(Label::Other);
    confidences.push(answer_confidence(label_answer));

    let priority_answer = field("priority")?;
    // The score indexes the criteria the question lists, one per priority level.
    let highest = (PRIORITY_LEVELS.len() - 1) as f64;
    let level = priority_answer
        .get("score")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .round()
        .clamp(0.0, highest) as usize;
    confidences.push(answer_confidence(priority_answer));

    Ok(ClauseAnswer {
        is_task,
        title,
        day,
        time,
        duration,
        label,
        priority: PRIORITY_LEVELS[level],
        confidence: confidences.iter().sum::<f64>() / confidences.len() as f64,
    })
}

fn optional(
    answers: &Map<String, Value>,
    name: &str,
    prefix: &str,
    confidences: &mut Vec<f64>,
) -> Option<usize> {
    let answer = answers.get(name)?;
    choice_index(answer, prefix, confidences)
}

/// The index Jev chose, or `None` for "none" or an unknown choice.
fn choice_index(answer: &Value, prefix: &str, confidences: &mut Vec<f64>) -> Option<usize> {
    confidences.push(answer_confidence(answer));
    answer
        .get("choice")
        .and_then(Value::as_str)
        .and_then(|choice| choice.strip_prefix(prefix))
        .and_then(|index| index.parse().ok())
}

fn label_named(name: &str) -> Option<Label> {
    LABEL_NAMES
        .iter()
        .find(|(label, _)| *label == name)
        .map(|(_, label)| *label)
}

fn answer_confidence(answer: &Value) -> f64 {
    answer
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_core::Priority;

    fn clause() -> ClauseQuestion {
        ClauseQuestion {
            clause: "call mom tomorrow at 3".into(),
            titles: vec!["Call mom".into()],
            days: vec!["tomorrow".into()],
            times: vec!["at 3".into()],
            durations: vec![],
            local_now: "Sunday 2026-10-11 21:00".into(),
        }
    }

    #[test]
    fn asks_only_the_questions_with_candidates() {
        let asked = questions(&clause());
        assert!(asked.get("day").is_some());
        assert!(asked.get("time").is_some());
        assert!(asked.get("duration").is_none());
        assert_eq!(asked["title"]["criteria"]["title_0"], "Call mom");
        assert!(asked["title"]["criteria"].get("none").is_none());
    }

    #[test]
    fn parses_choices_labels_and_priority() {
        let answers: Map<String, Value> = serde_json::from_value(json!({
            "is_a_task": {"noul": 0.9, "confidence": 0.9},
            "title": {"choice": "title_0", "confidence": 0.8},
            "day": {"choice": "candidate_0", "confidence": 0.9},
            "time": {"choice": "none", "confidence": 0.7},
            "label": {"choice": "Personal", "confidence": 0.8},
            "priority": {"score": 2.2, "confidence": 0.6}
        }))
        .unwrap();
        let answer = parse_answer(&answers).unwrap();
        assert_eq!(answer.title, Some(0));
        assert_eq!(answer.day, Some(0));
        assert_eq!(answer.time, None);
        assert_eq!(answer.duration, None);
        assert_eq!(answer.label, Label::Personal);
        assert_eq!(answer.priority, Priority::High);
        assert!(answer.is_task > 0.8);
    }

    #[test]
    fn a_missing_answer_is_an_error() {
        let answers: Map<String, Value> = Map::new();
        assert!(parse_answer(&answers).is_err());
    }

    /// Live check. Run by hand with `TYPESAFE_API_KEY` set:
    /// `cargo test -p judge -- --ignored`
    #[tokio::test]
    #[ignore = "calls the live Jev API"]
    async fn live_judge_answers_a_clause() {
        let judge = JevJudge::from_env().unwrap();
        let answers = judge.judge(&[clause()]).await.unwrap();
        assert_eq!(answers.len(), 1);
        assert!(answers[0].is_task > 0.5);
    }
}

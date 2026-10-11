//! `POST /api/tasks/:id/revise`: Jev re-reads a task together with the user's
//! instruction (e.g. "make it Thursday at 2, about an hour") and changes only
//! the fields that the instruction actually mentions.

use axum::{
    Json,
    extract::{Path as UrlPath, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use judge::{Judgment, find_date_candidates, understand};
use serde::Deserialize;
use todo_core::Task;

use crate::{AppState, clamp_duration, parse_deadline};

#[derive(Deserialize)]
pub struct ReviseIn {
    text: String,
}

type ApiError = (StatusCode, Json<serde_json::Value>);

fn error(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(serde_json::json!({"error": message.into()})))
}

/// `POST /api/tasks/:id/revise` with `{text}`. Returns the updated task.
pub async fn revise_task(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<ReviseIn>,
) -> Result<Json<Task>, ApiError> {
    let instruction = body.text.trim().to_string();
    if instruction.is_empty() {
        return Err(error(StatusCode::BAD_REQUEST, "text is required"));
    }
    let current = {
        let store = state.store.lock().unwrap();
        store
            .tasks
            .iter()
            .find(|task| task.id == id)
            .cloned()
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "task not found"))?
    };
    if std::env::var("TYPESAFE_API_KEY").is_err() {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Jev is not configured (TYPESAFE_API_KEY is not set)",
        ));
    }

    let now = Utc::now();
    let candidates = find_date_candidates(&instruction, now);
    let state_text = format!("{}. {}", current.title, instruction);
    let judgment = understand(&state_text, &candidates, now)
        .await
        .map_err(|e| error(StatusCode::BAD_GATEWAY, format!("Jev revise failed: {e}")))?;

    let mut updated = current;
    apply_revision(&mut updated, &judgment, &instruction, &candidates, now);

    {
        let mut store = state.store.lock().unwrap();
        if let Some(slot) = store.tasks.iter_mut().find(|task| task.id == id) {
            *slot = updated.clone();
        }
    }
    state.db.save_task(&updated);
    Ok(Json(updated))
}

/// Apply Jev's judgment to a task, but only for the fields the instruction mentions.
///
/// - Deadline: only when the instruction holds a date candidate.
/// - Duration, priority and label: only when the instruction uses a word for them.
/// - Confidence always takes Jev's value for this revision.
///
/// TODO: an explicit day + time should set `pinned_start` once Jim's ATD-28
/// adds that field to `Task`. Until then only the deadline is set.
pub fn apply_revision(
    task: &mut Task,
    judgment: &Judgment,
    instruction: &str,
    candidates: &[String],
    now: DateTime<Utc>,
) {
    if !candidates.is_empty()
        && let Some(deadline) = judgment
            .deadline
            .as_deref()
            .and_then(|candidate| parse_deadline(candidate, now))
    {
        task.deadline = Some(deadline);
    }
    if mentions(instruction, DURATION_WORDS) {
        task.duration_min = clamp_duration(i64::from(judgment.duration_min));
    }
    if mentions(instruction, PRIORITY_WORDS) {
        task.priority = judgment.priority;
    }
    if mentions(instruction, LABEL_WORDS)
        && let Some(label) = &judgment.label
    {
        task.label = label.clone();
    }
    task.confidence = judgment.confidence;
}

const DURATION_WORDS: &[&str] = &[
    "hour", "hours", "hr", "hrs", "minute", "minutes", "min", "mins", "quick", "short", "long",
    "half",
];
const PRIORITY_WORDS: &[&str] = &[
    "priority",
    "urgent",
    "asap",
    "important",
    "critical",
    "low",
    "medium",
    "high",
    "p0",
    "p1",
    "p2",
    "p3",
];
const LABEL_WORDS: &[&str] = &[
    "work", "job", "school", "class", "errand", "errands", "health", "doctor", "gym", "personal",
];

/// True when one of `words` appears as a whole word in the instruction.
fn mentions(instruction: &str, words: &[&str]) -> bool {
    instruction
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| words.contains(&word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use todo_core::Source;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 11, 15, 0, 0).unwrap()
    }

    fn task() -> Task {
        Task {
            id: "t1".into(),
            title: "Write report".into(),
            notes: String::new(),
            duration_min: 30,
            deadline: None,
            priority: 2,
            status: "todo".into(),
            confidence: 0.2,
            label: "Other".into(),
            source: Source::Text,
        }
    }

    fn judgment() -> Judgment {
        Judgment {
            is_task: 1.0,
            deadline: Some("tomorrow".into()),
            priority: 0,
            duration_min: 90,
            confidence: 0.8,
            label: Some("Work".into()),
        }
    }

    #[test]
    fn deadline_is_ignored_without_date_candidates() {
        let mut t = task();
        apply_revision(&mut t, &judgment(), "about an hour", &[], now());
        assert_eq!(t.deadline, None);
    }

    #[test]
    fn deadline_is_set_when_instruction_has_a_date() {
        let mut t = task();
        let candidates = vec!["tomorrow".to_string()];
        apply_revision(&mut t, &judgment(), "make it tomorrow", &candidates, now());
        assert!(t.deadline.is_some());
    }

    #[test]
    fn duration_changes_only_when_instruction_mentions_time() {
        let mut t = task();
        apply_revision(&mut t, &judgment(), "about an hour", &[], now());
        assert_eq!(t.duration_min, 90);

        let mut t = task();
        apply_revision(&mut t, &judgment(), "low priority", &[], now());
        assert_eq!(t.duration_min, 30);
    }

    #[test]
    fn work_low_priority_sets_label_and_priority_only() {
        let mut t = task();
        apply_revision(
            &mut t,
            &judgment(),
            "this is for work, low priority",
            &[],
            now(),
        );
        assert_eq!(t.label, "Work");
        assert_eq!(t.priority, 0);
        assert_eq!(t.duration_min, 30);
        assert_eq!(t.deadline, None);
    }

    #[test]
    fn unmentioned_fields_stay_put_and_confidence_updates() {
        let mut t = task();
        apply_revision(&mut t, &judgment(), "hmm", &[], now());
        assert_eq!(t.priority, 2);
        assert_eq!(t.label, "Other");
        assert_eq!(t.duration_min, 30);
        assert_eq!(t.confidence, 0.8);
    }

    #[test]
    fn mention_matches_whole_words_only() {
        assert!(!mentions("allowed", PRIORITY_WORDS));
        assert!(mentions("it is LOW priority", PRIORITY_WORDS));
    }
}

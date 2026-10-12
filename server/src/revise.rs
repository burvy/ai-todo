//! `POST /api/tasks/:id/revise`: Jev re-reads a task together with the user's
//! instruction (e.g. "make it Thursday at 2, about an hour") and changes only
//! the fields that the instruction actually mentions.

use axum::{
    Json,
    extract::{Path as UrlPath, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
#[cfg(test)]
use judge::find_date_candidates;
use judge::{Judgment, understand};
use serde::Deserialize;
use todo_core::v1::Task;

use crate::{
    AppState, clamp_duration, date_candidates_local, local_time_as_utc, parse_deadline,
    pinned_start_for, timezone,
};
use chrono::NaiveTime;

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
    let candidates = date_candidates_local(&instruction, now);
    let state_text = format!("{}. {}", current.title, instruction);
    let title_candidates = vec![crate::clean_title(&instruction)];
    let local_now = now.with_timezone(&timezone()).to_rfc3339();
    let judgment = understand(&state_text, &candidates, &title_candidates, now, &local_now)
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
    crate::reschedule(&state);
    Ok(Json(updated))
}

/// Apply Jev's judgment to a task, but only for the fields the instruction mentions.
///
/// - Pinned time: a day + time ("Thursday at 2") sets `pinned_start` to that time and
///   the deadline to that day. "anytime" or "no fixed time" clears `pinned_start`.
/// - Deadline: only when the instruction holds a date candidate.
/// - Duration, priority and label: only when the instruction uses a word for them.
/// - Confidence always takes Jev's value for this revision.
pub fn apply_revision(
    task: &mut Task,
    judgment: &Judgment,
    instruction: &str,
    candidates: &[String],
    now: DateTime<Utc>,
) {
    if clears_pin(instruction) {
        task.pinned_start = None;
    }
    if !candidates.is_empty()
        && let Some(deadline) = judgment
            .deadline
            .as_deref()
            .and_then(|candidate| parse_deadline(candidate, now))
    {
        task.deadline = Some(deadline);
    }
    if let Some(start) = pinned_start_for(instruction, now) {
        let day = start.with_timezone(&timezone()).date_naive();
        task.pinned_start = Some(start);
        task.deadline = Some(local_time_as_utc(
            timezone(),
            day.and_time(NaiveTime::from_hms_opt(17, 0, 0).unwrap()),
        ));
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

/// True when the instruction says the task has no fixed time any more.
fn clears_pin(instruction: &str) -> bool {
    instruction.to_ascii_lowercase().contains("no fixed time")
        || mentions(instruction, &["anytime"])
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
    use chrono::{TimeZone, Timelike};
    use todo_core::v1::Source;

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
            position: 0,
            pinned_start: None,
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
            title: None,
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

    #[test]
    fn day_and_time_pin_the_task_and_move_its_deadline() {
        let mut t = task();
        t.pinned_start = Some(Utc.with_ymd_and_hms(2026, 10, 16, 19, 0, 0).unwrap());
        let candidates = find_date_candidates("actually make it Thursday at 2", now());
        apply_revision(
            &mut t,
            &judgment(),
            "actually make it Thursday at 2",
            &candidates,
            now(),
        );
        let start = t.pinned_start.expect("pinned");
        let local = start.with_timezone(&timezone());
        assert_eq!(
            local.date_naive(),
            chrono::NaiveDate::from_ymd_opt(2026, 10, 15).unwrap()
        );
        assert_eq!(local.hour(), 14);
        let deadline = t.deadline.expect("deadline").with_timezone(&timezone());
        assert_eq!(
            deadline.date_naive(),
            chrono::NaiveDate::from_ymd_opt(2026, 10, 15).unwrap()
        );
        assert_eq!(deadline.hour(), 17);
    }

    #[test]
    fn time_without_a_day_does_not_pin() {
        let mut t = task();
        let before = Utc.with_ymd_and_hms(2026, 10, 16, 19, 0, 0).unwrap();
        t.pinned_start = Some(before);
        apply_revision(&mut t, &judgment(), "at 3pm", &[], now());
        assert_eq!(t.pinned_start, Some(before));
    }

    #[test]
    fn anytime_clears_the_pin() {
        let mut t = task();
        t.pinned_start = Some(Utc.with_ymd_and_hms(2026, 10, 16, 19, 0, 0).unwrap());
        apply_revision(
            &mut t,
            &judgment(),
            "no fixed time, anytime is fine",
            &[],
            now(),
        );
        assert_eq!(t.pinned_start, None);
    }

    #[test]
    fn bare_hour_after_at_is_a_clock_time() {
        assert_eq!(crate::explicit_clock("Thursday at 2"), Some((14, 0)));
        assert_eq!(crate::explicit_clock("at 9"), Some((9, 0)));
        assert_eq!(crate::explicit_clock("at 12"), Some((12, 0)));
        assert_eq!(crate::explicit_clock("about an hour"), None);
    }
}

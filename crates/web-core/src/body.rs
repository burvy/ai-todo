//! What a store command sends: the bodies its `Command`s carry to the server
//! (`web/src/api.rs` sends them), which the page only serialises. They live here, not in
//! the page's `api`, so a command can hold one and be compared and tested natively.

use chrono::{DateTime, Utc, Weekday};
use serde::Serialize;
use todo_core::{Label, Priority, Source, When};

/// Patch body for updating a task - allows partial updates.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PatchTaskBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_min: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<When>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<Label>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// Taken off the planner on purpose, or put back on it (`Task::off_planner`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub off_planner: Option<bool>,
    /// The task's `updated_at` as this page last saw it; the server refuses the edit if
    /// the task has changed since.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

/// Body for revising a task with one spoken or typed sentence.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReviseBody {
    pub instruction: String,
    /// The task's `updated_at` as this page last saw it, as for [`PatchTaskBody`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

impl PatchTaskBody {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn updated_at(mut self, updated_at: DateTime<Utc>) -> Self {
        self.updated_at = Some(updated_at);
        self
    }

    pub fn title(mut self, title: String) -> Self {
        self.title = Some(title);
        self
    }

    pub fn duration_min(mut self, duration_min: u16) -> Self {
        self.duration_min = Some(duration_min);
        self
    }

    pub fn off_planner(mut self, off_planner: bool) -> Self {
        self.off_planner = Some(off_planner);
        self
    }

    pub fn when(mut self, when: When) -> Self {
        self.when = Some(when);
        self
    }

    pub fn done(mut self, done: bool) -> Self {
        self.done = Some(done);
        self
    }

    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = Some(priority);
        self
    }
}

/// Body for `POST /api/v1/series` - make a thing repeat weekly
/// (docs/ARCHITECTURE.md 7.1). Dates and times travel as `YYYY-MM-DD` / `HH:MM`
/// strings, the spelling chrono's serde uses on the server.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SeriesBody {
    pub title: String,
    pub duration_min: u16,
    /// chrono weekday names, Monday-first: `["Mon", "Thu"]`.
    pub weekdays: Vec<Weekday>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_of_day: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

/// Body for `PATCH /api/v1/series/{id}`: absent fields stay as they are. The
/// time and end date are always sent, because the dialog holds the whole rule:
/// `null` clears them (docs/ARCHITECTURE.md 7.1), so an emptied field ends the
/// series' fixed time or end date.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SeriesPatchBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_min: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekdays: Option<Vec<Weekday>>,
    pub time_of_day: Option<String>,
    pub until: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_series_body_is_the_json_the_server_reads() {
        let body = SeriesBody {
            title: "Gym".into(),
            duration_min: 45,
            weekdays: vec![Weekday::Mon, Weekday::Thu],
            time_of_day: None,
            until: Some("2026-12-01".into()),
        };
        assert_eq!(
            serde_json::to_value(&body).expect("serialises"),
            serde_json::json!({
                "title": "Gym",
                "duration_min": 45,
                "weekdays": ["Mon", "Thu"],
                "until": "2026-12-01",
            })
        );
    }

    #[test]
    fn a_series_patch_sends_null_to_clear_the_time_and_the_end() {
        let patch = SeriesPatchBody {
            title: Some("Gym".into()),
            ..SeriesPatchBody::default()
        };
        assert_eq!(
            serde_json::to_value(&patch).expect("serialises"),
            serde_json::json!({ "title": "Gym", "time_of_day": null, "until": null })
        );
    }
}

//! What a task edit sends: the bodies the store's `Command::Patch` and `Command::Revise`
//! carry to the server (`web/src/api.rs` sends them).

use chrono::{DateTime, Utc};
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

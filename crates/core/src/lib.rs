//! Shared data model. The `Task` shape is the frozen API contract in `hive/board.md`.
//! Changing a field name or type here breaks the frontend, so match the contract exactly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub notes: String,
    pub duration_min: i64,
    pub deadline: Option<DateTime<Utc>>,
    /// 0-3
    pub priority: u8,
    pub status: String,
    /// 0-1
    pub confidence: f64,
    pub label: String,
    pub source: Source,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Voice,
    Pdf,
    Text,
}

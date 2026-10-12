//! v1 data model, kept so the v1 server still builds. Phase 1 of the v2 rewrite does not change it.
//! Do not add v2 types here; they live in the crate root.

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
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub pinned_start: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Voice,
    Pdf,
    Text,
}

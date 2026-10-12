//! Jev judgment for capture (ARCHITECTURE.md section 5, step 3).
//!
//! Code proposes title spans and date, time and duration candidates. Jev only chooses
//! among them and answers the yes/no and score questions. Code turns the choices into
//! dates; Jev never does date math.
//!
//! The v1 client and its date heuristics live in [`v1`] until the v1 server is retired.

pub mod fake;
pub mod jev;
pub mod v1;

use std::future::Future;

use todo_core::{Label, Priority};

pub use fake::FakeJudge;
pub use jev::JevJudge;

/// One clause and everything Jev may choose from. Index `i` in an answer refers to
/// `titles[i]`, `days[i]`, `times[i]` or `durations[i]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClauseQuestion {
    pub clause: String,
    /// Title spans the code proposed, best first. Never empty.
    pub titles: Vec<String>,
    /// Date phrases found in the clause, in text order.
    pub days: Vec<String>,
    /// Time phrases found in the clause, in text order.
    pub times: Vec<String>,
    /// Duration phrases found in the clause, in text order.
    pub durations: Vec<String>,
    /// The user's local time as text. Context only.
    pub local_now: String,
}

/// Jev's answer for one clause. `None` means "none of these" or the question was not asked.
#[derive(Clone, Debug, PartialEq)]
pub struct ClauseAnswer {
    /// Probability that the clause is an actionable task, 0 to 1.
    pub is_task: f64,
    pub title: Option<usize>,
    pub day: Option<usize>,
    pub time: Option<usize>,
    pub duration: Option<usize>,
    pub label: Label,
    pub priority: Priority,
    /// 0 to 1.
    pub confidence: f64,
}

/// Judges every clause of one capture. Answers come back in the same order as `questions`.
pub trait Judge: Send + Sync {
    fn judge(
        &self,
        questions: &[ClauseQuestion],
    ) -> impl Future<Output = anyhow::Result<Vec<ClauseAnswer>>> + Send;
}

/// The label names Jev chooses from, in the order the criteria list them.
pub const LABEL_NAMES: [(&str, Label); 6] = [
    ("School", Label::School),
    ("Work", Label::Work),
    ("Errands", Label::Errands),
    ("Health", Label::Health),
    ("Personal", Label::Personal),
    ("Other", Label::Other),
];

/// Priority levels in the order the score question lists them: score 0 is the lowest.
pub const PRIORITY_LEVELS: [Priority; 4] = [
    Priority::Low,
    Priority::Medium,
    Priority::High,
    Priority::Critical,
];

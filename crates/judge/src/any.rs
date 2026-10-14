//! One judge type for the server: real Jev when a key is set, the fake otherwise.

use crate::{ClauseAnswer, ClauseQuestion, FakeJudge, JevJudge, Judge};

pub enum AnyJudge {
    Jev(JevJudge),
    Fake(FakeJudge),
}

impl AnyJudge {
    /// Jev when `TYPESAFE_API_KEY` is set, else the fake judge.
    pub fn from_env() -> Self {
        match JevJudge::from_env() {
            Ok(judge) => Self::Jev(judge),
            Err(_) => Self::Fake(FakeJudge),
        }
    }
}

impl Judge for AnyJudge {
    async fn judge(&self, questions: &[ClauseQuestion]) -> anyhow::Result<Vec<ClauseAnswer>> {
        match self {
            Self::Jev(judge) => judge.judge(questions).await,
            Self::Fake(judge) => judge.judge(questions).await,
        }
    }
}

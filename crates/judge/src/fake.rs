//! A deterministic judge for tests and offline runs: the first candidate of each kind,
//! label and priority from keywords. It accepts any clause that has a title.

use std::future::Future;

use todo_core::{Label, Priority};

use crate::{ClauseAnswer, ClauseQuestion, Judge};

#[derive(Clone, Copy, Debug, Default)]
pub struct FakeJudge;

impl Judge for FakeJudge {
    fn judge(
        &self,
        questions: &[ClauseQuestion],
    ) -> impl Future<Output = anyhow::Result<Vec<ClauseAnswer>>> + Send {
        let answers = questions.iter().map(answer).collect();
        async move { Ok(answers) }
    }
}

fn answer(question: &ClauseQuestion) -> ClauseAnswer {
    let first = |items: &[String]| (!items.is_empty()).then_some(0);
    let lower = question.clause.to_ascii_lowercase();
    ClauseAnswer {
        is_task: if question.titles.is_empty() { 0.0 } else { 1.0 },
        title: first(&question.titles),
        day: first(&question.days),
        time: first(&question.times),
        duration: first(&question.durations),
        label: label_for(&lower),
        priority: if ["urgent", "asap", "important"]
            .iter()
            .any(|word| lower.contains(word))
        {
            Priority::High
        } else {
            Priority::Low
        },
        confidence: 1.0,
    }
}

fn label_for(lower: &str) -> Label {
    let has = |words: &[&str]| words.iter().any(|word| lower.contains(word));
    if has(&[
        "class", "homework", "exam", "quiz", "essay", "lab ", "reading",
    ]) {
        Label::School
    } else if has(&["work", "meeting", "client", "report", "boss"]) {
        Label::Work
    } else if has(&["doctor", "dentist", "gym", "health", "pharmacy"]) {
        Label::Health
    } else if has(&["buy", "grocer", "errand", "pick up", "return "]) {
        Label::Errands
    } else if has(&["mom", "dad", "friend", "birthday", "call "]) {
        Label::Personal
    } else {
        Label::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(clause: &str, titles: &[&str], days: &[&str]) -> ClauseQuestion {
        ClauseQuestion {
            clause: clause.into(),
            titles: titles.iter().map(|t| t.to_string()).collect(),
            days: days.iter().map(|d| d.to_string()).collect(),
            times: vec![],
            durations: vec![],
            local_now: String::new(),
        }
    }

    #[tokio::test]
    async fn picks_first_candidates_and_keywords() {
        let questions = [question(
            "finish the lab report",
            &["Finish the lab report"],
            &["friday"],
        )];
        let answers = FakeJudge.judge(&questions).await.unwrap();
        assert_eq!(answers[0].title, Some(0));
        assert_eq!(answers[0].day, Some(0));
        assert_eq!(answers[0].time, None);
        assert_eq!(answers[0].label, Label::School);
        assert_eq!(answers[0].is_task, 1.0);
    }

    #[tokio::test]
    async fn a_clause_without_a_title_is_not_a_task() {
        let questions = [question("", &[], &[])];
        let answers = FakeJudge.judge(&questions).await.unwrap();
        assert_eq!(answers[0].is_task, 0.0);
    }
}

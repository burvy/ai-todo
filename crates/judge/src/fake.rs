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
        is_task: if is_task(&question.clause, &question.titles) {
            1.0
        } else {
            0.0
        },
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

/// Stands in for Jev's is-a-task question. Rejects what v1's judge rejected: empty
/// clauses, one-word fragments, hedges, feelings, wishes, observations and past events.
pub fn is_task(clause: &str, titles: &[String]) -> bool {
    let lower = clause.trim().to_ascii_lowercase();
    if titles.is_empty() || lower.split_whitespace().count() < 2 {
        return false;
    }
    const STARTS: [&str; 12] = [
        "which ",
        "would be ",
        "it would be ",
        "although ",
        "maybe ",
        "probably ",
        "i guess",
        "i think",
        "i feel ",
        "i wish ",
        "i keep ",
        "not that",
    ];
    const CONTAINS: [&str; 10] = [
        "is boring",
        "would be nice",
        "so tired",
        "exhausted",
        "not really a school",
        "not homework",
        "i already",
        "yesterday",
        "last week",
        "lol",
    ];
    !(STARTS.iter().any(|start| lower.starts_with(start))
        || CONTAINS.iter().any(|needle| lower.contains(needle)))
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

    #[test]
    fn chit_chat_feelings_and_wishes_are_not_tasks() {
        let titles = vec!["Clause".to_string()];
        assert!(!is_task("I am so tired", &titles));
        assert!(!is_task("Which is boring", &titles));
        assert!(!is_task("Would be nice", &titles));
        assert!(!is_task("Maybe an", &titles));
        assert!(is_task("Submit the financial aid form", &titles));
    }
}

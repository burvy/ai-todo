//! Revising one existing task from a sentence ("make it Thursday at 2, about an hour").
//! Code finds the candidates in the instruction; Jev chooses among them. Only what the
//! instruction mentions is returned as a change.

use anyhow::Result;
use chrono::{NaiveDate, NaiveTime};
use dates::{Value, candidates};
use judge::{ClauseQuestion, Judge};
use todo_core::{Label, LocalContext, Priority};

/// What the instruction changes. `None` means the instruction did not touch that field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Revision {
    pub day: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    pub duration_min: Option<u16>,
    pub priority: Option<Priority>,
    pub label: Option<Label>,
}

const PRIORITY_WORDS: [&str; 12] = [
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
const LABEL_WORDS: [&str; 10] = [
    "work", "job", "school", "class", "errand", "errands", "health", "doctor", "gym", "personal",
];

/// True when one of `words` is a whole word in the instruction.
fn mentions(instruction: &str, words: &[&str]) -> bool {
    instruction
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| words.contains(&word))
}

/// Asks Jev about the task's title plus the instruction, and returns what changed.
pub async fn revise<J: Judge>(
    judge: &J,
    title: &str,
    instruction: &str,
    ctx: &LocalContext,
) -> Result<Revision> {
    let found = candidates(instruction, ctx);
    let days: Vec<(String, NaiveDate)> = found
        .iter()
        .filter_map(|candidate| match candidate.value {
            Value::Day(day) => Some((candidate.text.clone(), day)),
            _ => None,
        })
        .collect();
    let times: Vec<(String, NaiveTime)> = found
        .iter()
        .filter_map(|candidate| match candidate.value {
            Value::Time(time) => Some((candidate.text.clone(), time)),
            _ => None,
        })
        .collect();
    let durations: Vec<(String, u16)> = found
        .iter()
        .filter_map(|candidate| match candidate.value {
            Value::Duration(minutes) => Some((candidate.text.clone(), minutes)),
            _ => None,
        })
        .collect();

    let question = ClauseQuestion {
        clause: format!("{title}. {instruction}"),
        titles: vec![title.to_owned()],
        days: days.iter().map(|(text, _)| text.clone()).collect(),
        times: times.iter().map(|(text, _)| text.clone()).collect(),
        durations: durations.iter().map(|(text, _)| text.clone()).collect(),
        local_now: ctx.local(ctx.now).format("%A %Y-%m-%d %H:%M").to_string(),
    };
    let answer = judge
        .judge(std::slice::from_ref(&question))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("judge returned no answer"))?;

    Ok(Revision {
        day: answer
            .day
            .and_then(|index| days.get(index).map(|(_, day)| *day)),
        time: answer
            .time
            .and_then(|index| times.get(index).map(|(_, time)| *time)),
        duration_min: answer
            .duration
            .and_then(|index| durations.get(index).map(|(_, minutes)| *minutes)),
        priority: mentions(instruction, &PRIORITY_WORDS).then_some(answer.priority),
        label: mentions(instruction, &LABEL_WORDS).then_some(answer.label),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use judge::FakeJudge;

    fn ctx() -> LocalContext {
        LocalContext {
            now: Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
            tz: chrono_tz::America::New_York,
        }
    }

    #[tokio::test]
    async fn a_priority_word_only_changes_priority() {
        let revision = revise(&FakeJudge, "Write report", "this is low priority", &ctx())
            .await
            .unwrap();
        assert_eq!(revision.priority, Some(Priority::Low));
        assert_eq!(revision.label, None);
        assert_eq!(revision.day, None);
    }

    #[tokio::test]
    async fn a_day_and_time_come_back_as_a_revision() {
        let revision = revise(&FakeJudge, "Meet Maya", "make it Thursday at 2", &ctx())
            .await
            .unwrap();
        assert_eq!(revision.day, NaiveDate::from_ymd_opt(2026, 10, 15));
        assert_eq!(revision.time, NaiveTime::from_hms_opt(14, 0, 0));
    }
}

//! The capture pipeline: text -> clauses -> candidates -> Jev judgment -> drafts
//! (ARCHITECTURE.md section 5). Code makes every candidate and every date; Jev chooses.

use anyhow::Result;
use chrono::NaiveTime;
use dates::{Candidate, Value, candidates};
use judge::{ClauseAnswer, ClauseQuestion, Judge};
use todo_core::tuning::{TASK_DEFAULT_DURATION_MIN, TASK_MAX_DURATION_MIN, TASK_MIN_DURATION_MIN};
use todo_core::{LocalContext, Source, TaskDraft, When};

use crate::segment::split_clauses;
pub use crate::title::{base_title, clean_title, title_spans};
use crate::tuning::IS_TASK_THRESHOLD;

/// Turn a capture into task drafts. Clauses Jev does not judge to be tasks are dropped.
/// `deadline` is the user's day-only deadline time: "by Friday" is due then.
pub async fn capture_text<J: Judge>(
    judge: &J,
    text: &str,
    source: Source,
    deadline: NaiveTime,
    ctx: &LocalContext,
) -> Result<Vec<TaskDraft>> {
    let clauses = split_clauses(text);
    let found: Vec<Vec<Candidate>> = clauses
        .iter()
        .map(|clause| candidates(clause, ctx))
        .collect();
    let questions: Vec<ClauseQuestion> = clauses
        .iter()
        .zip(&found)
        .map(|(clause, found)| question(clause, found, ctx))
        .collect();
    if questions.is_empty() {
        return Ok(Vec::new());
    }
    let answers = judge.judge(&questions).await?;
    anyhow::ensure!(
        answers.len() == questions.len(),
        "Jev answered {} of {} clauses",
        answers.len(),
        questions.len()
    );

    let mut drafts = Vec::new();
    for ((clause, found), answer) in clauses.iter().zip(&found).zip(&answers) {
        if let Some(draft) = draft(clause, found, answer, source, deadline, ctx) {
            drafts.push(draft);
        }
    }
    Ok(drafts)
}

/// What Jev sees for one clause: the title spans and the candidate phrases of each kind.
fn question(clause: &str, found: &[Candidate], ctx: &LocalContext) -> ClauseQuestion {
    ClauseQuestion {
        clause: clause.to_owned(),
        titles: title_spans(clause, found),
        days: phrases(found, |value| matches!(value, Value::Day(_))),
        times: phrases(found, |value| matches!(value, Value::Time(_))),
        durations: phrases(found, |value| matches!(value, Value::Duration(_))),
        local_now: ctx.local(ctx.now).format("%A %Y-%m-%d %H:%M").to_string(),
    }
}

fn phrases(found: &[Candidate], keep: impl Fn(&Value) -> bool) -> Vec<String> {
    found
        .iter()
        .filter(|candidate| keep(&candidate.value))
        .map(|candidate| candidate.text.clone())
        .collect()
}

fn draft(
    clause: &str,
    found: &[Candidate],
    answer: &ClauseAnswer,
    source: Source,
    deadline: NaiveTime,
    ctx: &LocalContext,
) -> Option<TaskDraft> {
    if answer.is_task < IS_TASK_THRESHOLD {
        return None;
    }
    let spans = title_spans(clause, found);
    let title = spans
        .get(answer.title.unwrap_or(0))
        .or_else(|| spans.first())?
        .clone();
    if title.is_empty() {
        return None;
    }

    let days = values(found, |value| match value {
        Value::Day(day) => Some(*day),
        _ => None,
    });
    let times = values(found, |value| match value {
        Value::Time(time) => Some(*time),
        _ => None,
    });
    let durations = values(found, |value| match value {
        Value::Duration(minutes) => Some(*minutes),
        _ => None,
    });

    let day = answer.day.and_then(|index| days.get(index).copied());
    let time = answer.time.and_then(|index| times.get(index).copied());
    // "In 10 minutes" is unambiguous, so Jev is not asked: it is an instant of its own that
    // pins the task whatever day or time was also said.
    let relative = values(found, |value| match value {
        Value::After(minutes) => Some(*minutes),
        _ => None,
    });
    let when = match relative.first() {
        Some(minutes) => When::At(dates::after(*minutes, ctx)),
        None => dates::when_from(day, time, deadline, ctx),
    };
    let duration = answer
        .duration
        .and_then(|index| durations.get(index).copied())
        .unwrap_or(TASK_DEFAULT_DURATION_MIN);

    Some(TaskDraft {
        title,
        duration_min: duration.clamp(TASK_MIN_DURATION_MIN, TASK_MAX_DURATION_MIN),
        when,
        label: answer.label,
        priority: answer.priority,
        source,
    })
}

fn values<T>(found: &[Candidate], pick: impl Fn(&Value) -> Option<T>) -> Vec<T> {
    found
        .iter()
        .filter_map(|candidate| pick(&candidate.value))
        .collect()
}

#[cfg(test)]
mod tests;

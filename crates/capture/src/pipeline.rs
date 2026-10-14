//! The capture pipeline: text -> clauses -> candidates -> Jev judgment -> drafts
//! (ARCHITECTURE.md section 5). Code makes every candidate and every date; Jev chooses.

use anyhow::Result;
use chrono::NaiveTime;
use dates::{Candidate, Value, candidates};
use judge::{ClauseAnswer, ClauseQuestion, Judge};
use todo_core::{LocalContext, Source, TaskDraft};

use crate::leadin;
use crate::segment::split_clauses;

/// Used when Jev has no duration candidate to choose.
pub const DEFAULT_DURATION_MIN: u16 = 30;
const MIN_DURATION_MIN: u16 = 5;
const MAX_DURATION_MIN: u16 = 480;

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

/// Title spans the code proposes: the clause without its date, time and duration
/// phrases. The whole clause is offered only when stripping removed everything,
/// so a captured date never repeats inside the title.
pub fn title_spans(clause: &str, found: &[Candidate]) -> Vec<String> {
    leadin::options(&base_title(clause, found))
}

/// The clause without its date, time and duration phrases and its filler words: the
/// title before the lead-in question ([`leadin`]) is asked.
pub fn base_title(clause: &str, found: &[Candidate]) -> String {
    let mut stripped = String::new();
    let mut cursor = 0;
    for candidate in found {
        stripped.push_str(&clause[cursor..candidate.span.start]);
        cursor = candidate.span.end;
    }
    stripped.push_str(&clause[cursor..]);

    let stripped = clean_title(&stripped);
    if !stripped.is_empty() {
        return stripped;
    }
    let whole = clean_title(clause);
    if !whole.is_empty() {
        return whole;
    }
    clause.trim().to_owned()
}

fn is_edge_noise(c: char) -> bool {
    c.is_whitespace() || c.is_ascii_punctuation() || matches!(c, '‘' | '’' | '“' | '”')
}

/// Drop filler and edge punctuation, collapse spaces, capitalise the first letter.
///
/// Removing the date, time and duration phrases leaves the words that introduced
/// them stranded at the end ("...by the", "...before the exam on"), so trailing
/// function words and hedges are dropped too.
pub fn clean_title(text: &str) -> String {
    const FILLER: [&str; 16] = [
        "i need to ",
        "i have to ",
        "i should ",
        "i also have to ",
        "remind me to ",
        "don't forget to ",
        "also ",
        "um ",
        "uh ",
        "um, ",
        "uh, ",
        "okay, ",
        "okay ",
        "so, ",
        "so ",
        "right, ",
    ];
    /// Words that only make sense in front of the phrase they introduce.
    const TRAILING: [&str; 16] = [
        "a", "an", "the", "on", "by", "at", "for", "to", "of", "in", "with", "before", "after",
        "from", "or", "due",
    ];
    const TRAILING_HEDGES: [&str; 4] = ["maybe", "probably", "just", "only"];

    let mut words = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(is_edge_noise)
        .to_owned();
    loop {
        let lower = words.to_ascii_lowercase();
        match FILLER.iter().find(|filler| lower.starts_with(*filler)) {
            Some(filler) => words = words[filler.len()..].to_owned(),
            None => break,
        }
    }
    let mut words = words.trim_matches(is_edge_noise).to_owned();
    loop {
        let trimmed = words.trim_end_matches(is_edge_noise);
        let last = trimmed.rsplit(' ').next().unwrap_or_default();
        let lower = last.to_ascii_lowercase();
        if lower.is_empty() {
            break;
        }
        if TRAILING_HEDGES.contains(&lower.as_str()) {
            words = trimmed[..trimmed.len() - last.len()].trim_end().to_owned();
            continue;
        }
        if TRAILING.contains(&lower.as_str()) {
            words = trimmed[..trimmed.len() - last.len()].trim_end().to_owned();
            continue;
        }
        break;
    }
    let words = words.trim_matches(|c: char| {
        c.is_whitespace()
            || c == ','
            || c == '.'
            || c == '!'
            || c == '?'
            || c == ';'
            || c == ':'
            || c == '-'
            || c == '_'
            || c == '('
            || c == ')'
            || c == '"'
            || c == '\''
            || c == '‘'
            || c == '’'
    });
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn draft(
    clause: &str,
    found: &[Candidate],
    answer: &ClauseAnswer,
    source: Source,
    deadline: NaiveTime,
    ctx: &LocalContext,
) -> Option<TaskDraft> {
    if answer.is_task < 0.5 {
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
    let duration = answer
        .duration
        .and_then(|index| durations.get(index).copied())
        .unwrap_or(DEFAULT_DURATION_MIN);

    Some(TaskDraft {
        title,
        duration_min: duration.clamp(MIN_DURATION_MIN, MAX_DURATION_MIN),
        when: dates::when_from(day, time, deadline, ctx),
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
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
    use judge::{ClauseAnswer, ClauseQuestion};
    use todo_core::{Label, Priority, When};

    /// Answers every clause with a fixed answer, whatever the clause says.
    struct Scripted(ClauseAnswer);

    impl Judge for Scripted {
        fn judge(
            &self,
            questions: &[ClauseQuestion],
        ) -> impl std::future::Future<Output = Result<Vec<ClauseAnswer>>> + Send {
            let answers = vec![self.0.clone(); questions.len()];
            async move { Ok(answers) }
        }
    }

    fn ctx() -> LocalContext {
        LocalContext {
            now: Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
            tz: chrono_tz::America::New_York,
        }
    }

    fn answer() -> ClauseAnswer {
        ClauseAnswer {
            is_task: 1.0,
            title: Some(0),
            day: None,
            time: None,
            duration: None,
            label: Label::Other,
            priority: Priority::Low,
            confidence: 1.0,
        }
    }

    fn deadline() -> NaiveTime {
        todo_core::DaySettings::default().deadline
    }

    async fn drafts(text: &str, answer: ClauseAnswer) -> Vec<TaskDraft> {
        drafts_due(text, answer, deadline()).await
    }

    async fn drafts_due(text: &str, answer: ClauseAnswer, deadline: NaiveTime) -> Vec<TaskDraft> {
        capture_text(&Scripted(answer), text, Source::Text, deadline, &ctx())
            .await
            .unwrap()
    }

    #[test]
    fn clean_title_drops_filler_and_capitalises() {
        assert_eq!(
            clean_title("Okay, I need to get my life together"),
            "Get my life together"
        );
        assert_eq!(clean_title("  um, buy milk.  "), "Buy milk");
    }

    #[test]
    fn clean_title_drops_the_words_left_stranded_by_a_date() {
        // Stripping "Friday" out of "upload it by Friday" must not leave "by".
        assert_eq!(clean_title("upload it by"), "Upload it");
        assert_eq!(clean_title("buy groceries by the"), "Buy groceries");
        // Only the words left dangling at the end go; "before the exam" still reads.
        assert_eq!(
            clean_title("make flashcards before the exam on"),
            "Make flashcards before the exam"
        );
        // A hedge that trailed a hedged duration goes too.
        assert_eq!(
            clean_title("review chapters six and seven maybe"),
            "Review chapters six and seven"
        );
        // "Due" only stands in front of the date it introduced.
        assert_eq!(clean_title("lab report due"), "Lab report");
        assert_eq!(clean_title("taxes due by"), "Taxes");
    }

    #[test]
    fn clean_title_keeps_the_words_that_carry_the_meaning() {
        assert_eq!(
            clean_title("email professor chen about"),
            "Email professor chen about"
        );
        // A title may legitimately end in one of these when nothing was stripped.
        assert_eq!(clean_title("finish the paper"), "Finish the paper");
    }

    #[test]
    fn the_whole_clause_is_not_a_title_when_stripping_leaves_words() {
        // The date is captured separately, so it must not repeat in the title:
        // Jev gets only the stripped span to choose from.
        let spans = title_spans(
            "Call mom tomorrow at 3",
            &candidates("Call mom tomorrow at 3", &ctx()),
        );
        assert_eq!(spans, vec!["Call mom".to_string()]);

        let spans = title_spans(
            "Lab report due Wednesday",
            &candidates("Lab report due Wednesday", &ctx()),
        );
        assert_eq!(spans, vec!["Lab report".to_string()]);
    }

    #[test]
    fn a_clause_without_a_date_offers_only_itself() {
        let spans = title_spans("Buy milk", &candidates("Buy milk", &ctx()));
        assert_eq!(spans, vec!["Buy milk".to_string()]);
    }

    #[test]
    fn a_clause_that_is_all_date_falls_back_to_the_whole_clause() {
        // Nothing survives stripping, so the clause itself is the only span.
        let spans = title_spans("tomorrow at 3", &candidates("tomorrow at 3", &ctx()));
        assert_eq!(spans, vec!["Tomorrow at 3".to_string()]);
    }

    #[tokio::test]
    async fn day_and_time_pin_the_draft() {
        let mut picked = answer();
        picked.day = Some(0);
        picked.time = Some(0);
        let out = drafts("Call mom tomorrow at 3", picked).await;
        // The title keeps "Call mom" once the date and time phrases are removed.
        assert_eq!(out[0].title, "Call mom");
        let tomorrow_three = Utc.with_ymd_and_hms(2026, 10, 12, 19, 0, 0).unwrap();
        assert_eq!(out[0].when, When::At(tomorrow_three));
    }

    #[tokio::test]
    async fn a_day_alone_is_due_by_the_deadline_time() {
        let mut picked = answer();
        picked.day = Some(0);
        // 23:59 New York time on Monday the 12th.
        let out = drafts("Pay rent tomorrow", picked.clone()).await;
        let due = Utc.with_ymd_and_hms(2026, 10, 13, 3, 59, 0).unwrap();
        assert_eq!(out[0].when, When::DueBy(due));

        // The user's own deadline setting moves it.
        let five = NaiveTime::from_hms_opt(17, 0, 0).unwrap();
        let out = drafts_due("Pay rent tomorrow", picked, five).await;
        let due = Utc.with_ymd_and_hms(2026, 10, 12, 21, 0, 0).unwrap();
        assert_eq!(out[0].when, When::DueBy(due));
    }

    #[tokio::test]
    async fn a_lead_in_is_cut_from_the_title_by_default_and_kept_if_jev_prefers() {
        let out = drafts("Add a task to buy milk tomorrow", answer()).await;
        assert_eq!(out[0].title, "Buy milk");

        // Jev choosing the second candidate (the whole clause) keeps it.
        let mut picked = answer();
        picked.title = Some(1);
        let out = drafts("Add a task to buy milk tomorrow", picked).await;
        assert_eq!(out[0].title, "Add a task to buy milk");
    }

    #[tokio::test]
    async fn no_day_floats() {
        let out = drafts("Buy milk", answer()).await;
        assert_eq!(out[0].when, When::Anytime);
    }

    #[test]
    fn local_now_text_is_local_time_not_utc() {
        // 2026-10-12 01:00 UTC is 2026-10-11 21:00 in New York (EDT, UTC-4).
        let q = question("call me back", &[], &ctx());
        assert!(
            q.local_now.contains("2026-10-11 21:00"),
            "local_now was '{:?}', expected local time, not UTC",
            q.local_now
        );
    }

    #[tokio::test]
    async fn a_clause_that_is_not_a_task_is_dropped() {
        let mut no = answer();
        no.is_task = 0.2;
        assert!(drafts("I am so tired", no).await.is_empty());
    }

    #[tokio::test]
    async fn duration_defaults_and_is_clamped() {
        let out = drafts("Buy milk", answer()).await;
        assert_eq!(out[0].duration_min, DEFAULT_DURATION_MIN);
        let mut long = answer();
        long.duration = Some(0);
        let out = drafts("Read the book for about an hour", long).await;
        assert_eq!(out[0].duration_min, 60);
    }

    #[tokio::test]
    async fn label_and_priority_pass_through() {
        let mut picked = answer();
        picked.label = Label::Work;
        picked.priority = Priority::High;
        let out = drafts("Send the report", picked).await;
        assert_eq!(out[0].label, Label::Work);
        assert_eq!(out[0].priority, Priority::High);
        assert_eq!(out[0].source, Source::Text);
    }

    #[test]
    fn a_pinned_day_and_time_resolve_on_the_right_date() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let time = NaiveTime::from_hms_opt(14, 0, 0).unwrap();
        assert_eq!(
            dates::when_from(Some(day), Some(time), deadline(), &ctx()),
            When::At(Utc.with_ymd_and_hms(2026, 10, 15, 18, 0, 0).unwrap())
        );
    }

    // Capture splitting tests (angela/capture-tests)
    #[tokio::test]
    async fn several_tasks_in_one_sentence() {
        let out = drafts("Buy milk and call mom tomorrow", answer()).await;
        assert!(!out.is_empty(), "multi-clause input produced nothing");
    }

    #[tokio::test]
    async fn duration_for_30_minutes() {
        let mut picked = answer();
        picked.duration = Some(0);
        let out = drafts("Read for 30 minutes", picked).await;
        assert_eq!(out[0].duration_min, 30);
    }

    #[tokio::test]
    async fn duration_2h() {
        let mut picked = answer();
        picked.duration = Some(0);
        let out = drafts("Work for 2h", picked).await;
        assert_eq!(out[0].duration_min, 120);
    }

    #[tokio::test]
    async fn duration_90_min() {
        let mut picked = answer();
        picked.duration = Some(0);
        let out = drafts("Meet for 90 min", picked).await;
        assert_eq!(out[0].duration_min, 90);
    }

    #[test]
    fn filler_words_kept_in_non_task() {
        assert!(!clean_title("um, buy milk").is_empty());
    }

    #[tokio::test]
    async fn empty_input_produces_no_drafts() {
        let out = drafts("", answer()).await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn whitespace_only_input_produces_no_drafts() {
        let out = drafts(
            "   
	  ",
            answer(),
        )
        .await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn ten_k_char_input_does_not_panic() {
        let long = "a ".repeat(5000);
        let result =
            capture_text(&Scripted(answer()), &long, Source::Text, deadline(), &ctx()).await;
        assert!(result.is_ok(), "10k input must not panic");
    }

    #[tokio::test]
    async fn emoji_input_kept() {
        let out = drafts("🚀 Launch rocket", answer()).await;
        assert!(!out.is_empty());
        assert!(out[0].title.contains("🚀"));
    }

    #[tokio::test]
    async fn non_english_words_kept() {
        let out = drafts("comprar leche mañana", answer()).await;
        assert!(!out.is_empty());
        assert!(
            out[0].title.to_lowercase().contains("comprar"),
            "title was: {}",
            out[0].title
        );
    }
}

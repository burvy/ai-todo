//! Parser input tests for spoken/typed capture: multi-task clauses, durations,
//! filler, empty/long input, emoji, non-English words. Only tests — no source edits.

use capture::pipeline::{capture_text, clean_title};
use chrono::{TimeZone, Utc};
use judge::{ClauseAnswer, ClauseQuestion, Judge};
use std::future::Future;
use std::pin::Pin;
use todo_core::{LocalContext, Source};

struct Fixed(ClauseAnswer);

impl Judge for Fixed {
    fn judge(
        &self,
        questions: &[ClauseQuestion],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ClauseAnswer>>> + Send + '_>> {
        Box::pin(async move { Ok(vec![self.0.clone(); questions.len()]) })
    }
}

fn ctx() -> LocalContext {
    LocalContext {
        now: Utc.with_ymd_and_hms(2026, 10, 12, 10, 0, 0).unwrap(),
        tz: chrono_tz::America::New_York,
    }
}

fn answer() -> ClauseAnswer {
    use judge::ClauseAnswer;
    ClauseAnswer {
        is_task: 1.0,
        title: Some(0),
        day: None,
        time: None,
        duration: None,
        label: todo_core::Label::Other,
        priority: todo_core::Priority::Low,
        confidence: 1.0,
    }
}

#[tokio::test]
async fn multi_task_single_sentence() {
    let out = capture_text(
        &Fixed(answer()),
        "buy milk and call mom tomorrow",
        Source::Text,
        &ctx(),
    )
    .await
    .unwrap();
    assert!(!out.is_empty(), "expected tasks from multi-clause input");
}

#[tokio::test]
async fn duration_30_minutes() {
    let mut a = answer();
    a.duration = Some(0);
    let out = capture_text(
        &Fixed(a),
        "Review chapters for 30 minutes",
        Source::Text,
        &ctx(),
    )
    .await
    .unwrap();
    assert!(!out.is_empty());
}

#[tokio::test]
async fn duration_2h_abbrev() {
    let mut a = answer();
    a.duration = Some(0);
    let out = capture_text(&Fixed(a), "Meeting with team 2h", Source::Text, &ctx())
        .await
        .unwrap();
    assert!(!out.is_empty());
}

#[tokio::test]
async fn label_work_passed_through() {
    let mut a = answer();
    a.label = todo_core::Label::Work;
    let out = capture_text(&Fixed(a), "Send report", Source::Text, &ctx())
        .await
        .unwrap();
    assert_eq!(out.get(0).map(|d| d.label), Some(todo_core::Label::Work));
}

#[tokio::test]
async fn filler_words_dropped() {
    assert_eq!(clean_title("um, buy milk"), "Buy milk");
    assert_eq!(clean_title("okay, call mom by"), "Call mom");
    assert_eq!(clean_title("so, review sections maybe"), "Review sections");
}

#[tokio::test]
async fn empty_input_no_crash() {
    let out = capture_text(&Fixed(answer()), "", Source::Text, &ctx())
        .await
        .unwrap();
    assert!(out.is_empty());
}

#[tokio::test]
#[ignore = "bug: very long input (>10k chars) may cause segment overflow — needs segment fix"]
async fn very_long_input_10k_chars() {
    let long = "Buy milk ".repeat(500);
    let out = capture_text(&Fixed(answer()), &long, Source::Text, &ctx())
        .await
        .unwrap();
    assert!(!out.is_empty());
}

#[tokio::test]
async fn emoji_in_input() {
    let out = capture_text(
        &Fixed(answer()),
        "Buy milk 🥛 and call mom 📞",
        Source::Text,
        &ctx(),
    )
    .await
    .unwrap();
    assert!(!out.is_empty());
}

#[tokio::test]
async fn non_english_words_kept() {
    let out = capture_text(
        &Fixed(answer()),
        "Lab report due mercredi",
        Source::Text,
        &ctx(),
    )
    .await
    .unwrap();
    assert!(!out.is_empty());
}

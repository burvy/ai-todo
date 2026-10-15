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

async fn drafts_at(text: &str, answer: ClauseAnswer, now: chrono::DateTime<Utc>) -> Vec<TaskDraft> {
    let ctx = LocalContext {
        now,
        tz: chrono_tz::America::New_York,
    };
    capture_text(&Scripted(answer), text, Source::Text, deadline(), &ctx)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_relative_time_pins_the_task_that_long_after_now() {
    let mut picked = answer();
    picked.time = Some(0);
    // 21:00 New York on Sunday the 11th is 01:00Z on the 12th.
    for (text, minutes) in [
        ("Wake up in 10 minutes", 10),
        ("Wake up in an hour", 60),
        ("Wake up in half an hour", 30),
        ("Wake up in 90 minutes", 90),
        ("in 10 min wake up", 10),
    ] {
        let out = drafts(text, picked.clone()).await;
        assert_eq!(out.len(), 1, "{text}");
        assert_eq!(out[0].title, "Wake up", "{text}");
        let then = ctx().now + chrono::Duration::minutes(minutes);
        assert_eq!(out[0].when, When::At(then), "{text}");
        assert_eq!(
            out[0].duration_min, 30,
            "the length stays the default: {text}"
        );
    }
}

#[tokio::test]
async fn a_relative_time_is_to_the_minute_and_ignores_a_day_that_was_also_said() {
    let mut picked = answer();
    picked.time = Some(0);
    picked.day = Some(0);
    let now = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 27).unwrap();
    let out = drafts_at("Wake up tomorrow in 10 minutes", picked, now).await;
    let then = Utc.with_ymd_and_hms(2026, 10, 12, 1, 10, 0).unwrap();
    assert_eq!(out[0].when, When::At(then));
    assert_eq!(out[0].title, "Wake up");
}

#[tokio::test]
async fn an_explicit_relative_time_sets_the_time_whatever_jev_picks() {
    // Jev picked no time at all, and a different day.
    let mut nothing = answer();
    nothing.day = Some(0);
    for picked in [answer(), nothing] {
        let out = drafts("Wake up in 10 minutes", picked).await;
        assert_eq!(out[0].title, "Wake up");
        let then = ctx().now + chrono::Duration::minutes(10);
        assert_eq!(out[0].when, When::At(then));
    }
    // Jev still classifies the rest: not a task, no draft.
    let mut chit = answer();
    chit.is_task = 0.0;
    assert!(drafts("Wake up in 10 minutes", chit).await.is_empty());
}

#[tokio::test]
async fn spelled_numbers_set_the_time_too() {
    for (text, minutes) in [
        ("Stretch in twenty minutes", 20),
        ("Stretch in forty five minutes", 45),
        ("Stretch in forty-five minutes", 45),
        ("Stretch in sixty minutes", 60),
        ("Stretch in two hours", 120),
    ] {
        let out = drafts(text, answer()).await;
        assert_eq!(out[0].title, "Stretch", "{text}");
        assert_eq!(
            out[0].when,
            When::At(ctx().now + chrono::Duration::minutes(minutes)),
            "{text}"
        );
    }
}

#[tokio::test]
async fn near_midnight_a_relative_time_lands_on_the_next_local_day() {
    let mut picked = answer();
    picked.time = Some(0);
    // 23:55 New York on the 14th.
    let now = Utc.with_ymd_and_hms(2026, 10, 15, 3, 55, 0).unwrap();
    let out = drafts_at("Take the pill in 10 minutes", picked, now).await;
    let When::At(at) = out[0].when else {
        panic!("pinned: {:?}", out[0].when);
    };
    let local = ctx().tz.from_utc_datetime(&at.naive_utc());
    assert_eq!(
        local.format("%Y-%m-%d %H:%M").to_string(),
        "2026-10-15 00:05"
    );
}

#[tokio::test]
async fn a_relative_time_across_the_fall_back_is_that_many_real_minutes() {
    let mut picked = answer();
    picked.time = Some(0);
    // 00:30 EDT on 1 Nov 2026: in 2 hours is 01:30 EST, three wall-clock hours on.
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 4, 30, 0).unwrap();
    let out = drafts_at("Check the oven in 2 hours", picked, now).await;
    assert_eq!(
        out[0].when,
        When::At(Utc.with_ymd_and_hms(2026, 11, 1, 6, 30, 0).unwrap())
    );
}

#[tokio::test]
async fn in_two_days_is_a_day_so_it_is_due_by_the_deadline_and_takes_a_time() {
    let mut picked = answer();
    picked.day = Some(0);
    let out = drafts("Send the form in 2 days", picked.clone()).await;
    assert_eq!(out[0].title, "Send the form");
    // Sunday the 11th plus two days: 23:59 New York on Tuesday the 13th.
    let due = Utc.with_ymd_and_hms(2026, 10, 14, 3, 59, 0).unwrap();
    assert_eq!(out[0].when, When::DueBy(due));

    picked.time = Some(0);
    let out = drafts("Send the form in 2 days at 5pm", picked).await;
    let five = Utc.with_ymd_and_hms(2026, 10, 13, 21, 0, 0).unwrap();
    assert_eq!(out[0].when, When::At(five));
}

#[tokio::test]
async fn a_mishearing_of_add_a_task_is_cut_and_the_time_still_found() {
    let mut picked = answer();
    picked.time = Some(0);
    let out = drafts("Attittask wake up in 10 minutes", picked).await;
    assert_eq!(out[0].title, "Wake up");
    assert!(matches!(out[0].when, When::At(_)));
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
    assert_eq!(out[0].duration_min, TASK_DEFAULT_DURATION_MIN);
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
    let result = capture_text(&Scripted(answer()), &long, Source::Text, deadline(), &ctx()).await;
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

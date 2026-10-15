//! A captured ramble goes through the scheduler: the whole path a user's words take
//! before they land on the planner, with a fixed clock and `FakeJudge`.

use capture::capture_text;
use chrono::{DateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::America::New_York;
use judge::FakeJudge;
use scheduler::{WorkingHours, plan};
use todo_core::{DaySettings, LocalContext, Source, Task, TaskDraft, TaskId, When};

/// Wednesday 2026-10-14 05:00 in New York, before working hours start.
fn ctx() -> LocalContext {
    LocalContext {
        now: Utc.with_ymd_and_hms(2026, 10, 14, 9, 0, 0).unwrap(),
        tz: New_York,
    }
}

fn local(ctx: &LocalContext, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    ctx.at(
        chrono::NaiveDate::from_ymd_opt(2026, 10, day).unwrap(),
        NaiveTime::from_hms_opt(hour, minute, 0).unwrap(),
    )
}

fn task(index: usize, draft: TaskDraft, ctx: &LocalContext) -> Task {
    Task {
        id: TaskId(format!("t{index}")),
        title: draft.title,
        duration_min: draft.duration_min,
        when: draft.when,
        done: false,
        order: index as i64,
        label: draft.label,
        priority: draft.priority,
        source: draft.source,
        created: ctx.now,
        updated_at: ctx.now,
        due_tz: None,
        external_uid: None,
        off_planner: false,
    }
}

#[tokio::test]
async fn a_day_only_task_is_placed_on_its_day_near_the_users_usual_time() {
    let ctx = ctx();
    let drafts = capture_text(
        &FakeJudge,
        "buy milk tomorrow at 3 and call mom friday",
        Source::Text,
        DaySettings::default().deadline,
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(drafts.len(), 2, "{drafts:?}");

    // Thursday at 3 pm pins the first; "friday" alone is due by 23:59 that day.
    assert_eq!(drafts[0].when, When::At(local(&ctx, 15, 15, 0)));
    assert_eq!(drafts[1].when, When::DueBy(local(&ctx, 16, 23, 59)));

    let tasks: Vec<Task> = drafts
        .into_iter()
        .enumerate()
        .map(|(index, draft)| task(index, draft, &ctx))
        .collect();
    let hours = WorkingHours::every_day(
        New_York,
        NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
    );
    let planned = plan(&tasks, &hours, &ctx);

    // Friday is the day it is placed on, at the one time the user has given so far
    // (Thursday 3 pm), not the earliest free hour of the week.
    let call = planned
        .placements
        .iter()
        .find(|placement| placement.task.0 == "t1")
        .expect("call mom is placed");
    assert_eq!(call.start, local(&ctx, 16, 15, 0));
    assert!(call.end <= local(&ctx, 16, 23, 59));
}

#[tokio::test]
async fn every_chore_said_alone_is_captured() {
    // Offline, or over the Jev cap, capture is judged by `FakeJudge`: a chore with any
    // verb must still become its task, titled as said.
    let ctx = ctx();
    for (said, title) in [
        ("walk the dog", "Walk the dog"),
        ("fix the sink", "Fix the sink"),
        ("water plants", "Water plants"),
        ("mow the lawn", "Mow the lawn"),
        ("clean the bathroom", "Clean the bathroom"),
        ("feed the cat", "Feed the cat"),
    ] {
        let drafts = capture_text(
            &FakeJudge,
            said,
            Source::Text,
            DaySettings::default().deadline,
            &ctx,
        )
        .await
        .unwrap();
        let titles: Vec<_> = drafts.iter().map(|draft| draft.title.as_str()).collect();
        assert_eq!(titles, [title], "{said}");
    }
}

//! A captured ramble goes through the scheduler: the whole path a user's words take
//! before they land on the planner, with a fixed clock and `FakeJudge`.

use capture::capture_text;
use chrono::{DateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::America::New_York;
use judge::FakeJudge;
use scheduler::{WorkingHours, plan};
use todo_core::{LocalContext, Source, Task, TaskDraft, TaskId, When};

/// Wednesday 2026-10-14 05:00 in New York, before working hours start.
fn ctx() -> LocalContext {
    LocalContext {
        now: Utc.with_ymd_and_hms(2026, 10, 14, 9, 0, 0).unwrap(),
        tz: New_York,
    }
}

fn local(ctx: &LocalContext, day: u32, hour: u32) -> DateTime<Utc> {
    ctx.at(
        chrono::NaiveDate::from_ymd_opt(2026, 10, day).unwrap(),
        NaiveTime::from_hms_opt(hour, 0, 0).unwrap(),
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
    }
}

#[tokio::test]
async fn a_day_only_task_keeps_its_friday_deadline_and_is_placed_before_it() {
    let ctx = ctx();
    let drafts = capture_text(
        &FakeJudge,
        "buy milk tomorrow at 3 and call mom friday",
        Source::Text,
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(drafts.len(), 2, "{drafts:?}");

    // Thursday at 3 pm pins the first; "friday" alone is due by 17:00 that day.
    assert_eq!(drafts[0].when, When::At(local(&ctx, 15, 15)));
    assert_eq!(drafts[1].when, When::DueBy(local(&ctx, 16, 17)));

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

    // "Due by" means by, not on: the earliest free slot before the deadline is today.
    let call = planned
        .placements
        .iter()
        .find(|placement| placement.task.0 == "t1")
        .expect("call mom is placed");
    assert_eq!(call.start, local(&ctx, 14, 8));
    assert!(call.end <= local(&ctx, 16, 17));
}

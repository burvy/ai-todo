//! Deterministic task scheduling over recurring working hours and busy blocks.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use std::cmp::Ordering;
use std::collections::HashMap;

use todo_core::When;

const MAX_CHUNK_MINUTES: i64 = 120;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub duration_minutes: i64,
    pub deadline: Option<DateTime<Utc>>,
    pub priority: i32,
    pub position: i64,
    pub pinned_start: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BusyBlock {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeWindow {
    pub start: NaiveTime,
    pub end: NaiveTime,
}

/// The times a task may be placed, kept in the *local* timezone rather than
/// UTC.
///
/// Keeping these as UTC time-of-day loses the offset, so a window built on one
/// day is reused on days with a different offset and work drifts by an hour
/// across a DST change. Storing the zone alongside the windows lets each day
/// be resolved with the offset that is actually in force on that day.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkingHours {
    pub timezone: Tz,
    pub days: HashMap<Weekday, Vec<TimeWindow>>,
}

impl WorkingHours {
    pub fn every_day(timezone: Tz, start: NaiveTime, end: NaiveTime) -> Self {
        let mut days = HashMap::new();
        for day in [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ] {
            days.insert(day, vec![TimeWindow { start, end }]);
        }
        Self { timezone, days }
    }
}

impl Default for WorkingHours {
    fn default() -> Self {
        Self {
            timezone: chrono_tz::UTC,
            days: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledPlacement {
    pub task_id: String,
    pub title: String,
    pub chunk: usize,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnschedulableReason {
    InvalidDuration,
    DeadlinePassed,
    NoAvailableTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Placement {
    Scheduled(ScheduledPlacement),
    Unschedulable {
        task_id: String,
        reason: UnschedulableReason,
    },
}

/// Schedule tasks in deterministic order: deadline, priority, then input order.
/// Tasks longer than two hours are split into chunks of at most two hours.
pub fn schedule(
    tasks: &[Task],
    busy_blocks: &[BusyBlock],
    working_hours: &WorkingHours,
    now: DateTime<Utc>,
) -> Vec<Placement> {
    let mut order: Vec<usize> = (0..tasks.len()).collect();
    order.sort_by(|&a, &b| {
        let pinned_order = tasks[a]
            .pinned_start
            .is_none()
            .cmp(&tasks[b].pinned_start.is_none());
        if pinned_order != Ordering::Equal {
            return pinned_order;
        }
        let deadline_order = match (tasks[a].deadline, tasks[b].deadline) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        deadline_order
            .then_with(|| tasks[a].position.cmp(&tasks[b].position))
            .then_with(|| tasks[b].priority.cmp(&tasks[a].priority))
            .then(a.cmp(&b))
    });

    let mut occupied: Vec<(DateTime<Utc>, DateTime<Utc>)> = busy_blocks
        .iter()
        .filter(|block| block.end > block.start)
        .map(|block| (block.start, block.end))
        .collect();
    occupied.sort_by_key(|(start, _)| *start);

    let mut result = Vec::new();
    for index in order {
        let task = &tasks[index];
        if task.duration_minutes <= 0 {
            result.push(Placement::Unschedulable {
                task_id: task.id.clone(),
                reason: UnschedulableReason::InvalidDuration,
            });
            continue;
        }
        if task.deadline.is_some_and(|deadline| deadline <= now) {
            result.push(Placement::Unschedulable {
                task_id: task.id.clone(),
                reason: UnschedulableReason::DeadlinePassed,
            });
            continue;
        }

        let mut remaining = task.duration_minutes;
        let mut chunk = 1;
        let mut placements = Vec::new();
        if let Some(pinned) = task.pinned_start {
            let end = pinned + Duration::minutes(remaining);
            // A pinned appointment is an explicit user override: render it at
            // the requested instant for its full duration, even outside work
            // hours, over a busy block, or beyond its deadline.
            occupied.push((pinned, end));
            result.push(Placement::Scheduled(ScheduledPlacement {
                task_id: task.id.clone(),
                title: task.title.clone(),
                chunk: 1,
                start: pinned,
                end,
            }));
            continue;
        }
        while remaining > 0 {
            let minutes = remaining.min(MAX_CHUNK_MINUTES);
            let Some(start) = next_slot(now, minutes, task.deadline, working_hours, &occupied)
            else {
                break;
            };
            let end = start + Duration::minutes(minutes);
            occupied.push((start, end));
            occupied.sort_by_key(|(slot_start, _)| *slot_start);
            placements.push(Placement::Scheduled(ScheduledPlacement {
                task_id: task.id.clone(),
                title: task.title.clone(),
                chunk,
                start,
                end,
            }));
            remaining -= minutes;
            chunk += 1;
        }
        if remaining > 0 {
            result.push(Placement::Unschedulable {
                task_id: task.id.clone(),
                reason: UnschedulableReason::NoAvailableTime,
            });
        } else {
            result.extend(placements);
        }
    }
    result
}
/// Plan tasks into their week.
///
/// `At` tasks are fixed anchors: they are placed exactly at their start and
/// flexible tasks flow around them. `DueBy` tasks are placed in the earliest
/// free slot before their due time. `Anytime` tasks go in the earliest free
/// slot. Done tasks are skipped. No task is split into chunks.
///
/// Scheduling order is ascending `order`; input order breaks ties.
pub fn plan(
    tasks: &[todo_core::Task],
    hours: &WorkingHours,
    ctx: &todo_core::LocalContext,
) -> todo_core::Plan {
    let now = ctx.now;

    // Fixed anchors first so flexible tasks route around them. Their relative
    // order does not affect their own placement.
    let mut anchors: Vec<usize> = Vec::new();
    // Flexible tasks, sorted by `order` (input order breaks ties).
    let mut flexible: Vec<usize> = Vec::new();
    for (i, task) in tasks.iter().enumerate() {
        if task.done {
            continue;
        }
        match task.when {
            When::At(_) => anchors.push(i),
            When::DueBy(_) | When::Anytime => flexible.push(i),
        }
    }

    let mut occupied: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    let mut placements = Vec::new();
    let mut unplaced = Vec::new();

    for i in &anchors {
        let task = &tasks[*i];
        let When::At(start) = task.when else {
            unreachable!("An anchor always carries When::At");
        };
        let end = start + Duration::minutes(task.duration_min as i64);
        placements.push(todo_core::Placement {
            task: task.id.clone(),
            start,
            end,
        });
        occupied.push((start, end));
    }

    // Ascending order; the stable sort keeps input order for equal `order`.
    flexible.sort_by_key(|&i| tasks[i].order);

    for i in &flexible {
        let task = &tasks[*i];
        let deadline = match task.when {
            When::DueBy(d) => Some(d),
            When::Anytime => None,
            When::At(_) => unreachable!("Anchors are handled before flexible tasks"),
        };
        let start = next_slot(now, task.duration_min as i64, deadline, hours, &occupied);
        match start {
            Some(start) => {
                let end = start + Duration::minutes(task.duration_min as i64);
                placements.push(todo_core::Placement {
                    task: task.id.clone(),
                    start,
                    end,
                });
                occupied.push((start, end));
                occupied.sort_by_key(|(s, _)| *s);
            }
            None => unplaced.push(task.id.clone()),
        }
    }

    todo_core::Plan {
        placements,
        unplaced,
    }
}

fn next_slot(
    from: DateTime<Utc>,
    minutes: i64,
    deadline: Option<DateTime<Utc>>,
    hours: &WorkingHours,
    occupied: &[(DateTime<Utc>, DateTime<Utc>)],
) -> Option<DateTime<Utc>> {
    let timezone = hours.timezone;
    // Walk local dates, not UTC dates: the window belongs to a local day, and
    // near midnight the two disagree.
    let first_day = from.with_timezone(&timezone).date_naive();
    for day_offset in 0..366 {
        let local_date = first_day + Duration::days(day_offset);
        let Some(windows) = hours.days.get(&local_date.weekday()) else {
            continue;
        };
        for window in windows {
            let (Some(mut candidate), Some(window_end)) = (
                local_to_utc(timezone, local_date, window.start),
                local_to_utc(timezone, local_date, window.end),
            ) else {
                continue;
            };
            if window_end <= candidate {
                continue;
            }
            if candidate < from {
                candidate = from;
            }
            while candidate + Duration::minutes(minutes) <= window_end {
                let end = candidate + Duration::minutes(minutes);
                if deadline.is_some_and(|limit| end > limit) {
                    break;
                }
                if let Some((_, busy_end)) = occupied
                    .iter()
                    .filter(|(busy_start, busy_end)| *busy_start < end && *busy_end > candidate)
                    .min_by_key(|(busy_start, _)| *busy_start)
                {
                    candidate = *busy_end;
                    continue;
                }
                return Some(candidate);
            }
        }
    }
    None
}

/// Resolve one local wall-clock slot to the UTC instant it names on that date.
///
/// A spring-forward gap has no such instant, so it yields nothing and the slot
/// is simply skipped. A repeated hour is ambiguous; the first (pre-shift)
/// reading is used so a slot never lands twice.
fn local_to_utc(timezone: Tz, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    match timezone.from_local_datetime(&date.and_time(time)) {
        chrono::LocalResult::Single(instant) => Some(instant.with_timezone(&Utc)),
        chrono::LocalResult::Ambiguous(instant, _) => Some(instant.with_timezone(&Utc)),
        chrono::LocalResult::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 5, hour, 0, 0).unwrap()
    }
    fn hours() -> WorkingHours {
        WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        )
    }
    fn task(id: &str, duration_minutes: i64) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            duration_minutes,
            deadline: None,
            priority: 0,
            position: 0,
            pinned_start: None,
        }
    }

    #[test]
    fn avoids_busy_overlap() {
        let busy = [BusyBlock {
            start: at(9),
            end: at(10),
        }];
        let output = schedule(&[task("a", 60)], &busy, &hours(), at(8));
        assert_eq!(
            output[0],
            Placement::Scheduled(ScheduledPlacement {
                task_id: "a".into(),
                title: "a".into(),
                chunk: 1,
                start: at(10),
                end: at(11)
            })
        );
    }

    #[test]
    fn pinned_tasks_are_first_at_their_requested_time() {
        let mut pinned = task("pinned", 30);
        pinned.position = 99;
        pinned.pinned_start = Some(at(13));
        let mut normal = task("normal", 30);
        normal.position = 0;
        let output = schedule(&[normal, pinned], &[], &hours(), at(8));
        let Placement::Scheduled(first) = &output[0] else {
            panic!("expected pinned placement")
        };
        assert_eq!(first.task_id, "pinned");
        assert_eq!(first.start, at(13));
    }

    #[test]
    fn pinned_tasks_override_hours_busy_blocks_and_deadlines() {
        let mut pinned = task("pinned", 180);
        pinned.pinned_start = Some(at(16));
        pinned.deadline = Some(at(10));
        let output = schedule(
            &[pinned],
            &[BusyBlock {
                start: at(16),
                end: at(17),
            }],
            &hours(),
            at(8),
        );
        assert!(matches!(&output[0], Placement::Scheduled(p)
            if p.task_id == "pinned" && p.start == at(16) && p.end == at(19)));
    }

    #[test]
    fn orders_by_deadline_then_priority() {
        let mut low = task("low", 30);
        low.priority = 10;
        let mut early = task("early", 30);
        early.deadline = Some(at(12));
        let output = schedule(&[low, early], &[], &hours(), at(8));
        assert!(matches!(&output[0], Placement::Scheduled(p) if p.task_id == "early"));
    }

    #[test]
    fn splits_long_tasks() {
        let output = schedule(&[task("a", 300)], &[], &hours(), at(8));
        assert_eq!(output.len(), 3);
        assert!(matches!(&output[0], Placement::Scheduled(p) if p.chunk == 1 && p.start == at(9)));
        assert!(matches!(&output[2], Placement::Scheduled(p) if p.chunk == 3 && p.start == at(13)));
    }

    #[test]
    fn reports_missed_deadline_and_empty_calendar() {
        let mut missed = task("missed", 30);
        missed.deadline = Some(at(8));
        let empty = task("empty", 30);
        let output = schedule(&[missed, empty], &[], &WorkingHours::default(), at(8));
        assert!(output.contains(&Placement::Unschedulable {
            task_id: "missed".into(),
            reason: UnschedulableReason::DeadlinePassed
        }));
        assert!(output.contains(&Placement::Unschedulable {
            task_id: "empty".into(),
            reason: UnschedulableReason::NoAvailableTime
        }));
    }

    /// The live bug: a working window turned into a UTC time-of-day on the day the
    /// request arrives is reused for every later day, so across a DST change work
    /// lands an hour off. The task's own deadline is unaffected, which is why the
    /// task time looked right while the calendar time did not.
    #[test]
    fn working_window_stays_local_across_a_dst_change() {
        let local_hours = WorkingHours::every_day(
            chrono_tz::America::New_York,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        // US DST began 2026-03-08, so from the Monday after it New York is
        // UTC-4 and 09:00 local is 13:00 UTC, not the 14:00 that the winter
        // offset would have produced.
        let after_change = Utc.with_ymd_and_hms(2026, 3, 9, 8, 0, 0).unwrap();
        let output = schedule(&[task("a", 60)], &[], &local_hours, after_change);
        let Placement::Scheduled(placement) = &output[0] else {
            panic!("expected a placement, got {:?}", output);
        };
        assert_eq!(
            placement.start,
            Utc.with_ymd_and_hms(2026, 3, 9, 13, 0, 0).unwrap(),
            "09:00 New York on 2026-03-09 is 13:00 UTC"
        );
    }

    #[test]
    fn the_window_end_is_local_too_on_the_dst_day() {
        let local_hours = WorkingHours::every_day(
            chrono_tz::America::New_York,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        );
        let after_change = Utc.with_ymd_and_hms(2026, 3, 9, 8, 0, 0).unwrap();
        // Busy the morning away. The last hour of the local day has to still
        // fit before the 17:00 local close, which is 21:00 UTC on that date.
        let busy = [BusyBlock {
            start: Utc.with_ymd_and_hms(2026, 3, 9, 13, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 3, 9, 20, 0, 0).unwrap(),
        }];
        let output = schedule(&[task("b", 60)], &busy, &local_hours, after_change);
        let Placement::Scheduled(placement) = &output[0] else {
            panic!("expected a placement, got {:?}", output);
        };
        assert_eq!(
            placement.start,
            #[test]
            fn at_task_is_placed_exactly_at_its_start() {
                let hours = WorkingHours::every_day(chrono_tz::UTC, 9, 17);
                let at = Utc.with_ymd_and_hms(2026, 1, 5, 14, 0, 0).unwrap();
                let plan = plan(
                    &[v2_task("call", 30, todo_core::When::At(at), 0, false)],
                    &hours,
                    &v2_ctx(at),
                );
                assert_eq!(plan.placements.len(), 1);
                let placement = &plan.placements[0];
                assert_eq!(placement.task, todo_core::TaskId("call".into()));
                assert_eq!(placement.start, at);
                assert_eq!(
                    placement.end,
                    Utc.with_ymd_and_hms(2026, 1, 5, 14, 30, 0).unwrap()
                );
            },
            #[test]
            fn dueby_task_ends_before_due_when_there_is_room() {
                let hours = WorkingHours::every_day(chrono_tz::UTC, 9, 17);
                let due = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
                let now = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
                let plan = plan(
                    &[v2_task("run", 30, todo_core::When::DueBy(due), 0, false)],
                    &hours,
                    &v2_ctx(now),
                );
                assert_eq!(plan.placements.len(), 1);
                let placement = &plan.placements[0];
                assert!(placement.start < due);
                assert_eq!(placement.end, due);
                assert!(plan.unplaced.is_empty());
            },
            #[test]
            fn anytime_goes_in_first_free_slot() {
                let hours = WorkingHours::every_day(chrono_tz::UTC, 9, 17);
                let now = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
                let plan = plan(
                    &[v2_task("fold", 45, todo_core::When::Anytime, 0, false)],
                    &hours,
                    &v2_ctx(now),
                );
                assert_eq!(plan.placements.len(), 1);
                let placement = &plan.placements[0];
                assert_eq!(
                    placement.start,
                    Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap()
                );
                assert_eq!(
                    placement.end,
                    Utc.with_ymd_and_hms(2026, 1, 5, 9, 45, 0).unwrap()
                );
                assert!(plan.unplaced.is_empty());
            },
            fn v2_task(
                id: &str,
                duration_min: u16,
                when: todo_core::When,
                order: i64,
                done: bool,
            ) -> todo_core::Task {
                todo_core::Task {
                    id: todo_core::TaskId(id.into()),
                    title: id.into(),
                    duration_min,
                    when,
                    done,
                    order,
                    label: todo_core::Label::Personal,
                    priority: todo_core::Priority::Medium,
                    source: todo_core::Source::Text,
                    created: Utc.with_ymd_and_hms(2026, 10, 12, 8, 0, 0).unwrap(),
                    external_uid: None,
                }
            },
            fn v2_ctx(now: DateTime<Utc>) -> todo_core::LocalContext {
                todo_core::LocalContext {
                    now,
                    tz: chrono_tz::UTC,
                }
            },
            Utc.with_ymd_and_hms(2026, 3, 9, 20, 0, 0).unwrap(),
            "20:00 UTC is 16:00 New York on the DST day"
        );
        assert_eq!(
            placement.end,
            Utc.with_ymd_and_hms(2026, 3, 9, 21, 0, 0).unwrap(),
            "21:00 UTC is 17:00 New York, the close of the working day"
        );
    }

    /// A zone far enough east that the local working day wraps midnight in UTC.
    /// The window must still be found, and still start at 09:00 local.
    #[test]
    fn a_local_day_that_wraps_midnight_in_utc_is_still_found() {
        let timezone = chrono_tz::Pacific::Auckland;
        let local_hours = WorkingHours::every_day(
            timezone,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        // 2026-01-05 08:00 UTC is late evening on Monday the 5th in Auckland,
        // so Monday's working day is over and the next one is Tuesday.
        let monday_evening = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let output = schedule(&[task("a", 60)], &[], &local_hours, monday_evening);
        let Placement::Scheduled(placement) = &output[0] else {
            panic!("expected a placement, got {:?}", output);
        };
        let local_start = placement.start.with_timezone(&timezone);
        let local_end = placement.end.with_timezone(&timezone);
        assert_eq!(
            local_start.time(),
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            "work still starts at 09:00 local"
        );
        assert_eq!(local_end.time(), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
        assert_eq!(local_start.weekday(), Weekday::Tue, "the next local day");
    }
    #[test]
    fn done_tasks_are_never_placed() {
        let hours = WorkingHours::every_day(chrono_tz::UTC, 9, 17);
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let plan = plan(
            &[
                v2_task("done", 30, todo_core::When::Anytime, 0, true),
                v2_task("open", 30, todo_core::When::Anytime, 0, false),
            ],
            &hours,
            &v2_ctx(now),
        );
        assert_eq!(plan.placements.len(), 1);
        assert_eq!(plan.placements[0].task, todo_core::TaskId("open".into()));
    }

    #[test]
    fn tasks_are_not_split_into_chunks() {
        // One-hour window: a 120-minute task cannot fit, so it is left
        // unplaced instead of being split into chunks (the v1 behaviour).
        let hours = WorkingHours::every_day(chrono_tz::UTC, 9, 10);
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let plan = plan(
            &[v2_task("long", 120, todo_core::When::Anytime, 0, false)],
            &hours,
            &v2_ctx(now),
        );
        assert!(plan.placements.is_empty());
        assert_eq!(plan.unplaced.len(), 1);
    }

    #[test]
    fn flexible_tasks_are_scheduled_by_order() {
        let hours = WorkingHours::every_day(chrono_tz::UTC, 9, 17);
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let plan = plan(
            &[
                v2_task("later", 60, todo_core::When::Anytime, 1, false),
                v2_task("earlier", 60, todo_core::When::Anytime, 0, false),
            ],
            &hours,
            &v2_ctx(now),
        );
        assert_eq!(plan.placements.len(), 2);
        assert_eq!(plan.placements[0].task, todo_core::TaskId("earlier".into()));
        assert_eq!(plan.placements[1].task, todo_core::TaskId("later".into()));
    }

    #[test]
    fn equal_order_ties_break_by_input_order() {
        let hours = WorkingHours::every_day(chrono_tz::UTC, 9, 17);
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let plan = plan(
            &[
                v2_task("second", 30, todo_core::When::Anytime, 0, false),
                v2_task("first", 30, todo_core::When::Anytime, 0, false),
            ],
            &hours,
            &v2_ctx(now),
        );
        assert_eq!(plan.placements.len(), 2);
        assert_eq!(plan.placements[0].task, todo_core::TaskId("second".into()));
        assert_eq!(plan.placements[1].task, todo_core::TaskId("first".into()));
    }

    #[test]
    fn local_working_hours_are_resolved_per_local_day_across_dst() {
        // DST began 2026-03-08 in New York; from 2026-03-09 New York is UTC-4,
        // so 09:00 local is 13:00 UTC, not 14:00 from a fixed winter offset.
        let hours = WorkingHours::every_day(
            chrono_tz::America::New_York,
            chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            chrono::NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        let now = Utc.with_ymd_and_hms(2026, 3, 9, 8, 0, 0).unwrap();
        let at = Utc.with_ymd_and_hms(2026, 3, 9, 9, 0, 0).unwrap();
        let plan = plan(
            &[v2_task("work", 60, todo_core::When::At(at), 0, false)],
            &hours,
            &v2_ctx(now),
        );
        assert_eq!(plan.placements.len(), 1);
        let placement = &plan.placements[0];
        assert_eq!(
            placement.start,
            Utc.with_ymd_and_hms(2026, 3, 9, 13, 0, 0).unwrap()
        );
        assert_eq!(
            placement.end,
            Utc.with_ymd_and_hms(2026, 3, 9, 14, 0, 0).unwrap()
        );
    }

    proptest::proptest! {
        #[test]
        fn flexible_never_overlap_at_or_each_other(
            at_hour in 9u32..17,
            at_dur in 15u16..120,
            flex_count in 0u32..4,
            flex_dur in 15u16..120,
            due_delta in 1u32..24,
        ) {
            let hours = WorkingHours::every_day(
                chrono_tz::UTC,
                chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                chrono::NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            );
            let now = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
            let at_start = Utc.with_ymd_and_hms(2026, 1, 5, at_hour, 0, 0).unwrap();

            let mut tasks = vec![v2_task(
                "at",
                at_dur,
                todo_core::When::At(at_start),
                0,
                false,
            )];
            for i in 0..flex_count {
                let when = if i % 2 == 0 {
                    todo_core::When::Anytime
                } else {
                    let due = at_start + chrono::Duration::minutes((due_delta * 60) as i64);
                    todo_core::When::DueBy(due)
                };
                tasks.push(v2_task(
                    &format!("flex{}", i),
                    flex_dur,
                    when,
                    i as i64,
                    false,
                ));
            }

            let plan = plan(
                &tasks,
                &hours,
                &todo_core::LocalContext { now, tz: chrono_tz::UTC },
            );

            // Every At task is placed exactly at its start with the right end.
            let mut at_intervals: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
            for task in &tasks {
                if let todo_core::When::At(start) = task.when {
                    let placement = plan
                        .placements
                        .iter()
                        .find(|p| p.task == task.id)
                        .expect("At task should be placed");
                    assert_eq!(placement.start, *start);
                    assert_eq!(
                        placement.end,
                        start + chrono::Duration::minutes(task.duration_min as i64)
                    );
                    at_intervals.push((placement.start, placement.end));
                }
            }

            // No task is split across multiple placements.
            let mut seen = std::collections::HashSet::new();
            for p in &plan.placements {
                assert!(
                    seen.insert(p.task.clone()),
                    "task appears in more than one placement"
                );
            }

            // DueBy tasks end by their due time whenever they are placed.
            for task in &tasks {
                if let todo_core::When::DueBy(due) = task.when {
                    if let Some(p) = plan.placements.iter().find(|p| p.task == task.id) {
                        assert!(
                            p.end <= *due,
                            "DueBy task ends after due: {:?} > {:?}",
                            p.end, due
                        );
                    }
                }
            }

            // Flexible tasks never overlap At tasks or each other.
            let mut occupied: Vec<(DateTime<Utc>, DateTime<Utc>)> = at_intervals;
            for p in &plan.placements {
                let is_at = tasks
                    .iter()
                    .any(|t| t.id == p.task && matches!(t.when, todo_core::When::At(_)));
                if is_at {
                    continue;
                }
                for (s, e) in &occupied {
                    assert!(
                        !(p.start < *e && p.end > *s),
                        "flexible placement {:?} overlaps occupied interval ({:?}, {:?})",
                        p, s, e
                    );
                }
                occupied.push((p.start, p.end));
            }
        }
    }
}

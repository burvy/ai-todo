//! Deterministic task scheduling over recurring working hours and busy blocks.

use chrono::{DateTime, Datelike, Duration, NaiveTime, Utc, Weekday};
use std::cmp::Ordering;
use std::collections::HashMap;

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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkingHours {
    pub days: HashMap<Weekday, Vec<TimeWindow>>,
}

impl WorkingHours {
    pub fn every_day(start: NaiveTime, end: NaiveTime) -> Self {
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
        Self { days }
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

fn next_slot(
    from: DateTime<Utc>,
    minutes: i64,
    deadline: Option<DateTime<Utc>>,
    hours: &WorkingHours,
    occupied: &[(DateTime<Utc>, DateTime<Utc>)],
) -> Option<DateTime<Utc>> {
    for day_offset in 0..366 {
        let date = from.date_naive() + Duration::days(day_offset);
        let Some(windows) = hours.days.get(&date.weekday()) else {
            continue;
        };
        for window in windows {
            let mut candidate =
                DateTime::<Utc>::from_naive_utc_and_offset(date.and_time(window.start), Utc);
            let window_end =
                DateTime::<Utc>::from_naive_utc_and_offset(date.and_time(window.end), Utc);
            if window.end <= window.start {
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 5, hour, 0, 0).unwrap()
    }
    fn hours() -> WorkingHours {
        WorkingHours::every_day(
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
}

//! Deterministic scheduling over local working hours.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use std::cmp::Ordering;
use std::collections::HashMap;
use todo_core::When;

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkingHours {
    pub timezone: Tz,
    pub days: HashMap<Weekday, Vec<TimeWindow>>,
}

impl WorkingHours {
    pub fn every_day(timezone: Tz, start: NaiveTime, end: NaiveTime) -> Self {
        let days = Weekday::iter()
            .map(|day| (day, vec![TimeWindow { start, end }]))
            .collect();
        Self { timezone, days }
    }
}

impl Default for WorkingHours {
    fn default() -> Self {
        Self::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
        )
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

/// Legacy scheduler API retained for the v1 server.
pub fn schedule(
    tasks: &[Task],
    busy_blocks: &[BusyBlock],
    working_hours: &WorkingHours,
    now: DateTime<Utc>,
) -> Vec<Placement> {
    let mut order: Vec<usize> = (0..tasks.len()).collect();
    order.sort_by(|&a, &b| {
        tasks[a]
            .pinned_start
            .is_none()
            .cmp(&tasks[b].pinned_start.is_none())
            .then_with(|| match (tasks[a].deadline, tasks[b].deadline) {
                (Some(left), Some(right)) => left.cmp(&right),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
            .then_with(|| tasks[a].position.cmp(&tasks[b].position))
            .then_with(|| tasks[b].priority.cmp(&tasks[a].priority))
            .then_with(|| a.cmp(&b))
    });

    let mut occupied: Vec<_> = busy_blocks
        .iter()
        .filter(|block| block.end > block.start)
        .map(|block| (block.start, block.end))
        .collect();
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
        if let Some(start) = task.pinned_start {
            let end = start + Duration::minutes(task.duration_minutes);
            occupied.push((start, end));
            result.push(Placement::Scheduled(ScheduledPlacement {
                task_id: task.id.clone(),
                title: task.title.clone(),
                chunk: 1,
                start,
                end,
            }));
            continue;
        }
        let mut remaining = task.duration_minutes;
        let mut chunk = 1;
        let mut chunks = Vec::new();
        while remaining > 0 {
            let minutes = remaining.min(120);
            let Some(start) = next_slot(now, minutes, task.deadline, working_hours, &occupied)
            else {
                break;
            };
            let end = start + Duration::minutes(minutes);
            occupied.push((start, end));
            chunks.push(Placement::Scheduled(ScheduledPlacement {
                task_id: task.id.clone(),
                title: task.title.clone(),
                chunk,
                start,
                end,
            }));
            remaining -= minutes;
            chunk += 1;
        }
        if remaining == 0 {
            result.extend(chunks);
        } else {
            result.push(Placement::Unschedulable {
                task_id: task.id.clone(),
                reason: UnschedulableReason::NoAvailableTime,
            });
        }
    }
    result
}

/// Place fixed tasks exactly and fill flexible tasks in list order.
pub fn plan(
    tasks: &[todo_core::Task],
    hours: &WorkingHours,
    ctx: &todo_core::LocalContext,
) -> todo_core::Plan {
    let mut occupied = Vec::new();
    let mut placements = Vec::new();
    let mut flexible = Vec::new();

    for (index, task) in tasks.iter().enumerate() {
        if task.done {
            continue;
        }
        match task.when {
            When::At(start) => {
                let end = start + Duration::minutes(i64::from(task.duration_min));
                placements.push(todo_core::Placement {
                    task: task.id.clone(),
                    start,
                    end,
                });
                occupied.push((start, end));
            }
            When::DueBy(_) | When::Anytime => flexible.push(index),
        }
    }
    flexible.sort_by_key(|&index| (tasks[index].order, index));

    let mut unplaced = Vec::new();
    for index in flexible {
        let task = &tasks[index];
        let deadline = match task.when {
            When::DueBy(deadline) => Some(deadline),
            When::Anytime => None,
            When::At(_) => unreachable!(),
        };
        let Some(start) = next_slot(
            ctx.now,
            i64::from(task.duration_min),
            deadline,
            hours,
            &occupied,
        ) else {
            unplaced.push(task.id.clone());
            continue;
        };
        let end = start + Duration::minutes(i64::from(task.duration_min));
        placements.push(todo_core::Placement {
            task: task.id.clone(),
            start,
            end,
        });
        occupied.push((start, end));
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
    if minutes <= 0 {
        return None;
    }
    let first_day = from.with_timezone(&hours.timezone).date_naive();
    for offset in 0..366 {
        let date = first_day + Duration::days(offset);
        for window in hours.days.get(&date.weekday()).into_iter().flatten() {
            let (Some(window_start), Some(window_end)) = (
                local_to_utc(hours.timezone, date, window.start),
                local_to_utc(hours.timezone, date, window.end),
            ) else {
                continue;
            };
            let mut candidate = window_start.max(from);
            while candidate + Duration::minutes(minutes) <= window_end {
                let end = candidate + Duration::minutes(minutes);
                if deadline.is_some_and(|limit| end > limit) {
                    break;
                }
                if let Some((_, busy_end)) = occupied
                    .iter()
                    .filter(|(start, end)| {
                        *start < candidate + Duration::minutes(minutes) && *end > candidate
                    })
                    .min_by_key(|(start, _)| *start)
                {
                    candidate = (*busy_end).max(candidate + Duration::seconds(1));
                } else {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn local_to_utc(timezone: Tz, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    match timezone.from_local_datetime(&date.and_time(time)) {
        chrono::LocalResult::Single(value) | chrono::LocalResult::Ambiguous(value, _) => {
            Some(value.with_timezone(&Utc))
        }
        chrono::LocalResult::None => None,
    }
}

trait WeekdayIter {
    fn iter() -> std::array::IntoIter<Weekday, 7>;
}

impl WeekdayIter for Weekday {
    fn iter() -> std::array::IntoIter<Weekday, 7> {
        [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ]
        .into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    fn ctx(now: DateTime<Utc>, tz: Tz) -> todo_core::LocalContext {
        todo_core::LocalContext { now, tz }
    }

    fn task(id: &str, duration_min: u16, when: When, order: i64) -> todo_core::Task {
        todo_core::Task {
            id: todo_core::TaskId(id.into()),
            title: id.into(),
            duration_min,
            when,
            done: false,
            order,
            label: todo_core::Label::Personal,
            priority: todo_core::Priority::Medium,
            source: todo_core::Source::Text,
            created: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            external_uid: None,
        }
    }

    #[test]
    fn fixed_task_is_unchanged_and_flexible_tasks_avoid_it() {
        let at = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        );
        let plan = plan(
            &[
                task("fixed", 60, When::At(at), 0),
                task("flex", 60, When::Anytime, 1),
            ],
            &hours,
            &ctx(
                Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap(),
                chrono_tz::UTC,
            ),
        );
        assert!(
            plan.placements
                .iter()
                .any(|p| p.task.0 == "fixed" && p.start == at)
        );
        assert!(
            plan.placements
                .iter()
                .any(|p| p.task.0 == "flex" && p.start.hour() == 8)
        );
    }

    #[test]
    fn due_by_ends_at_deadline_when_room_exists() {
        let due = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        );
        let result = plan(
            &[task("due", 60, When::DueBy(due), 0)],
            &hours,
            &ctx(
                Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap(),
                chrono_tz::UTC,
            ),
        );
        assert!(result.placements[0].start < due);
        assert!(result.placements[0].end <= due);
    }

    #[test]
    fn impossible_flexible_task_is_unplaced() {
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        );
        let result = plan(
            &[task("long", 120, When::Anytime, 0)],
            &hours,
            &ctx(
                Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap(),
                chrono_tz::UTC,
            ),
        );
        assert_eq!(result.unplaced, vec![todo_core::TaskId("long".into())]);
    }

    #[test]
    fn daylight_saving_changes_offset_per_local_day() {
        let tz = chrono_tz::America::New_York;
        let hours = WorkingHours::every_day(
            tz,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let mut hours = hours;
        hours.days.remove(&Weekday::Sat);
        hours.days.remove(&Weekday::Sun);
        let now = Utc.with_ymd_and_hms(2026, 3, 8, 14, 0, 0).unwrap();
        let result = plan(
            &[task("later", 60, When::Anytime, 0)],
            &hours,
            &ctx(now, tz),
        );
        assert_eq!(
            result.placements[0].start,
            Utc.with_ymd_and_hms(2026, 3, 9, 13, 0, 0).unwrap()
        );
    }
}

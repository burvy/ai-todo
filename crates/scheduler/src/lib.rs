//! Deterministic task scheduling over recurring working hours and busy blocks.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
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
}

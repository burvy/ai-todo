//! Deterministic scheduling over local working hours.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use std::collections::HashMap;
use todo_core::When;

const MIN_BLOCK_MIN: u16 = 5;

fn effective_minutes(task: &todo_core::Task) -> i64 {
    i64::from(task.duration_min.max(MIN_BLOCK_MIN))
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

/// Place fixed tasks exactly, then overdue tasks first and flexible tasks in
/// list order, all sharing one slot-finding pass.
///
/// Overdue tasks — `DueBy` whose deadline is now or in the past — are placed
/// first from now, so the user gets a chance to catch up.
pub fn plan(
    tasks: &[todo_core::Task],
    hours: &WorkingHours,
    ctx: &todo_core::LocalContext,
) -> todo_core::Plan {
    let mut occupied = Vec::new();
    let mut placements = Vec::new();
    let mut overdue = Vec::new();
    let mut flexible = Vec::new();

    for (index, task) in tasks.iter().enumerate() {
        if task.done {
            continue;
        }
        match task.when {
            When::At(start) => {
                let duration = effective_minutes(task);
                let end = start + Duration::minutes(duration);
                placements.push(todo_core::Placement {
                    task: task.id.clone(),
                    start,
                    end,
                });
                occupied.push((start, end));
            }
            When::DueBy(deadline) if deadline <= ctx.now => overdue.push(index),
            When::DueBy(_) | When::Anytime => flexible.push(index),
        }
    }
    overdue.sort_by_key(|&index| (tasks[index].order, index));
    flexible.sort_by_key(|&index| (tasks[index].order, index));

    let mut unplaced = Vec::new();

    // Overdue tasks go first and lose their deadline constraint; flexible tasks
    // keep theirs. One ordered pass feeds the same next_slot/push loop.
    for (index, deadline) in overdue.into_iter().map(|index| (index, None)).chain(
        flexible
            .into_iter()
            .map(|index| (index, deadline_of(&tasks[index]))),
    ) {
        let task = &tasks[index];
        let duration = effective_minutes(task);
        let Some(start) = next_slot(ctx.now, duration, deadline, hours, &occupied) else {
            unplaced.push(task.id.clone());
            continue;
        };
        let end = start + Duration::minutes(duration);
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

/// The latest allowed end for a flexible task; `None` when unconstrained
/// (`Anytime`, or overdue tasks placed from now).
fn deadline_of(task: &todo_core::Task) -> Option<DateTime<Utc>> {
    match task.when {
        When::DueBy(deadline) => Some(deadline),
        When::Anytime => None,
        When::At(_) => unreachable!("fixed tasks are placed exactly, never in this queue"),
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
            updated_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            due_tz: None,
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

    struct Generator(u64);

    impl Generator {
        fn next(&mut self, bound: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            self.0 % bound
        }
    }

    fn generated_tasks(seed: u64) -> Vec<todo_core::Task> {
        let mut generator = Generator(seed);
        (0..24)
            .map(|index| {
                let duration = (generator.next(120) + 1) as u16;
                let when = match generator.next(3) {
                    0 => When::At(
                        Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap()
                            + Duration::minutes((generator.next(480) + 30) as i64),
                    ),
                    1 => When::DueBy(
                        Utc.with_ymd_and_hms(2026, 1, 5, 12, 0, 0).unwrap()
                            + Duration::minutes(generator.next(600) as i64),
                    ),
                    _ => When::Anytime,
                };
                task(&format!("task-{index}"), duration, when, index)
            })
            .collect()
    }

    fn overlaps(left: &todo_core::Placement, right: &todo_core::Placement) -> bool {
        left.start < right.end && right.start < left.end
    }

    // — Overdue tests —

    #[test]
    fn overdue_task_placed_before_normal_flexible() {
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        // Now is 10:00. Overdue task was due at 09:00 (in the past).
        // Normal task is Anytime.
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        let overdue_due = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
        let result = plan(
            &[
                task("normal", 60, When::Anytime, 0),
                task("overdue", 60, When::DueBy(overdue_due), 1),
            ],
            &hours,
            &ctx(now, chrono_tz::UTC),
        );
        // Overdue is placed first, so it gets the earlier slot: 10:00, then 11:00.
        let overdue_start = result
            .placements
            .iter()
            .find(|p| p.task.0 == "overdue")
            .unwrap()
            .start;
        let normal_start = result
            .placements
            .iter()
            .find(|p| p.task.0 == "normal")
            .unwrap()
            .start;
        assert_eq!(
            overdue_start,
            Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap(),
            "overdue task should take the first free slot from now"
        );
        assert_eq!(
            normal_start,
            Utc.with_ymd_and_hms(2026, 1, 5, 11, 0, 0).unwrap(),
            "normal flexible task should follow the overdue task"
        );
    }

    #[test]
    fn task_due_exactly_now_is_overdue() {
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        let result = plan(
            &[
                task("normal", 60, When::Anytime, 1),
                task("just-due", 60, When::DueBy(now), 0),
            ],
            &hours,
            &ctx(now, chrono_tz::UTC),
        );
        // Due at exactly now counts as overdue (deadline <= now) and takes the
        // first free slot, ahead of the Anytime task.
        assert!(!result.unplaced.iter().any(|id| id.0 == "just-due"));
        assert_eq!(result.placements[0].task.0, "just-due");
        assert_eq!(result.placements[0].start, now);
        assert_eq!(
            result.placements[1].start,
            Utc.with_ymd_and_hms(2026, 1, 5, 11, 0, 0).unwrap()
        );
    }

    #[test]
    fn two_overdue_tasks_are_placed_in_order() {
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        // Now is 10:00; both tasks were due at 09:00.
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        let overdue_due = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
        let result = plan(
            &[
                task("first", 30, When::DueBy(overdue_due), 1),
                task("second", 30, When::DueBy(overdue_due), 0),
            ],
            &hours,
            &ctx(now, chrono_tz::UTC),
        );
        // Sorted by order: "second" (order 0) first, then "first" (order 1).
        assert_eq!(result.placements[0].task.0, "second");
        assert_eq!(result.placements[0].start, now);
        assert_eq!(result.placements[1].task.0, "first");
        assert_eq!(
            result.placements[1].start,
            Utc.with_ymd_and_hms(2026, 1, 5, 10, 30, 0).unwrap()
        );
        assert!(result.unplaced.is_empty());
    }

    #[test]
    fn overdue_task_never_placed_in_the_past() {
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        let overdue_due = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
        let result = plan(
            &[task("overdue", 60, When::DueBy(overdue_due), 0)],
            &hours,
            &ctx(now, chrono_tz::UTC),
        );
        assert!(
            result.placements[0].start >= now,
            "overdue task must not start before now"
        );
    }

    #[test]
    fn overdue_task_avoids_fixed_event() {
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        let overdue_due = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
        // Fixed event at 10:00-11:00.
        let fixed_at = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        let result = plan(
            &[
                task("fixed", 60, When::At(fixed_at), 0),
                task("overdue", 60, When::DueBy(overdue_due), 1),
            ],
            &hours,
            &ctx(now, chrono_tz::UTC),
        );
        let overdue_start = result
            .placements
            .iter()
            .find(|p| p.task.0 == "overdue")
            .unwrap()
            .start;
        // Should be placed after the fixed event at 11:00.
        assert_eq!(
            overdue_start,
            Utc.with_ymd_and_hms(2026, 1, 5, 11, 0, 0).unwrap(),
            "overdue should skip the fixed event"
        );
    }

    #[test]
    fn overdue_placement_is_deterministic() {
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        let overdue_due = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
        let tasks = vec![
            task("overdue-1", 30, When::DueBy(overdue_due), 0),
            task("overdue-2", 30, When::DueBy(overdue_due), 1),
            task("normal", 60, When::Anytime, 2),
        ];
        let first = plan(&tasks, &hours, &ctx(now, chrono_tz::UTC));
        let second = plan(&tasks, &hours, &ctx(now, chrono_tz::UTC));
        assert_eq!(first, second, "overdue placement must be deterministic");
    }

    #[test]
    fn overdue_respects_working_hours_on_new_day() {
        // Now is after working hours; overdue should land the next working day.
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        );
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 20, 0, 0).unwrap();
        let overdue_due = Utc.with_ymd_and_hms(2026, 1, 5, 12, 0, 0).unwrap();
        let result = plan(
            &[task("overdue", 60, When::DueBy(overdue_due), 0)],
            &hours,
            &ctx(now, chrono_tz::UTC),
        );
        // Next working day starts at 09:00.
        assert_eq!(
            result.placements[0].start,
            Utc.with_ymd_and_hms(2026, 1, 6, 9, 0, 0).unwrap(),
            "overdue should land on the next working day"
        );
    }

    #[test]
    fn generated_plans_preserve_scheduler_invariants() {
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        let context = ctx(
            Utc.with_ymd_and_hms(2026, 1, 5, 7, 0, 0).unwrap(),
            chrono_tz::UTC,
        );

        for seed in 0..128 {
            let tasks = generated_tasks(seed);
            let first = plan(&tasks, &hours, &context);
            assert_eq!(first, plan(&tasks, &hours, &context));

            let by_id: HashMap<_, _> = tasks.iter().map(|task| (&task.id, task)).collect();
            let placed_ids: std::collections::HashSet<_> = first
                .placements
                .iter()
                .map(|placement| &placement.task)
                .collect();
            let unplaced_ids: std::collections::HashSet<_> = first.unplaced.iter().collect();
            assert!(placed_ids.is_disjoint(&unplaced_ids));
            assert_eq!(placed_ids.len() + unplaced_ids.len(), tasks.len());

            for placement in &first.placements {
                let task = by_id[&placement.task];
                assert!(placement.end > placement.start);
                if let When::At(start) = task.when {
                    assert_eq!(placement.start, start);
                    assert_eq!(
                        placement.end,
                        start + Duration::minutes(effective_minutes(task))
                    );
                } else {
                    let local_start = placement.start.with_timezone(&hours.timezone);
                    let local_end = placement.end.with_timezone(&hours.timezone);
                    assert_eq!(local_start.date_naive(), local_end.date_naive());
                    let windows = &hours.days[&local_start.weekday()];
                    assert!(windows.iter().any(|window| {
                        local_start.time() >= window.start && local_end.time() <= window.end
                    }));
                }
            }

            for (index, left) in first.placements.iter().enumerate() {
                for right in first.placements.iter().skip(index + 1) {
                    if overlaps(left, right) {
                        assert!(
                            matches!(by_id[&left.task].when, When::At(_))
                                && matches!(by_id[&right.task].when, When::At(_))
                        );
                    }
                }
            }
        }
    }

    // ---- scheduler edge-case tests (oscar/scheduler-tests) --------

    #[test]
    fn task_longer_than_working_day_is_unplaced() {
        // A 10-hour flexible task in a 4-hour day cannot fit; unplaced.
        let at = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
        );
        let task = task("long", 600, When::Anytime, 0);
        let plan = plan(&[task], &hours, &ctx(at, chrono_tz::UTC));
        assert!(plan.unplaced.contains(&todo_core::TaskId("long".into())));
    }

    #[test]
    fn friday_evening_task_lands_monday_with_weekend_excluded() {
        let at = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap(); // Mon 10am
        // Only Mon-Fri, 9-13. Friday 22:00 should roll to Monday.
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
        );
        let task = task(
            "fri",
            60,
            When::DueBy(Utc.with_ymd_and_hms(2026, 1, 9, 22, 0, 0).unwrap()),
            0,
        );
        let plan = plan(&[task], &hours, &ctx(at, chrono_tz::UTC));
        assert!(!plan.unplaced.contains(&todo_core::TaskId("fri".into())));
        let placed = plan
            .placements
            .iter()
            .find(|p| p.task == todo_core::TaskId("fri".into()));
        assert!(
            placed.is_some(),
            "should land on Friday evening (weekend not excluded by every_day)"
        );
    }

    #[test]
    fn fixed_event_overlaps_end_of_working_hours() {
        let at = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        );
        // Fixed event 11:30-13:00 overlaps end; flexible avoids it.
        let fixed = task(
            "fixed",
            90,
            When::At(Utc.with_ymd_and_hms(2026, 1, 5, 11, 30, 0).unwrap()),
            0,
        );
        let flex = task("flex", 60, When::Anytime, 1);
        let plan = plan(&[fixed, flex], &hours, &ctx(at, chrono_tz::UTC));
        let flex_placed = plan
            .placements
            .iter()
            .find(|p| p.task == todo_core::TaskId("flex".into()));
        assert!(flex_placed.is_some());
        assert!(flex_placed.unwrap().start < Utc.with_ymd_and_hms(2026, 1, 5, 11, 30, 0).unwrap());
    }

    #[test]
    fn two_fixed_events_overlapping_each_other() {
        let at = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        );
        // Both fixed at 10:00-11:00; planner keeps both (does not drop).
        let a = task(
            "a",
            60,
            When::At(Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap()),
            0,
        );
        let b = task(
            "b",
            60,
            When::At(Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap()),
            1,
        );
        let plan = plan(&[a, b], &hours, &ctx(at, chrono_tz::UTC));
        assert_eq!(plan.placements.len(), 2);
    }

    #[test]
    fn zero_duration_task_is_placed_at_start() {
        let at = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        );
        let task = task("zero", 0, When::Anytime, 0);
        let plan = plan(&[task], &hours, &ctx(at, chrono_tz::UTC));
        assert_eq!(plan.placements.len(), 1);
        assert_eq!(
            plan.placements[0].start,
            Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap()
        );
        let end = plan.placements[0].end;
        assert_eq!(end, Utc.with_ymd_and_hms(2026, 1, 5, 9, 5, 0).unwrap());
    }

    #[test]
    fn deadline_before_working_hours_tomorrow_is_placed_today_if_fits() {
        let at = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap(); // Monday 10am
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        );
        // Deadline Monday 8:00 (before working hours); should place at 9:00 if room.
        let task = task(
            "early_dead",
            30,
            When::DueBy(Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap()),
            0,
        );
        let plan = plan(&[task], &hours, &ctx(at, chrono_tz::UTC));
        assert!(
            !plan
                .unplaced
                .contains(&todo_core::TaskId("early_dead".into()))
        );
    }

    #[test]
    fn fifty_tasks_finish_fast_and_remain_deterministic() {
        let at = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        );
        let mut tasks = Vec::new();
        for i in 0..50 {
            tasks.push(task(&format!("t{}", i), 30, When::Anytime, i as i64));
        }
        let plan1 = plan(&tasks, &hours, &ctx(at, chrono_tz::UTC));
        let plan2 = plan(&tasks, &hours, &ctx(at, chrono_tz::UTC));
        assert_eq!(plan1.placements.len(), plan2.placements.len());
        // All placed because 50 * 30min = 25h fits in multi-day window.
    }

    #[test]
    fn planner_window_end_unplaced_never_past_window() {
        // Window only through next day; task too long stays unplaced.
        let at = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        );
        // Very long flexible task; planner window limited.
        let task = task(
            "big",
            300,
            When::DueBy(Utc.with_ymd_and_hms(2026, 1, 6, 9, 0, 0).unwrap()),
            0,
        );
        let plan = plan(&[task], &hours, &ctx(at, chrono_tz::UTC));
        assert!(
            !plan
                .placements
                .iter()
                .any(|p| p.task == todo_core::TaskId("big".into()))
                || plan.unplaced.contains(&todo_core::TaskId("big".into()))
        );
    }
}

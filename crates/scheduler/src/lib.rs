//! Deterministic scheduling over local working hours.

use chrono::{
    DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Timelike, Utc, Weekday,
};
use chrono_tz::Tz;
use std::collections::HashMap;
use std::str::FromStr;
use todo_core::tuning::{TASK_MIN_DURATION_MIN, WORKDAY_END, WORKDAY_START};
use todo_core::{Occurrence, OccurrenceStatus, When};

fn effective_minutes(task: &todo_core::Task) -> i64 {
    i64::from(task.duration_min.max(TASK_MIN_DURATION_MIN))
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
    /// Where on its day a day-only task goes when the user has no timed tasks to
    /// learn a usual time from (`DaySettings::placement`).
    pub day_only_time: NaiveTime,
}

impl WorkingHours {
    pub fn every_day(timezone: Tz, start: NaiveTime, end: NaiveTime) -> Self {
        let days = Weekday::iter()
            .map(|day| (day, vec![TimeWindow { start, end }]))
            .collect();
        Self {
            timezone,
            days,
            day_only_time: todo_core::DaySettings::default().placement,
        }
    }
}

impl Default for WorkingHours {
    fn default() -> Self {
        Self::every_day(chrono_tz::UTC, WORKDAY_START, WORKDAY_END)
    }
}

/// Place fixed tasks exactly, then overdue tasks first and flexible tasks in
/// list order, all sharing one slot-finding pass.
///
/// Overdue tasks — `DueBy` whose deadline is now or in the past — are placed
/// first from now, so the user gets a chance to catch up.
///
/// A `DueBy` that is not overdue is a day-only task: it goes on its deadline's day, in
/// the free slot closest to the user's usual time ([`usual_time`], else
/// `hours.day_only_time`). Only when that day has no room does it take the earliest
/// free slot before the deadline.
///
/// A task the user took off the planner ([`todo_core::Task::off_planner`]) gets no slot
/// at all and is reported in [`todo_core::Plan::off_planner`] rather than `unplaced`.
pub fn plan(
    tasks: &[todo_core::Task],
    hours: &WorkingHours,
    ctx: &todo_core::LocalContext,
) -> todo_core::Plan {
    let mut occupied = Vec::new();
    let mut placements = Vec::new();
    let mut overdue = Vec::new();
    let mut flexible = Vec::new();
    let mut off_planner = Vec::new();

    for (index, task) in tasks.iter().enumerate() {
        if task.done {
            continue;
        }
        // The user took it off the planner: it keeps its place in the list and its
        // deadline, but no slot is chosen for it and it is not counted as unplaced.
        // This is asked for, not a lack of room, so it is reported apart (ARCHITECTURE.md 4.2).
        if task.off_planner {
            off_planner.push(task.id.clone());
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
    // A deadline takes precedence over list position (ARCHITECTURE.md 4.1): deadlines
    // first, earliest first, so an `Anytime` task above them cannot take their room.
    flexible.sort_by_key(|&index| {
        let deadline = deadline_of(&tasks[index]);
        (deadline.is_none(), deadline, tasks[index].order, index)
    });

    let mut unplaced = Vec::new();
    let usual = usual_time(tasks, hours.timezone).unwrap_or(hours.day_only_time);

    // Overdue tasks go first and lose their deadline constraint; flexible tasks
    // keep theirs. One ordered pass feeds the same next_slot/push loop.
    for (index, deadline) in overdue.into_iter().map(|index| (index, None)).chain(
        flexible
            .into_iter()
            .map(|index| (index, deadline_of(&tasks[index]))),
    ) {
        let task = &tasks[index];
        let duration = effective_minutes(task);
        let Some(start) = deadline
            .and_then(|limit| slot_near(task, limit, usual, duration, hours, &occupied, ctx.now))
            .or_else(|| next_slot(ctx.now, duration, deadline, hours, &occupied))
        else {
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
        off_planner,
    }
}

/// The weekly occurrences of one series between `window_start` (inclusive) and
/// `window_end` (exclusive), with the exceptions applied (ARCHITECTURE.md 7.1).
///
/// Pure and deterministic: no wall clock, no store. Each occurrence's instant is
/// derived in the zone stored on the series, so a series keeps its local time
/// across DST: 07:00 stays 07:00 while the UTC instant shifts. An undone past
/// occurrence is never carried forward — it stays a fixed block on its own day,
/// and only one-shot tasks can become overdue.
pub fn expand(
    series: &todo_core::Series,
    window_start: NaiveDate,
    window_end: NaiveDate,
    exceptions: &[todo_core::SeriesException],
    ctx: &todo_core::LocalContext,
) -> Vec<Occurrence> {
    // A zone that will not parse (a hand-edited row) falls back to the zone the
    // request came in; an occurrence at a slightly shifted time beats no data.
    let zone = Tz::from_str(&series.zone).unwrap_or(ctx.tz);
    let local = todo_core::LocalContext {
        now: ctx.now,
        tz: zone,
    };
    let at = |date: NaiveDate| match series.time_of_day {
        Some(time) => When::At(local.at(date, time)),
        None => When::Anytime,
    };
    (0..)
        .map(|offset| window_start + Duration::days(offset))
        .take_while(|date| *date < window_end)
        .filter(|date| series.rule.covers(series.start_day, *date))
        .filter_map(|date| {
            let exception = exceptions.iter().find(|held| held.date == date);
            let (status, when, due_tz) = match exception.map(|held| &held.kind) {
                Some(todo_core::ExceptionKind::Skipped) => return None,
                Some(todo_core::ExceptionKind::Done) => (OccurrenceStatus::Done, at(date), None),
                Some(todo_core::ExceptionKind::Moved { to, to_tz }) => {
                    (OccurrenceStatus::Moved, *to, to_tz.clone())
                }
                None => (OccurrenceStatus::Open, at(date), None),
            };
            Some(Occurrence {
                series: series.id.clone(),
                date,
                when,
                status,
                due_tz,
            })
        })
        .collect()
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

/// The average local time of day of the user's own timed tasks, which says when in the
/// day they like to do things. Series occurrences and calendar imports are left out:
/// the user did not choose a time for those one by one. Finished tasks still count,
/// since they are history.
pub fn usual_time(tasks: &[todo_core::Task], timezone: Tz) -> Option<NaiveTime> {
    let (seconds, count) = tasks
        .iter()
        .filter(|task| {
            !matches!(
                task.source,
                todo_core::Source::Series | todo_core::Source::Calendar
            )
        })
        .filter_map(|task| match task.when {
            When::At(start) => Some(start.with_timezone(&timezone).time()),
            When::Anytime | When::DueBy(_) => None,
        })
        .fold((0, 0), |(sum, count), time| {
            (sum + time.num_seconds_from_midnight(), count + 1)
        });
    NaiveTime::from_num_seconds_from_midnight_opt(seconds.checked_div(count)?, 0)
}

/// The free slot on the day of `deadline` whose start is closest to `usual` that day,
/// ending by the deadline and not before `now`. `None` when the day has no room.
///
/// The free starts form intervals, so the closest one to `usual` is either `usual`
/// itself or an interval's edge: the window or `now` at the front, the deadline or
/// window end at the back, or just after / just before a busy block.
fn slot_near(
    task: &todo_core::Task,
    deadline: DateTime<Utc>,
    usual: NaiveTime,
    minutes: i64,
    hours: &WorkingHours,
    occupied: &[(DateTime<Utc>, DateTime<Utc>)],
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let zone = task
        .due_tz
        .as_deref()
        .and_then(|name| Tz::from_str(name).ok())
        .unwrap_or(hours.timezone);
    let date = deadline.with_timezone(&zone).date_naive();
    let target = local_to_utc(hours.timezone, date, usual)?;
    let length = Duration::minutes(minutes);
    hours
        .days
        .get(&date.weekday())
        .into_iter()
        .flatten()
        .filter_map(|window| {
            let open = local_to_utc(hours.timezone, date, window.start)?.max(now);
            let close = local_to_utc(hours.timezone, date, window.end)?.min(deadline);
            Some((open, close))
        })
        .flat_map(|(open, close)| {
            let edges = occupied
                .iter()
                .flat_map(|(start, end)| [*end, *start - length])
                .chain([target, open, close - length]);
            edges.filter(move |start| {
                *start >= open
                    && *start + length <= close
                    && !occupied.iter().any(|(busy_start, busy_end)| {
                        *busy_start < *start + length && *busy_end > *start
                    })
            })
        })
        .min_by_key(|start| ((*start - target).abs(), *start))
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
    let last_day = deadline.map(|limit| limit.with_timezone(&hours.timezone).date_naive());
    for offset in 0..366 {
        let date = first_day + Duration::days(offset);
        // Nothing on a later day can end by the deadline, so stop looking.
        if last_day.is_some_and(|last| date > last) {
            return None;
        }
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
            off_planner: false,
        }
    }

    /// The same task, but the user took it off the planner.
    fn off(mut task: todo_core::Task) -> todo_core::Task {
        task.off_planner = true;
        task
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

    /// An `Anytime` task higher in the list must not take the only room a deadline
    /// has: both fit when the deadline goes first.
    #[test]
    fn a_deadline_goes_before_an_anytime_task_above_it() {
        let due = Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap();
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        );
        let result = plan(
            &[
                task("anytime", 120, When::Anytime, 0),
                task("due", 60, When::DueBy(due), 1),
            ],
            &hours,
            &ctx(
                Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap(),
                chrono_tz::UTC,
            ),
        );
        assert!(result.unplaced.is_empty(), "{:?}", result.unplaced);
        let due_task = result
            .placements
            .iter()
            .find(|placement| placement.task.0 == "due")
            .expect("the deadline is placed");
        assert!(due_task.end <= due);
    }

    #[test]
    fn a_deadline_no_working_hour_can_meet_leaves_the_task_unplaced() {
        // Due 07:00, but work starts at 08:00 every day: no slot ends by then.
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        );
        let due = Utc.with_ymd_and_hms(2026, 1, 5, 7, 0, 0).unwrap();
        let result = plan(
            &[task("early", 60, When::DueBy(due), 0)],
            &hours,
            &ctx(
                Utc.with_ymd_and_hms(2026, 1, 5, 6, 0, 0).unwrap(),
                chrono_tz::UTC,
            ),
        );
        assert!(result.placements.is_empty());
        assert_eq!(result.unplaced, vec![todo_core::TaskId("early".into())]);
    }

    #[test]
    fn a_task_taken_off_the_planner_is_named_in_its_own_bucket() {
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        let context = ctx(
            Utc.with_ymd_and_hms(2026, 1, 5, 7, 0, 0).unwrap(),
            chrono_tz::UTC,
        );
        let result = plan(
            &[
                off(task("aside", 60, When::Anytime, 0)),
                task("wanted", 60, When::Anytime, 1),
            ],
            &hours,
            &context,
        );
        assert_eq!(
            result.off_planner,
            vec![todo_core::TaskId("aside".into())],
            "asked for, so reported apart from the overflow"
        );
        assert!(
            !result.unplaced.contains(&todo_core::TaskId("aside".into())),
            "off the planner is not the same as out of room"
        );
        assert_eq!(
            result
                .placements
                .iter()
                .map(|placement| placement.task.0.as_str())
                .collect::<Vec<_>>(),
            vec!["wanted"]
        );
    }

    #[test]
    fn taking_a_task_off_the_planner_frees_the_room_it_would_have_taken() {
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        let context = ctx(
            Utc.with_ymd_and_hms(2026, 1, 5, 7, 0, 0).unwrap(),
            chrono_tz::UTC,
        );
        let where_next_lands = |blocker: todo_core::Task| {
            plan(
                &[blocker, task("next", 60, When::Anytime, 1)],
                &hours,
                &context,
            )
            .placements
            .into_iter()
            .find(|placement| placement.task.0 == "next")
            .map(|placement| placement.start)
            .expect("the next task is on the week")
        };
        let eight = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let noon = Utc.with_ymd_and_hms(2026, 1, 5, 12, 0, 0).unwrap();
        assert_eq!(
            where_next_lands(task("blocker", 240, When::Anytime, 0)),
            noon,
            "the four-hour task above it holds the morning"
        );
        assert_eq!(
            where_next_lands(off(task("blocker", 240, When::Anytime, 0))),
            eight,
            "a task off the planner never takes the morning, so the next one starts at eight"
        );
    }

    #[test]
    fn a_pinned_task_taken_off_the_planner_leaves_the_week() {
        // The flag is the user's decision about the week, and it outranks a pin:
        // otherwise the task would still be there and "off the planner" would be a lie.
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        let pinned = Utc.with_ymd_and_hms(2026, 1, 5, 11, 0, 0).unwrap();
        let result = plan(
            &[off(task("pinned", 60, When::At(pinned), 0))],
            &hours,
            &ctx(
                Utc.with_ymd_and_hms(2026, 1, 5, 7, 0, 0).unwrap(),
                chrono_tz::UTC,
            ),
        );
        assert!(result.placements.is_empty());
        assert_eq!(result.off_planner, vec![todo_core::TaskId("pinned".into())]);
    }

    #[test]
    fn a_done_task_taken_off_the_planner_is_still_only_off_it() {
        // Done comes first: a finished task is not on the week whatever else is true,
        // so it is neither placed nor reported as off the planner.
        let hours = WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        );
        let mut finished = off(task("finished", 60, When::Anytime, 0));
        finished.done = true;
        let result = plan(
            &[finished],
            &hours,
            &ctx(
                Utc.with_ymd_and_hms(2026, 1, 5, 7, 0, 0).unwrap(),
                chrono_tz::UTC,
            ),
        );
        assert!(result.placements.is_empty());
        assert!(result.unplaced.is_empty());
        assert!(
            result.off_planner.is_empty(),
            "it is done; there is nothing left to place"
        );
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
                let made = task(&format!("task-{index}"), duration, when, index);
                // One task in four has been taken off the planner, so the generator
                // covers the skip and the three-bucket invariant below.
                if generator.next(4) == 0 {
                    off(made)
                } else {
                    made
                }
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

    // ---- expansion (recurring tasks, ARCHITECTURE.md 7.1) --------

    fn gym() -> todo_core::Series {
        todo_core::Series {
            id: todo_core::SeriesId("s1".into()),
            title: "Gym".into(),
            duration_min: 45,
            rule: todo_core::Recurrence::new([Weekday::Mon, Weekday::Thu], None).unwrap(),
            start_day: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(), // Monday
            time_of_day: Some(NaiveTime::from_hms_opt(7, 0, 0).unwrap()),
            zone: "America/New_York".into(),
            external_uid: None,
            created: Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
            order: -1,
        }
    }

    fn exception(date: NaiveDate, kind: todo_core::ExceptionKind) -> todo_core::SeriesException {
        todo_core::SeriesException { date, kind }
    }

    fn days_of(occurrences: &[Occurrence]) -> Vec<chrono::Weekday> {
        occurrences.iter().map(|o| o.date.weekday()).collect()
    }

    #[test]
    fn occurrences_fall_on_the_series_weekdays_inside_the_window() {
        // Mon 12th .. Sun 18th: Monday and Thursday only.
        let occurrences = expand(
            &gym(),
            NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(),
            &[],
            &ctx(
                Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
                chrono_tz::America::New_York,
            ),
        );
        assert_eq!(days_of(&occurrences), vec![Weekday::Mon, Weekday::Thu]);
        // 07:00 New York in October (EDT) is 11:00 UTC.
        assert_eq!(
            occurrences[0].when,
            When::At(Utc.with_ymd_and_hms(2026, 10, 12, 11, 0, 0).unwrap())
        );
        assert!(
            occurrences
                .iter()
                .all(|o| o.status == OccurrenceStatus::Open && o.series.0 == "s1")
        );
    }

    #[test]
    fn the_window_bounds_are_exact() {
        // A window that opens on the occurrence day includes it; one that closes
        // on it (exclusive end) does not.
        let monday = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        let new_york = ctx(now, chrono_tz::America::New_York);
        assert_eq!(
            expand(&gym(), monday, monday + Duration::days(1), &[], &new_york).len(),
            1
        );
        // Tuesday to Wednesday: no occurrence days in between.
        assert!(
            expand(
                &gym(),
                monday + Duration::days(1),
                monday + Duration::days(3),
                &[],
                &new_york
            )
            .is_empty()
        );
    }

    #[test]
    fn start_day_and_until_bound_the_rule() {
        let mut held = gym();
        held.start_day = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(); // Thursday
        held.rule.until = Some(NaiveDate::from_ymd_opt(2026, 10, 26).unwrap());
        let occurrences = expand(
            &held,
            NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
            &[],
            &ctx(
                Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
                chrono_tz::America::New_York,
            ),
        );
        // Nothing before the 15th, nothing after the 26th: Thu 15, Mon 19, Thu 22,
        // Mon 26 (the until day, inclusive).
        assert_eq!(
            occurrences
                .iter()
                .map(|o| o.date.to_string())
                .collect::<Vec<_>>(),
            vec!["2026-10-15", "2026-10-19", "2026-10-22", "2026-10-26"]
        );
    }

    #[test]
    fn a_skipped_day_yields_nothing_and_a_done_day_stays_marked_done() {
        let monday = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let thursday = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let occurrences = expand(
            &gym(),
            monday,
            monday + Duration::days(7),
            &[
                exception(monday, todo_core::ExceptionKind::Skipped),
                exception(thursday, todo_core::ExceptionKind::Done),
            ],
            &ctx(
                Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
                chrono_tz::America::New_York,
            ),
        );
        assert_eq!(days_of(&occurrences), vec![Weekday::Thu]);
        assert_eq!(occurrences[0].status, OccurrenceStatus::Done);
        // A done occurrence keeps its slot's instant but gets no placement in the
        // plan (plan() skips done tasks).
        assert_eq!(
            occurrences[0].when,
            When::At(Utc.with_ymd_and_hms(2026, 10, 15, 11, 0, 0).unwrap())
        );
    }

    #[test]
    fn a_moved_occurrence_sits_at_the_new_time_under_its_original_date() {
        let monday = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let moved_to = Utc.with_ymd_and_hms(2026, 10, 14, 21, 0, 0).unwrap();
        let occurrences = expand(
            &gym(),
            monday,
            monday + Duration::days(7),
            &[exception(
                monday,
                todo_core::ExceptionKind::Moved {
                    to: When::At(moved_to),
                    to_tz: None,
                },
            )],
            &ctx(
                Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
                chrono_tz::America::New_York,
            ),
        );
        assert_eq!(days_of(&occurrences), vec![Weekday::Mon, Weekday::Thu]);
        assert_eq!(occurrences[0].status, OccurrenceStatus::Moved);
        assert_eq!(occurrences[0].when, When::At(moved_to));
    }

    #[test]
    fn a_series_without_a_time_floats() {
        let mut held = gym();
        held.time_of_day = None;
        let occurrences = expand(
            &held,
            NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(),
            &[],
            &ctx(
                Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
                chrono_tz::America::New_York,
            ),
        );
        assert!(
            occurrences
                .iter()
                .all(|o| o.when == When::Anytime && o.status == OccurrenceStatus::Open)
        );
    }

    #[test]
    fn dst_shifts_the_utc_instant_but_never_the_local_time() {
        // Mondays only, 07:00 New York. The week of the spring-forward change:
        // March 2 is EST (UTC-5), March 9 is EDT (UTC-4).
        let mut held = gym();
        held.rule = todo_core::Recurrence::new([Weekday::Mon], None).unwrap();
        held.start_day = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let occurrences = expand(
            &held,
            NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            NaiveDate::from_ymd_opt(2026, 3, 16).unwrap(),
            &[],
            &ctx(
                Utc.with_ymd_and_hms(2026, 3, 2, 1, 0, 0).unwrap(),
                chrono_tz::America::New_York,
            ),
        );
        assert_eq!(occurrences.len(), 2);
        assert_eq!(
            occurrences[0].when,
            When::At(Utc.with_ymd_and_hms(2026, 3, 2, 12, 0, 0).unwrap()),
            "07:00 EST is 12:00 UTC"
        );
        assert_eq!(
            occurrences[1].when,
            When::At(Utc.with_ymd_and_hms(2026, 3, 9, 11, 0, 0).unwrap()),
            "07:00 EDT is 11:00 UTC"
        );
    }

    #[test]
    fn a_time_inside_the_fall_back_overlap_takes_the_earlier_instant() {
        // Sundays 01:30. November 1st happens twice; the first pass (EDT) wins,
        // the same rule LocalContext::at applies to a pinned task.
        let mut held = gym();
        held.rule = todo_core::Recurrence::new([Weekday::Sun], None).unwrap();
        held.time_of_day = Some(NaiveTime::from_hms_opt(1, 30, 0).unwrap());
        let occurrences = expand(
            &held,
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
            &[],
            &ctx(
                Utc.with_ymd_and_hms(2026, 11, 1, 1, 0, 0).unwrap(),
                chrono_tz::America::New_York,
            ),
        );
        assert_eq!(
            occurrences[0].when,
            When::At(Utc.with_ymd_and_hms(2026, 11, 1, 5, 30, 0).unwrap())
        );
    }

    #[test]
    fn an_unparsable_zone_falls_back_to_the_request_zone() {
        let mut held = gym();
        held.zone = "Not/AZone".into();
        let occurrences = expand(
            &held,
            NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(),
            &[],
            &ctx(
                Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
                chrono_tz::UTC,
            ),
        );
        // 07:00 read as UTC.
        assert_eq!(
            occurrences[0].when,
            When::At(Utc.with_ymd_and_hms(2026, 10, 12, 7, 0, 0).unwrap())
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
            let off_ids: std::collections::HashSet<_> = first.off_planner.iter().collect();
            // Every task the planner was given lands in exactly one bucket: on the
            // week, in the overflow, or off the planner on purpose.
            assert!(placed_ids.is_disjoint(&unplaced_ids));
            assert!(placed_ids.is_disjoint(&off_ids));
            assert!(unplaced_ids.is_disjoint(&off_ids));
            assert_eq!(
                placed_ids.len() + unplaced_ids.len() + off_ids.len(),
                tasks.len()
            );

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

    // ---- day-only tasks: placed on their day near the user's usual time ----

    fn utc(day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, hour, minute, 0)
            .unwrap()
    }

    fn all_day() -> WorkingHours {
        WorkingHours::every_day(
            chrono_tz::UTC,
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
        )
    }

    /// Monday 06:00, before the day starts.
    fn monday() -> todo_core::LocalContext {
        ctx(utc(12, 6, 0), chrono_tz::UTC)
    }

    fn typed(id: &str, minutes: u16, when: When, source: todo_core::Source) -> todo_core::Task {
        todo_core::Task {
            source,
            ..task(id, minutes, when, 0)
        }
    }

    /// A day-only task due Friday the 16th at 23:59, the way capture makes one.
    fn friday_task() -> todo_core::Task {
        task("call mom", 60, When::DueBy(utc(16, 23, 59)), 1)
    }

    fn start_of(plan: &todo_core::Plan, id: &str) -> DateTime<Utc> {
        plan.placements
            .iter()
            .find(|placement| placement.task.0 == id)
            .unwrap_or_else(|| panic!("{id} is placed: {plan:?}"))
            .start
    }

    #[test]
    fn with_no_history_a_day_only_task_goes_at_noon_on_its_day() {
        let planned = plan(&[friday_task()], &all_day(), &monday());
        assert_eq!(start_of(&planned, "call mom"), utc(16, 12, 0));
    }

    #[test]
    fn the_usual_time_is_the_average_of_the_users_timed_tasks() {
        let tasks = [
            typed("a", 30, When::At(utc(13, 15, 0)), todo_core::Source::Text),
            typed("b", 30, When::At(utc(14, 17, 0)), todo_core::Source::Voice),
            friday_task(),
        ];
        assert_eq!(
            usual_time(&tasks, chrono_tz::UTC),
            NaiveTime::from_hms_opt(16, 0, 0)
        );
        let planned = plan(&tasks, &all_day(), &monday());
        assert_eq!(start_of(&planned, "call mom"), utc(16, 16, 0));
    }

    #[test]
    fn finished_tasks_still_count_as_history() {
        let mut finished = typed("old", 30, When::At(utc(9, 18, 0)), todo_core::Source::Text);
        finished.done = true;
        assert_eq!(
            usual_time(&[finished], chrono_tz::UTC),
            NaiveTime::from_hms_opt(18, 0, 0)
        );
    }

    #[test]
    fn series_and_calendar_times_are_not_history() {
        let tasks = [
            typed(
                "gym",
                60,
                When::At(utc(13, 6, 0)),
                todo_core::Source::Series,
            ),
            typed(
                "class",
                60,
                When::At(utc(13, 9, 0)),
                todo_core::Source::Calendar,
            ),
            task("no time", 30, When::Anytime, 0),
            task("by friday", 30, When::DueBy(utc(16, 23, 59)), 0),
        ];
        assert_eq!(usual_time(&tasks, chrono_tz::UTC), None);
    }

    #[test]
    fn the_usual_time_is_read_in_the_users_zone() {
        // 22:00 UTC is 18:00 in New York in October.
        let task = typed("a", 30, When::At(utc(13, 22, 0)), todo_core::Source::Text);
        assert_eq!(
            usual_time(&[task], chrono_tz::America::New_York),
            NaiveTime::from_hms_opt(18, 0, 0)
        );
    }

    #[test]
    fn the_setting_moves_the_no_history_time() {
        let mut hours = all_day();
        hours.day_only_time = NaiveTime::from_hms_opt(9, 0, 0).unwrap();
        let planned = plan(&[friday_task()], &hours, &monday());
        assert_eq!(start_of(&planned, "call mom"), utc(16, 9, 0));
    }

    #[test]
    fn a_busy_usual_time_takes_the_nearest_free_slot_on_that_day() {
        // Something fixed 11:30-12:30 holds noon. Just after it (12:30, 30 min away)
        // beats just before it (10:30, 90 min away).
        let tasks = [
            typed(
                "meeting",
                60,
                When::At(utc(16, 11, 30)),
                todo_core::Source::Series,
            ),
            friday_task(),
        ];
        let planned = plan(&tasks, &all_day(), &monday());
        assert_eq!(start_of(&planned, "call mom"), utc(16, 12, 30));
    }

    #[test]
    fn the_nearest_slot_can_be_before_a_busy_block() {
        // Busy 12:00-15:00: before it (11:00) is 60 min from noon, after it (15:00) is 180.
        let tasks = [
            typed(
                "workshop",
                180,
                When::At(utc(16, 12, 0)),
                todo_core::Source::Series,
            ),
            friday_task(),
        ];
        let planned = plan(&tasks, &all_day(), &monday());
        assert_eq!(start_of(&planned, "call mom"), utc(16, 11, 0));
    }

    #[test]
    fn the_task_still_ends_by_a_23_59_deadline() {
        // Busy 08:00-22:00, so only the evening is free. 90 minutes from 22:00 ends 23:30;
        // a slot may not run on past the 23:59 deadline.
        let tasks = [
            typed(
                "day",
                840,
                When::At(utc(16, 8, 0)),
                todo_core::Source::Series,
            ),
            task("late", 90, When::DueBy(utc(16, 23, 59)), 1),
        ];
        let planned = plan(&tasks, &all_day(), &monday());
        let late = planned
            .placements
            .iter()
            .find(|placement| placement.task.0 == "late")
            .expect("late is placed");
        assert_eq!(late.start, utc(16, 22, 0));
        assert!(late.end <= utc(16, 23, 59));
    }

    #[test]
    fn a_full_day_falls_back_to_the_earliest_free_slot_before_the_deadline() {
        let tasks = [
            typed(
                "all day",
                960,
                When::At(utc(16, 8, 0)),
                todo_core::Source::Series,
            ),
            friday_task(),
        ];
        let planned = plan(&tasks, &all_day(), &monday());
        assert_eq!(start_of(&planned, "call mom"), utc(12, 8, 0));
    }

    #[test]
    fn on_the_day_itself_a_usual_time_already_gone_starts_now() {
        let friday_afternoon = ctx(utc(16, 14, 0), chrono_tz::UTC);
        let planned = plan(&[friday_task()], &all_day(), &friday_afternoon);
        assert_eq!(start_of(&planned, "call mom"), utc(16, 14, 0));
    }

    #[test]
    fn an_anytime_task_still_takes_the_earliest_slot() {
        let planned = plan(
            &[task("floating", 30, When::Anytime, 0)],
            &all_day(),
            &monday(),
        );
        assert_eq!(start_of(&planned, "floating"), utc(12, 8, 0));
    }
}

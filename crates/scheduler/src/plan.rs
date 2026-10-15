//! Placing tasks into the free time of the working day.

use crate::hours::{WorkingHours, local_to_utc};
use chrono::{DateTime, Datelike, Duration, NaiveTime, Timelike, Utc};
use chrono_tz::Tz;
use std::str::FromStr;
use todo_core::When;
use todo_core::tuning::TASK_MIN_DURATION_MIN;

pub(crate) fn effective_minutes(task: &todo_core::Task) -> i64 {
    i64::from(task.duration_min.max(TASK_MIN_DURATION_MIN))
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

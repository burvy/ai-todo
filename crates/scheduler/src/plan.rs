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
    let mut board = Board::new(hours, ctx.now, usual_time(tasks, hours.timezone));
    let mut overdue = Vec::new();
    let mut day_only = Vec::new();
    let mut anytime = Vec::new();
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
            When::At(start) => board.fixed(task, start),
            When::DueBy(deadline) if deadline <= ctx.now => overdue.push(index),
            When::DueBy(deadline) => day_only.push((deadline, index)),
            When::Anytime => anytime.push(index),
        }
    }
    overdue.sort_by_key(|&index| (tasks[index].order, index));
    // A deadline takes precedence over list position (ARCHITECTURE.md 4.1): deadlines
    // first, earliest first, so an `Anytime` task above them cannot take their room.
    day_only.sort_by_key(|&(deadline, index)| (deadline, tasks[index].order, index));
    anytime.sort_by_key(|&index| (tasks[index].order, index));

    for index in overdue {
        board.overdue(&tasks[index]);
    }
    for (deadline, index) in day_only {
        board.day_only(&tasks[index], deadline);
    }
    for index in anytime {
        board.anytime(&tasks[index]);
    }
    todo_core::Plan {
        placements: board.placements,
        unplaced: board.unplaced,
        off_planner,
    }
}

/// The plan being built: what is placed so far, and the busy time every later task must
/// keep clear of. One method per queue; each takes one task.
pub(crate) struct Board<'a> {
    hours: &'a WorkingHours,
    now: DateTime<Utc>,
    usual: NaiveTime,
    busy: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    pub(crate) placements: Vec<todo_core::Placement>,
    pub(crate) unplaced: Vec<todo_core::TaskId>,
}

impl<'a> Board<'a> {
    /// An empty board. `usual` is the user's usual time of day, if they have one;
    /// day-only tasks otherwise aim for `hours.day_only_time`.
    pub(crate) fn new(
        hours: &'a WorkingHours,
        now: DateTime<Utc>,
        usual: Option<NaiveTime>,
    ) -> Self {
        Self {
            hours,
            now,
            usual: usual.unwrap_or(hours.day_only_time),
            busy: Vec::new(),
            placements: Vec::new(),
            unplaced: Vec::new(),
        }
    }

    /// A fixed task sits exactly at `start`, whatever else is there.
    pub(crate) fn fixed(&mut self, task: &todo_core::Task, start: DateTime<Utc>) {
        self.put(task, start);
    }

    /// An overdue task takes the earliest free slot from now, with no deadline left.
    pub(crate) fn overdue(&mut self, task: &todo_core::Task) {
        self.earliest(task);
    }

    /// A day-only task takes the free slot on its deadline's day closest to the usual
    /// time, else the earliest free slot before the deadline.
    pub(crate) fn day_only(&mut self, task: &todo_core::Task, deadline: DateTime<Utc>) {
        let minutes = effective_minutes(task);
        let slot = slot_near(
            task, deadline, self.usual, minutes, self.hours, &self.busy, self.now,
        )
        .or_else(|| next_slot(self.now, minutes, Some(deadline), self.hours, &self.busy));
        self.settle(task, slot);
    }

    /// A task with no time at all takes the earliest free slot from now.
    pub(crate) fn anytime(&mut self, task: &todo_core::Task) {
        self.earliest(task);
    }

    fn earliest(&mut self, task: &todo_core::Task) {
        let slot = next_slot(
            self.now,
            effective_minutes(task),
            None,
            self.hours,
            &self.busy,
        );
        self.settle(task, slot);
    }

    fn settle(&mut self, task: &todo_core::Task, slot: Option<DateTime<Utc>>) {
        match slot {
            Some(start) => self.put(task, start),
            None => self.unplaced.push(task.id.clone()),
        }
    }

    fn put(&mut self, task: &todo_core::Task, start: DateTime<Utc>) {
        let end = start + Duration::minutes(effective_minutes(task));
        self.placements.push(todo_core::Placement {
            task: task.id.clone(),
            start,
            end,
        });
        self.busy.push((start, end));
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

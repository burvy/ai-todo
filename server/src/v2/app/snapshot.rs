//! Reading a user's list: load, sort, expand series and plan into a `Snapshot`.

use std::collections::HashSet;

use chrono::{Duration, NaiveDate, NaiveTime};
use chrono_tz::Tz;
use scheduler::expand;
use todo_core::{
    ApiError, ErrorCode, Label, LocalContext, Occurrence, OccurrenceStatus, Priority, Series,
    Snapshot, Source, Task, TaskId, When,
};

use super::{App, check_not_occurrence, plan};
use crate::v2::store::Store;
use todo_core::tuning::DEFAULT_WINDOW_DAYS;

impl<S: Store> App<S> {
    pub(super) fn load(&self, user: &str) -> Result<Vec<Task>, ApiError> {
        let mut tasks = self.store.tasks(user).map_err(Self::storage_error)?;
        Self::sort_tasks(&mut tasks);
        Ok(tasks)
    }

    /// The order the client sees: by position, then by when each was made.
    pub(super) fn sort_tasks(tasks: &mut [Task]) {
        tasks.sort_by(|a, b| a.order.cmp(&b.order).then(a.created.cmp(&b.created)));
    }

    /// A snapshot over tasks already in hand, with the user's series expanded
    /// for the default window (ARCHITECTURE.md 7.1).
    ///
    /// Every mutating method loads the store once, changes its copy and returns a
    /// snapshot built from that copy. Loading again to return the result doubled the
    /// reads on every write and could show a value the caller never wrote.
    pub(super) fn snapshot_now(
        &self,
        user: &str,
        tasks: Vec<Task>,
        ctx: &LocalContext,
        tz: Tz,
    ) -> Result<Snapshot, ApiError> {
        let today = ctx.today();
        self.snapshot_in(
            user,
            tasks,
            ctx,
            tz,
            today,
            today + Duration::days(i64::from(DEFAULT_WINDOW_DAYS)),
        )
    }

    /// The same, with the series expanded over `[window_start, window_end)`.
    ///
    /// The window is the planner's requested range — the same days the view
    /// asks for — not a fixed horizon. Occurrences are ephemeral tasks: skipped
    /// days never appear, done ones appear marked done, moved ones sit at their
    /// new time. An undone past occurrence stays a fixed block on its own day;
    /// routines do not pile up as overdue — only one-shot `DueBy` tasks do.
    pub(super) fn snapshot_in(
        &self,
        user: &str,
        mut tasks: Vec<Task>,
        ctx: &LocalContext,
        tz: Tz,
        window_start: NaiveDate,
        window_end: NaiveDate,
    ) -> Result<Snapshot, ApiError> {
        let series = self.store.series(user).map_err(Self::storage_error)?;
        for held in &series {
            let exceptions = self
                .store
                .exceptions(user, &held.id)
                .map_err(Self::storage_error)?;
            for occurrence in expand(held, window_start, window_end, &exceptions, ctx) {
                tasks.push(Self::occurrence_task(held, &occurrence));
            }
        }
        // Every mutation appends its new tasks after the list it loaded, so the
        // next sort_tasks key (order, created) decides where they appear — that
        // must be true in the answer too, or a freshly created task would look
        // like it landed at the bottom of the list.
        Self::sort_tasks(&mut tasks);
        Ok(Snapshot {
            plan: plan(&tasks, &self.day_settings(user)?, ctx),
            tasks,
            tz: tz.name().to_owned(),
            generated: ctx.now,
            series,
        })
    }

    /// An occurrence as a row in the snapshot. Its id is
    /// `"{series_id}:{date}"`, a valid `TaskId`, so a placement can name it;
    /// it is never written to the store.
    pub(super) fn occurrence_task(series: &Series, occurrence: &Occurrence) -> Task {
        Task {
            id: TaskId(format!(
                "{}:{}",
                occurrence.series.0,
                occurrence.date.format("%Y-%m-%d")
            )),
            title: series.title.clone(),
            duration_min: series.duration_min,
            when: occurrence.when,
            done: occurrence.status == OccurrenceStatus::Done,
            order: series.order,
            label: Label::Personal,
            priority: Priority::Medium,
            source: Source::Series,
            created: series.created,
            updated_at: series.created,
            due_tz: occurrence.due_tz.clone(),
            external_uid: None,
            // An occurrence is on the planner like any other pinned task. Taking one
            // off clears the day, it does not hide the whole series.
            off_planner: false,
        }
    }

    /// The whole week in one value: tasks, their plan, the zone and the time it was made.
    pub fn snapshot(&self, user: &str, tz: Tz) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        self.snapshot_now(user, self.load(user)?, &ctx, tz)
    }

    /// Every task this user owns, in the order the client sees them, and no plan.
    ///
    /// For a caller that only wants the list — the export writes one out — so the
    /// scheduler is never run for a read that cannot use it.
    pub fn tasks(&self, user: &str) -> Result<Vec<Task>, ApiError> {
        self.load(user)
    }

    /// The same value with the plan narrowed to `from .. from + days`.
    ///
    /// The scheduler fills a long horizon and a week grid has nowhere to draw a
    /// placement nine months out, so floating placements are filtered to the window.
    /// **Pinned tasks keep their placement whatever the window is**: they are facts
    /// about a fixed instant, and the section 9 invariant requires every open pinned
    /// task to show one. The task list stays the whole list too, since a task the
    /// caller cannot see yet must not look deleted.
    ///
    /// This is also the window the series expand over: occurrences exist for the
    /// days the view actually asks about, not a fixed horizon.
    pub fn snapshot_in_window(
        &self,
        user: &str,
        tz: Tz,
        from: NaiveDate,
        days: u32,
    ) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut snapshot = self.snapshot_in(
            user,
            self.load(user)?,
            &ctx,
            tz,
            from,
            from + Duration::days(i64::from(days)),
        )?;
        let Some(start) = from
            .and_time(NaiveTime::MIN)
            .and_local_timezone(tz)
            .single()
        else {
            return Ok(snapshot);
        };
        let end = start + Duration::days(i64::from(days));
        let pinned: HashSet<TaskId> = snapshot
            .tasks
            .iter()
            .filter(|task| matches!(task.when, When::At(_)))
            .map(|task| task.id.clone())
            .collect();
        snapshot.plan.placements.retain(|placement| {
            pinned.contains(&placement.task) || (start <= placement.start && placement.end <= end)
        });
        Ok(snapshot)
    }

    /// The task's title, for a revision prompt.
    pub fn title_of(&self, user: &str, id: &str) -> Result<String, ApiError> {
        Ok(self.find(user, id)?.title)
    }

    pub(super) fn find(&self, user: &str, id: &str) -> Result<Task, ApiError> {
        check_not_occurrence(id)?;
        self.load(user)?
            .into_iter()
            .find(|task| task.id.0 == id)
            .ok_or_else(|| ApiError {
                code: ErrorCode::NotFound,
                message: "task not found".into(),
            })
    }
}

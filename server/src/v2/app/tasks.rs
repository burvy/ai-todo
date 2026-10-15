//! Writing one-off tasks: create, patch, restore, delete, reorder, add from a ramble.

use std::collections::HashMap;

use capture::Revision;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use todo_core::{
    ApiError, ErrorCode, Label, Priority, Snapshot, Source, Task, TaskDraft, TaskId, When,
};

use super::{App, check_duration, check_fresh, check_not_occurrence, check_room, check_title};
use crate::v2::store::Store;
use todo_core::tuning::{TASK_MAX_DURATION_MIN, TASK_MIN_DURATION_MIN};

/// The zone a `DueBy` deadline was set in, and `None` for anything that is not a
/// deadline (docs/ARCHITECTURE.md 4.1): `Anytime` and `At` have no zone of their own.
pub(super) fn due_zone(when: When, tz: Tz) -> Option<String> {
    match when {
        When::DueBy(_) => Some(tz.name().to_owned()),
        When::Anytime | When::At(_) => None,
    }
}

/// A restored task's place: where it was, kept within one step of the list as it is
/// now. Its relative place survives, and a hostile `i64::MIN` cannot make the next
/// capture's `min_order - n` overflow.
pub(super) fn restored_order(order: i64, tasks: &[Task]) -> i64 {
    let orders = || tasks.iter().map(|task| task.order);
    let low = orders().min().unwrap_or(0).saturating_sub(1);
    let high = orders().max().unwrap_or(0).saturating_add(1);
    order.clamp(low, high)
}

/// A restored deadline keeps the zone it was set in when that names a real zone;
/// anything else is re-anchored where the user is now, as an edit would.
pub(super) fn restored_zone(task: &Task, tz: Tz) -> Option<String> {
    match (task.when, task.due_tz.as_deref().map(str::parse::<Tz>)) {
        (When::DueBy(_), Some(Ok(zone))) => Some(zone.name().to_owned()),
        (when, _) => due_zone(when, tz),
    }
}

/// Moves a task to a new `when`, re-anchoring a deadline's `due_tz` only when the
/// instant actually changes (docs/ARCHITECTURE.md 4.1). Every edit path goes through
/// here, so a save that restates the instant the task already has — the edit dialog
/// always sends `when`, and a revision that repeats the same day — keeps the zone the
/// deadline was set in.
pub(super) fn set_when(task: &mut Task, when: When, tz: Tz) {
    if when != task.when {
        task.when = when;
        task.due_tz = due_zone(when, tz);
    }
}

/// The fields a client may change with PATCH. Absent fields stay as they are.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
pub struct Patch {
    pub title: Option<String>,
    pub duration_min: Option<u16>,
    pub when: Option<When>,
    pub done: Option<bool>,
    pub label: Option<Label>,
    pub priority: Option<Priority>,
    /// Take the task off the planner, or put it back. Absent leaves it as it is, so
    /// this is the whole of "off the planner" for a client (ARCHITECTURE.md 4.2).
    pub off_planner: Option<bool>,
    /// The `updated_at` of the task as the client last saw it. Absent means "don't check".
    pub updated_at: Option<DateTime<Utc>>,
}

impl<S: Store> App<S> {
    pub fn create(&mut self, draft: TaskDraft, user: &str, tz: Tz) -> Result<Snapshot, ApiError> {
        check_title(&draft.title)?;
        check_duration(draft.duration_min)?;
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        check_room(tasks.len(), 1)?;
        // Newest first (docs/ARCHITECTURE.md 4.1): a fresh task takes the
        // smallest order so sort_tasks puts it at the top of the list. After a
        // reorder orders are 0..n again, so this stays safe indefinitely.
        let order = tasks.iter().map(|task| task.order).min().unwrap_or(0) - 1;
        let task = Task {
            id: Self::fresh_id(),
            title: draft.title.trim().to_owned(),
            duration_min: draft.duration_min,
            when: draft.when,
            done: false,
            order,
            label: draft.label,
            priority: draft.priority,
            source: draft.source,
            created: ctx.now,
            updated_at: ctx.now,
            due_tz: due_zone(draft.when, tz),
            external_uid: None,
            off_planner: false,
        };
        self.store
            .put(user, task.clone())
            .map_err(Self::storage_error)?;
        tasks.push(task);
        self.snapshot_now(user, tasks, &ctx, tz)
    }

    pub fn patch(
        &mut self,
        id: &str,
        patch: Patch,
        user: &str,
        tz: Tz,
    ) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        let task = Self::held(&mut tasks, id)?;
        check_fresh(task, patch.updated_at)?;
        if let Some(title) = patch.title {
            check_title(&title)?;
            task.title = title.trim().to_owned();
        }
        if let Some(minutes) = patch.duration_min {
            check_duration(minutes)?;
            task.duration_min = minutes;
        }
        if let Some(when) = patch.when {
            set_when(task, when, tz);
        }
        if let Some(done) = patch.done {
            task.done = done;
        }
        if let Some(label) = patch.label {
            task.label = label;
        }
        if let Some(priority) = patch.priority {
            task.priority = priority;
        }
        if let Some(off_planner) = patch.off_planner {
            task.off_planner = off_planner;
        }
        task.updated_at = ctx.now;
        let task = task.clone();
        self.store.put(user, task).map_err(Self::storage_error)?;
        self.snapshot_now(user, tasks, &ctx, tz)
    }

    /// Adds the drafts a capture produced. Returns their ids and the new snapshot.
    pub fn add_drafts(
        &mut self,
        drafts: Vec<TaskDraft>,
        user: &str,
        tz: Tz,
    ) -> Result<(Vec<TaskId>, Snapshot), ApiError> {
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        check_room(tasks.len(), drafts.len())?;
        // The whole capture lands on top (newest first), still in capture order:
        // orders run from min - drafts.len() up to min - 1, so they never collide
        // with orders already held (a capture of several drafts would otherwise
        // walk straight into them).
        let min_order = tasks.iter().map(|task| task.order).min().unwrap_or(0);
        let first_order = min_order - drafts.len() as i64;
        let mut ids = Vec::new();
        for (offset, draft) in drafts.into_iter().enumerate() {
            let order = first_order + offset as i64;
            check_title(&draft.title)?;
            check_duration(draft.duration_min)?;
            // Each new task goes onto `tasks` before the next id is minted, so ids
            // stay unique inside one capture without re-reading the store.
            let task = Task {
                id: Self::fresh_id(),
                title: draft.title.trim().to_owned(),
                duration_min: draft.duration_min,
                when: draft.when,
                done: false,
                order,
                label: draft.label,
                priority: draft.priority,
                source: draft.source,
                created: ctx.now,
                updated_at: ctx.now,
                due_tz: due_zone(draft.when, tz),
                external_uid: None,
                off_planner: false,
            };
            ids.push(task.id.clone());
            self.store
                .put(user, task.clone())
                .map_err(Self::storage_error)?;
            tasks.push(task);
        }
        Ok((ids, self.snapshot_now(user, tasks, &ctx, tz)?))
    }

    /// Applies what a revision changed. Fields the instruction did not touch stay as they are.
    pub fn apply_revision(
        &mut self,
        id: &str,
        revision: &Revision,
        seen: Option<DateTime<Utc>>,
        user: &str,
        tz: Tz,
    ) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        let task = Self::held(&mut tasks, id)?;
        check_fresh(task, seen)?;
        // Without a day the revision leaves the task's time alone.
        if revision.day.is_some() {
            set_when(
                task,
                dates::when_from(
                    revision.day,
                    revision.time,
                    self.day_settings(user)?.deadline,
                    &ctx,
                ),
                tz,
            );
        }
        if let Some(minutes) = revision.duration_min {
            task.duration_min = minutes.clamp(TASK_MIN_DURATION_MIN, TASK_MAX_DURATION_MIN);
        }
        if let Some(priority) = revision.priority {
            task.priority = priority;
        }
        if let Some(label) = revision.label {
            task.label = label;
        }
        task.updated_at = ctx.now;
        let task = task.clone();
        self.store.put(user, task).map_err(Self::storage_error)?;
        self.snapshot_now(user, tasks, &ctx, tz)
    }

    /// Put back a task the user has just deleted (the scrap undo): its title, length,
    /// time, done state, place in the list, label, priority and creation time come back
    /// as they were. The id is a fresh one, because ids are unique across every user and
    /// one sent by a client cannot be trusted not to name someone else's. A repeat's day
    /// is not a stored task; its undo clears the skip on the series instead.
    ///
    /// Everything else the client sends is checked the way the server would have
    /// written it: the place is kept within one step of the current list, a deadline's
    /// zone must be a real zone, the creation time cannot be in the future, and the
    /// import link is dropped, so a restored task can never shadow another for a
    /// re-import. Sending the same restore twice stores two tasks (the task cap bounds it).
    pub fn restore(&mut self, task: Task, user: &str, tz: Tz) -> Result<Snapshot, ApiError> {
        check_title(&task.title)?;
        check_duration(task.duration_min)?;
        if task.source == Source::Series {
            return Err(ApiError {
                code: ErrorCode::BadRequest,
                message: "a repeat's day is put back on its series, not restored".into(),
            });
        }
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        check_room(tasks.len(), 1)?;
        let task = Task {
            id: Self::fresh_id(),
            title: task.title.trim().to_owned(),
            order: restored_order(task.order, &tasks),
            due_tz: restored_zone(&task, tz),
            created: task.created.min(ctx.now),
            updated_at: ctx.now,
            external_uid: None,
            ..task
        };
        self.store
            .put(user, task.clone())
            .map_err(Self::storage_error)?;
        tasks.push(task);
        self.snapshot_now(user, tasks, &ctx, tz)
    }

    pub fn delete(&mut self, id: &str, user: &str, tz: Tz) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        let task_id = Self::held(&mut tasks, id)?.id.clone();
        tasks.retain(|task| task.id != task_id);
        self.store
            .delete(user, &task_id)
            .map_err(Self::storage_error)?;
        self.snapshot_now(user, tasks, &ctx, tz)
    }

    /// `ids` is the full new order: every task exactly once.
    pub fn reorder(&mut self, ids: &[String], user: &str, tz: Tz) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        let mut current: Vec<&str> = tasks.iter().map(|task| task.id.0.as_str()).collect();
        let mut given: Vec<&str> = ids.iter().map(String::as_str).collect();
        current.sort_unstable();
        given.sort_unstable();
        if current != given {
            return Err(ApiError {
                code: ErrorCode::BadRequest,
                message: "ids must list every task exactly once".into(),
            });
        }
        // One transaction for the whole reorder, so a list that fails halfway
        // cannot leave the tasks half-renumbered.
        let ordered: Vec<TaskId> = ids.iter().cloned().map(TaskId).collect();
        self.store
            .set_order(user, &ordered)
            .map_err(Self::storage_error)?;
        // The store now numbers them by position, so mirror that here rather than
        // re-reading to find out what the client is about to be shown.
        let mut by_id: HashMap<TaskId, Task> = tasks
            .drain(..)
            .map(|task| (task.id.clone(), task))
            .collect();
        tasks = ordered
            .into_iter()
            .filter_map(|id| by_id.remove(&id))
            .collect();
        // The local copies still carry their old orders; the store renumbered by
        // position. Mirror it so the snapshot we hand back sorts like the next
        // load would (snapshot_of sorts by order).
        for (position, task) in tasks.iter_mut().enumerate() {
            task.order = position as i64;
        }
        self.snapshot_now(user, tasks, &ctx, tz)
    }

    /// The task with this id, in a list already loaded, for mutation.
    pub(super) fn held<'a>(tasks: &'a mut [Task], id: &str) -> Result<&'a mut Task, ApiError> {
        check_not_occurrence(id)?;
        tasks
            .iter_mut()
            .find(|task| task.id.0 == id)
            .ok_or_else(|| ApiError {
                code: ErrorCode::NotFound,
                message: "task not found".into(),
            })
    }

    /// A new task id: 128 random bits, so two users (or two requests) can never be
    /// handed the same one. Ids are one namespace across every user in the database.
    pub(super) fn fresh_id() -> TaskId {
        TaskId(format!("t{}", uuid::Uuid::new_v4().simple()))
    }
}

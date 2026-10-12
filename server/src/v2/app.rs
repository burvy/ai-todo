//! The v2 service. One method per endpoint. Each method loads, changes, re-plans and
//! returns a `Snapshot` in one call, so a snapshot is never half-updated. The HTTP
//! layer holds the `App` lock for the whole call.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use capture::Revision;
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;
use scheduler::WorkingHours;
use serde::Serialize;
use todo_core::{
    ApiError, Clock, ErrorCode, Label, LocalContext, Plan, Priority, Snapshot, Source, Task,
    TaskDraft, TaskId, When,
};

use super::store::{Store, StoreError};

pub const MIN_DURATION_MIN: u16 = 5;
pub const MAX_DURATION_MIN: u16 = 480;

/// Working hours used when placing floating tasks. Local time, every day.
fn working_hours(tz: Tz) -> WorkingHours {
    WorkingHours::every_day(
        tz,
        NaiveTime::from_hms_opt(8, 0, 0).expect("08:00 is valid"),
        NaiveTime::from_hms_opt(23, 59, 59).expect("23:59:59 is valid"),
    )
}

/// The fields a client may change with PATCH. Absent fields stay as they are.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
pub struct Patch {
    pub title: Option<String>,
    pub duration_min: Option<u16>,
    pub when: Option<When>,
    pub done: Option<bool>,
    pub label: Option<Label>,
}

pub struct App<S: Store> {
    store: S,
    clock: Arc<dyn Clock>,
}

impl<S: Store> App<S> {
    pub fn new(store: S, clock: Arc<dyn Clock>) -> Self {
        Self { store, clock }
    }

    fn ctx(&self, tz: Tz) -> LocalContext {
        LocalContext {
            now: self.clock.now(),
            tz,
        }
    }

    /// Storage failures become the error the client sees.
    ///
    /// A missing task is a 404 and a refused write is a 409; anything else is
    /// the database's fault and must not read as the caller's.
    fn storage_error(error: StoreError) -> ApiError {
        let code = match error {
            StoreError::NotFound => ErrorCode::NotFound,
            StoreError::Conflict(_) => ErrorCode::Conflict,
            StoreError::Backend(_) => ErrorCode::Internal,
        };
        ApiError {
            code,
            message: error.to_string(),
        }
    }

    fn load(&self, user: &str) -> Result<Vec<Task>, ApiError> {
        let mut tasks = self.store.tasks(user).map_err(Self::storage_error)?;
        Self::sort_tasks(&mut tasks);
        Ok(tasks)
    }

    /// The order the client sees: by position, then by when each was made.
    fn sort_tasks(tasks: &mut [Task]) {
        tasks.sort_by(|a, b| a.order.cmp(&b.order).then(a.created.cmp(&b.created)));
    }

    /// A snapshot over tasks already in hand.
    ///
    /// Every mutating method loads the store once, changes its copy and returns a
    /// snapshot built from that copy. Loading again to return the result doubled the
    /// reads on every write and could show a value the caller never wrote.
    fn snapshot_of(tasks: Vec<Task>, ctx: &LocalContext, tz: Tz) -> Snapshot {
        Snapshot {
            plan: plan(&tasks, ctx),
            tasks,
            tz: tz.name().to_owned(),
            generated: ctx.now,
        }
    }

    /// The whole week in one value: tasks, their plan, the zone and the time it was made.
    pub fn snapshot(&self, user: &str, tz: Tz) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        Ok(Self::snapshot_of(self.load(user)?, &ctx, tz))
    }

    /// The same value with the plan narrowed to `from .. from + days`.
    ///
    /// The scheduler fills a long horizon and a week grid has nowhere to draw a
    /// placement nine months out, so floating placements are filtered to the window.
    /// **Pinned tasks keep their placement whatever the window is**: they are facts
    /// about a fixed instant, and the section 9 invariant requires every open pinned
    /// task to show one. The task list stays the whole list too, since a task the
    /// caller cannot see yet must not look deleted.
    pub fn snapshot_in_window(
        &self,
        user: &str,
        tz: Tz,
        from: NaiveDate,
        days: u32,
    ) -> Result<Snapshot, ApiError> {
        let mut snapshot = self.snapshot(user, tz)?;
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

    pub fn create(&mut self, draft: TaskDraft, user: &str, tz: Tz) -> Result<Snapshot, ApiError> {
        check_title(&draft.title)?;
        check_duration(draft.duration_min)?;
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        let order = tasks.iter().map(|task| task.order).max().unwrap_or(-1) + 1;
        let task = Task {
            id: Self::fresh_id(&tasks, ctx.now.timestamp_nanos_opt().unwrap_or_default()),
            title: draft.title.trim().to_owned(),
            duration_min: draft.duration_min,
            when: draft.when,
            done: false,
            order,
            label: draft.label,
            priority: draft.priority,
            source: draft.source,
            created: ctx.now,
            external_uid: None,
        };
        self.store
            .put(user, task.clone())
            .map_err(Self::storage_error)?;
        tasks.push(task);
        Ok(Self::snapshot_of(tasks, &ctx, tz))
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
        if let Some(title) = patch.title {
            check_title(&title)?;
            task.title = title.trim().to_owned();
        }
        if let Some(minutes) = patch.duration_min {
            check_duration(minutes)?;
            task.duration_min = minutes;
        }
        if let Some(when) = patch.when {
            task.when = when;
        }
        if let Some(done) = patch.done {
            task.done = done;
        }
        if let Some(label) = patch.label {
            task.label = label;
        }
        let task = task.clone();
        self.store.put(user, task).map_err(Self::storage_error)?;
        Ok(Self::snapshot_of(tasks, &ctx, tz))
    }

    /// Import calendar events as `At` tasks. An event whose UID is already a task
    /// updates that task; any other event becomes a new task.
    pub fn import(
        &mut self,
        events: Vec<ImportedEvent>,
        user: &str,
        tz: Tz,
    ) -> Result<ImportResult, ApiError> {
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        let mut order = tasks.iter().map(|task| task.order).max().unwrap_or(-1) + 1;
        let mut created = Vec::new();
        let mut updated = Vec::new();
        for event in events {
            let duration_min = event_minutes(event.start, event.end);
            // The store owns the uid index, so this is one lookup per event
            // rather than a map built from every task on the server.
            let existing = match &event.uid {
                Some(uid) => self
                    .store
                    .find_by_uid(user, uid)
                    .map_err(Self::storage_error)?,
                None => None,
            };
            if let Some(mut task) = existing {
                task.title = event.title;
                task.duration_min = duration_min;
                task.when = When::At(event.start);
                updated.push(task.id.clone());
                if let Some(slot) = tasks.iter_mut().find(|held| held.id == task.id) {
                    *slot = task.clone();
                }
                self.store.put(user, task).map_err(Self::storage_error)?;
                continue;
            }
            let task = Task {
                id: Self::fresh_id(&tasks, ctx.now.timestamp_nanos_opt().unwrap_or_default()),
                title: event.title,
                duration_min,
                when: When::At(event.start),
                done: false,
                order,
                label: Label::Other,
                priority: Priority::Low,
                source: Source::Calendar,
                created: ctx.now,
                external_uid: event.uid.clone(),
            };
            order += 1;
            created.push(task.id.clone());
            self.store
                .put(user, task.clone())
                .map_err(Self::storage_error)?;
            tasks.push(task);
        }
        Ok(ImportResult {
            created,
            updated,
            snapshot: Self::snapshot_of(tasks, &ctx, tz),
        })
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
        let first_order = tasks.iter().map(|task| task.order).max().unwrap_or(-1) + 1;
        let mut ids = Vec::new();
        for (offset, draft) in drafts.into_iter().enumerate() {
            let order = first_order + offset as i64;
            check_title(&draft.title)?;
            check_duration(draft.duration_min)?;
            // Each new task goes onto `tasks` before the next id is minted, so ids
            // stay unique inside one capture without re-reading the store.
            let task = Task {
                id: Self::fresh_id(&tasks, ctx.now.timestamp_nanos_opt().unwrap_or_default()),
                title: draft.title.trim().to_owned(),
                duration_min: draft.duration_min,
                when: draft.when,
                done: false,
                order,
                label: draft.label,
                priority: draft.priority,
                source: draft.source,
                created: ctx.now,
                external_uid: None,
            };
            ids.push(task.id.clone());
            self.store
                .put(user, task.clone())
                .map_err(Self::storage_error)?;
            tasks.push(task);
        }
        Ok((ids, Self::snapshot_of(tasks, &ctx, tz)))
    }

    /// Applies what a revision changed. Fields the instruction did not touch stay as they are.
    pub fn apply_revision(
        &mut self,
        id: &str,
        revision: &Revision,
        user: &str,
        tz: Tz,
    ) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        let task = Self::held(&mut tasks, id)?;
        match (revision.day, revision.time) {
            (Some(day), Some(time)) => task.when = When::At(ctx.at(day, time)),
            (Some(day), None) => {
                let end_of_day = NaiveTime::from_hms_opt(17, 0, 0).expect("17:00 is valid");
                task.when = When::DueBy(ctx.at(day, end_of_day));
            }
            _ => {}
        }
        if let Some(minutes) = revision.duration_min {
            task.duration_min = minutes.clamp(MIN_DURATION_MIN, MAX_DURATION_MIN);
        }
        if let Some(priority) = revision.priority {
            task.priority = priority;
        }
        if let Some(label) = revision.label {
            task.label = label;
        }
        let task = task.clone();
        self.store.put(user, task).map_err(Self::storage_error)?;
        Ok(Self::snapshot_of(tasks, &ctx, tz))
    }

    /// The task's title, for a revision prompt.
    pub fn title_of(&self, user: &str, id: &str) -> Result<String, ApiError> {
        Ok(self.find(user, id)?.title)
    }

    /// The app's notion of now.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// How many captures this user has run on `day`, for the daily Jev cap.
    pub fn captures_today(&self, user: &str, day: chrono::NaiveDate) -> Result<u32, ApiError> {
        self.store
            .captures_today(user, day)
            .map_err(Self::storage_error)
    }

    /// The store itself, for the account calls in [`super::auth`], which work on
    /// users and sessions rather than on tasks.
    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn delete(&mut self, id: &str, user: &str, tz: Tz) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        let task_id = Self::held(&mut tasks, id)?.id.clone();
        tasks.retain(|task| task.id != task_id);
        self.store
            .delete(user, &task_id)
            .map_err(Self::storage_error)?;
        Ok(Self::snapshot_of(tasks, &ctx, tz))
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
        Ok(Self::snapshot_of(tasks, &ctx, tz))
    }

    /// The task with this id, in a list already loaded, for mutation.
    fn held<'a>(tasks: &'a mut [Task], id: &str) -> Result<&'a mut Task, ApiError> {
        tasks
            .iter_mut()
            .find(|task| task.id.0 == id)
            .ok_or_else(|| ApiError {
                code: ErrorCode::NotFound,
                message: "task not found".into(),
            })
    }

    fn find(&self, user: &str, id: &str) -> Result<Task, ApiError> {
        self.load(user)?
            .into_iter()
            .find(|task| task.id.0 == id)
            .ok_or_else(|| ApiError {
                code: ErrorCode::NotFound,
                message: "task not found".into(),
            })
    }

    /// An id that is not used yet. The first candidate is the clock in nanoseconds.
    ///
    /// `tasks` is the list the caller already holds, so minting an id costs no store
    /// read. It must include anything minted earlier in the same call, which is why
    /// callers push each new task as they go.
    fn fresh_id(tasks: &[Task], seed: i64) -> TaskId {
        let taken: HashSet<&str> = tasks.iter().map(|task| task.id.0.as_str()).collect();
        (0..)
            .map(|attempt| TaskId(format!("t{}", seed + attempt)))
            .find(|candidate| !taken.contains(candidate.0.as_str()))
            .expect("an unused id exists")
    }
}

/// One calendar event, as the import needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedEvent {
    pub uid: Option<String>,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImportResult {
    pub created: Vec<TaskId>,
    pub updated: Vec<TaskId>,
    pub snapshot: Snapshot,
}

/// An event's length, kept inside the same limits as any task.
fn event_minutes(start: DateTime<Utc>, end: DateTime<Utc>) -> u16 {
    let minutes = (end - start).num_minutes();
    minutes.clamp(i64::from(MIN_DURATION_MIN), i64::from(MAX_DURATION_MIN)) as u16
}

fn check_title(title: &str) -> Result<(), ApiError> {
    if title.trim().is_empty() {
        return Err(ApiError {
            code: ErrorCode::BadRequest,
            message: "title is required".into(),
        });
    }
    Ok(())
}

fn check_duration(minutes: u16) -> Result<(), ApiError> {
    if !(MIN_DURATION_MIN..=MAX_DURATION_MIN).contains(&minutes) {
        return Err(ApiError {
            code: ErrorCode::BadRequest,
            message: format!("duration_min must be {MIN_DURATION_MIN} to {MAX_DURATION_MIN}"),
        });
    }
    Ok(())
}

/// Where each open task goes: the scheduler fits floating tasks into the working
/// time around pinned (`At`) tasks, which stay exactly where they are.
pub fn plan(tasks: &[Task], ctx: &LocalContext) -> Plan {
    scheduler::plan(tasks, &working_hours(ctx.tz), ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::store::{MemoryStore, StoreError};
    use chrono::{TimeZone, Utc};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use todo_core::{FixedClock, Label, Priority};

    /// Counts how many times the whole task list is read from the store.
    ///
    /// Behaviour tests cannot see this: a method that loads twice returns exactly the
    /// same snapshot as one that loads once. Only a count tells the two apart.
    struct CountingStore {
        inner: MemoryStore,
        reads: Arc<AtomicUsize>,
    }

    impl CountingStore {
        /// The store, and a handle on its read counter.
        fn new() -> (Self, Arc<AtomicUsize>) {
            let reads = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    inner: MemoryStore::default(),
                    reads: Arc::clone(&reads),
                },
                reads,
            )
        }
    }

    impl Store for CountingStore {
        fn tasks(&self, user: &str) -> Result<Vec<Task>, StoreError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.inner.tasks(user)
        }

        fn put(&self, user: &str, task: Task) -> Result<(), StoreError> {
            self.inner.put(user, task)
        }

        fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError> {
            self.inner.delete(user, id)
        }

        fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError> {
            self.inner.set_order(user, ids)
        }

        fn find_by_uid(&self, user: &str, uid: &str) -> Result<Option<Task>, StoreError> {
            self.inner.find_by_uid(user, uid)
        }

        fn upsert_user_and_session(
            &self,
            user: &crate::v2::auth::Me,
            token: &str,
            expires_at: chrono::DateTime<Utc>,
        ) -> Result<(), StoreError> {
            self.inner.upsert_user_and_session(user, token, expires_at)
        }

        fn delete_session(&self, token: &str) -> Result<(), StoreError> {
            self.inner.delete_session(token)
        }

        fn session_user(
            &self,
            token: &str,
            now: chrono::DateTime<Utc>,
        ) -> Result<Option<crate::v2::auth::Me>, StoreError> {
            self.inner.session_user(token, now)
        }

        fn delete_user(&self, user: &str) -> Result<(), StoreError> {
            self.inner.delete_user(user)
        }

        fn captures_today(&self, user: &str, day: chrono::NaiveDate) -> Result<u32, StoreError> {
            self.inner.captures_today(user, day)
        }
    }

    fn app() -> (App<CountingStore>, Arc<AtomicUsize>) {
        let (store, reads) = CountingStore::new();
        let now = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        (App::new(store, Arc::new(FixedClock(now))), reads)
    }

    fn draft(title: &str) -> TaskDraft {
        TaskDraft {
            title: title.into(),
            duration_min: 30,
            when: When::Anytime,
            label: Label::Other,
            priority: Priority::Low,
            source: Source::Text,
        }
    }

    /// Creating one task reads the store once. It used to read it twice: once to find
    /// the next position, and again inside `snapshot()` to build the answer.
    #[test]
    fn create_reads_the_store_once() {
        let (mut app, reads) = app();
        app.create(
            draft("Write the report"),
            crate::v2::auth::LOCAL_USER,
            chrono_tz::America::New_York,
        )
        .expect("create");
        assert_eq!(reads.load(Ordering::SeqCst), 1, "create re-read the store");
    }

    /// The real win: minting an id used to load every task, so a capture that produced
    /// ten tasks read the store ten times on top of the read to answer the request.
    #[test]
    fn minting_ids_does_not_read_the_store() {
        let (mut app, reads) = app();
        let drafts: Vec<TaskDraft> = (0..10).map(|i| draft(&format!("Task {i}"))).collect();
        let (ids, snapshot) = app
            .add_drafts(
                drafts,
                crate::v2::auth::LOCAL_USER,
                chrono_tz::America::New_York,
            )
            .expect("add drafts");

        assert_eq!(ids.len(), 10, "every draft became a task");
        assert_eq!(snapshot.tasks.len(), 10);
        assert_eq!(
            reads.load(Ordering::SeqCst),
            1,
            "ten tasks should not mean eleven store reads"
        );
        let unique: HashSet<_> = ids.iter().map(|id| &id.0).collect();
        assert_eq!(unique.len(), 10, "ids minted in one batch must be unique");
    }

    #[test]
    fn patch_and_delete_and_reorder_each_read_the_store_once() {
        let (mut app, reads) = app();
        let tz = chrono_tz::America::New_York;
        let snapshot = app
            .create(draft("Write the report"), crate::v2::auth::LOCAL_USER, tz)
            .expect("create");
        let id = snapshot.tasks[0].id.0.clone();

        let before = reads.load(Ordering::SeqCst);
        app.patch(
            &id,
            Patch {
                title: Some("Write the essay".into()),
                ..Patch::default()
            },
            crate::v2::auth::LOCAL_USER,
            tz,
        )
        .expect("patch");

        assert_eq!(
            reads.load(Ordering::SeqCst) - before,
            1,
            "patch re-read the store"
        );

        let before = reads.load(Ordering::SeqCst);
        app.reorder(std::slice::from_ref(&id), crate::v2::auth::LOCAL_USER, tz)
            .expect("reorder");
        assert_eq!(
            reads.load(Ordering::SeqCst) - before,
            1,
            "reorder re-read the store"
        );

        let before = reads.load(Ordering::SeqCst);
        let after = app
            .delete(&id, crate::v2::auth::LOCAL_USER, tz)
            .expect("delete");
        assert_eq!(
            reads.load(Ordering::SeqCst) - before,
            1,
            "delete re-read the store"
        );
        assert!(after.tasks.is_empty(), "the task is gone from the answer");
    }

    /// The snapshot a mutation returns must show the write that was just made.
    /// Building it from the list in hand rather than re-reading is only safe if the
    /// two stay identical.
    #[test]
    fn the_returned_snapshot_shows_every_change_it_just_made() {
        let (mut app, _reads) = app();
        let tz = chrono_tz::America::New_York;
        let mut snapshot = app
            .create(draft("Write the report"), crate::v2::auth::LOCAL_USER, tz)
            .expect("create");
        let id = snapshot.tasks[0].id.0.clone();
        assert_eq!(snapshot.tasks[0].title, "Write the report");

        snapshot = app
            .patch(
                &id,
                Patch {
                    title: Some("Write the essay".into()),
                    done: Some(true),
                    ..Patch::default()
                },
                crate::v2::auth::LOCAL_USER,
                tz,
            )
            .expect("patch");

        assert_eq!(
            snapshot.tasks[0].title, "Write the essay",
            "patch is in the answer"
        );
        assert!(snapshot.tasks[0].done, "the done flag is in the answer");

        snapshot = app
            .delete(&id, crate::v2::auth::LOCAL_USER, tz)
            .expect("delete");
        assert!(snapshot.tasks.is_empty(), "the delete is in the answer");
    }
}

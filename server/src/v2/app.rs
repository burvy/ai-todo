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

/// The zone a `DueBy` deadline was set in, and `None` for anything that is not a
/// deadline (docs/ARCHITECTURE.md 4.1): `Anytime` and `At` have no zone of their own.
fn due_zone(when: When, tz: Tz) -> Option<String> {
    match when {
        When::DueBy(_) => Some(tz.name().to_owned()),
        When::Anytime | When::At(_) => None,
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
    /// The `updated_at` of the task as the client last saw it. Absent means "don't check".
    pub updated_at: Option<DateTime<Utc>>,
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
    fn snapshot_of(mut tasks: Vec<Task>, ctx: &LocalContext, tz: Tz) -> Snapshot {
        // Every mutation appends its new tasks after the list it loaded, so the
        // next sort_tasks key (order, created) decides where they appear — that
        // must be true in the answer too, or a freshly created task would look
        // like it landed at the bottom of the list.
        Self::sort_tasks(&mut tasks);
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
            // A deadline remembers the zone it was set in (docs/ARCHITECTURE.md 4.1):
            // editing a task's when re-anchors it to the current zone, and anything
            // that is no longer a deadline has no zone to remember.
            task.when = when;
            task.due_tz = due_zone(when, tz);
        }
        if let Some(done) = patch.done {
            task.done = done;
        }
        if let Some(label) = patch.label {
            task.label = label;
        }
        task.updated_at = ctx.now;
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
                task.updated_at = ctx.now;
                updated.push(task.id.clone());
                if let Some(slot) = tasks.iter_mut().find(|held| held.id == task.id) {
                    *slot = task.clone();
                }
                self.store.put(user, task).map_err(Self::storage_error)?;
                continue;
            }
            let task = Task {
                id: Self::fresh_id(),
                title: event.title,
                duration_min,
                when: When::At(event.start),
                done: false,
                order: 0,
                label: Label::Other,
                priority: Priority::Low,
                source: Source::Calendar,
                created: ctx.now,
                updated_at: ctx.now,
                due_tz: None,
                external_uid: event.uid.clone(),
            };
            created.push(task.id.clone());
            tasks.push(task);
        }
        // The imported batch lands on top in import order: its orders run from
        // one below the smallest held order down to min - created.len().
        let held = tasks.len() - created.len();
        let min = tasks[..held]
            .iter()
            .map(|task| task.order)
            .min()
            .unwrap_or(0);
        let base = min - created.len() as i64;
        for (offset, task) in tasks[held..].iter_mut().enumerate() {
            task.order = base + offset as i64;
            self.store
                .put(user, task.clone())
                .map_err(Self::storage_error)?;
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
        seen: Option<DateTime<Utc>>,
        user: &str,
        tz: Tz,
    ) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        let task = Self::held(&mut tasks, id)?;
        check_fresh(task, seen)?;
        match (revision.day, revision.time) {
            (Some(day), Some(time)) => {
                task.when = When::At(ctx.at(day, time));
                task.due_tz = None;
            }
            (Some(day), None) => {
                let end_of_day = NaiveTime::from_hms_opt(17, 0, 0).expect("17:00 is valid");
                task.when = When::DueBy(ctx.at(day, end_of_day));
                task.due_tz = Some(tz.name().to_owned());
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
        task.updated_at = ctx.now;
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
        // The local copies still carry their old orders; the store renumbered by
        // position. Mirror it so the snapshot we hand back sorts like the next
        // load would (snapshot_of sorts by order).
        for (position, task) in tasks.iter_mut().enumerate() {
            task.order = position as i64;
        }
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

    /// A new task id: 128 random bits, so two users (or two requests) can never be
    /// handed the same one. Ids are one namespace across every user in the database.
    fn fresh_id() -> TaskId {
        TaskId(format!("t{}", uuid::Uuid::new_v4().simple()))
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

/// Refuses an edit made against a version older than the stored one, so the last
/// writer cannot silently undo a change it never saw. No version sent means no check.
fn check_fresh(task: &Task, seen: Option<DateTime<Utc>>) -> Result<(), ApiError> {
    match seen {
        Some(seen) if seen < task.updated_at => Err(ApiError {
            code: ErrorCode::StaleUpdate,
            message: "the task changed since you loaded it; reload and try again".into(),
        }),
        _ => Ok(()),
    }
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

        fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError> {
            self.inner.adopt_anonymous(anonymous, user)
        }

        fn user_plan(&self, user: &str) -> Result<crate::v2::auth::Plan, StoreError> {
            self.inner.user_plan(user)
        }

        fn set_user_plan(&self, user: &str, plan: crate::v2::auth::Plan) -> Result<(), StoreError> {
            self.inner.set_user_plan(user, plan)
        }

        fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError> {
            self.inner.anonymous_exists(user)
        }

        fn ensure_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
            self.inner.ensure_anonymous(user, now)
        }
        fn touch_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
            self.inner.touch_anonymous(user, now)
        }
        fn expire_anonymous(&self, cutoff: DateTime<Utc>) -> Result<usize, StoreError> {
            self.inner.expire_anonymous(cutoff)
        }
        fn record_capture(
            &self,
            user: &str,
            ip: &str,
            day: chrono::NaiveDate,
        ) -> Result<(), StoreError> {
            self.inner.record_capture(user, ip, day)
        }
        fn ip_captures_today(&self, ip: &str, day: chrono::NaiveDate) -> Result<u32, StoreError> {
            self.inner.ip_captures_today(ip, day)
        }
        fn record_anon_issued(&self, ip: &str, day: chrono::NaiveDate) -> Result<u32, StoreError> {
            self.inner.record_anon_issued(ip, day)
        }
        fn block(&self, target: &crate::v2::store::Block) -> Result<(), StoreError> {
            self.inner.block(target)
        }
        fn unblock(&self, target: &crate::v2::store::Block) -> Result<bool, StoreError> {
            self.inner.unblock(target)
        }
        fn is_blocked(&self, user: &str, ip: &str) -> Result<bool, StoreError> {
            self.inner.is_blocked(user, ip)
        }
        fn blocks(&self) -> Result<Vec<crate::v2::store::Block>, StoreError> {
            self.inner.blocks()
        }
        fn usage_today(
            &self,
            day: chrono::NaiveDate,
            top: usize,
        ) -> Result<crate::v2::store::Usage, StoreError> {
            self.inner.usage_today(day, top)
        }
        fn user_id_for_email(&self, email: &str) -> Result<Option<String>, StoreError> {
            self.inner.user_id_for_email(email)
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
    /// Two users creating in the same clock tick used to be handed the same id, because
    /// an id was only checked against its own user's tasks.
    #[test]
    fn two_users_creating_at_the_same_instant_get_different_ids() {
        let clock = Arc::new(FixedClock(
            Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        ));
        let mut app = App::new(MemoryStore::new(), clock);
        let first = app.create(draft("One"), "alice", chrono_tz::UTC).unwrap();
        let second = app.create(draft("Two"), "bob", chrono_tz::UTC).unwrap();
        assert_ne!(first.tasks[0].id, second.tasks[0].id);
        assert_eq!(first.tasks[0].title, "One", "alice's task is untouched");
    }

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

    /// docs/ARCHITECTURE.md 4.1: a `DueBy` deadline keeps the zone it was set in.
    /// `Anytime` and `At` tasks have no zone of their own.
    #[test]
    fn due_tasks_remember_the_zone_they_were_set_in() {
        let clock = Arc::new(FixedClock(
            Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        ));
        let mut app = App::new(MemoryStore::new(), clock);
        let due = Utc.with_ymd_and_hms(2026, 10, 16, 21, 0, 0).unwrap();

        let mut deadline = draft("Deadline");
        deadline.when = When::DueBy(due);
        let created = app.create(deadline, USER, NEW_YORK).expect("create");
        assert_eq!(
            created.tasks[0].due_tz.as_deref(),
            Some("America/New_York"),
            "a deadline remembers the zone it was set in"
        );

        let mut floating = draft("Floating");
        floating.when = When::Anytime;
        assert_eq!(
            app.create(floating, USER, NEW_YORK).expect("create").tasks[0].due_tz,
            None
        );

        let mut pinned = draft("Pinned");
        pinned.when = When::At(due);
        assert_eq!(
            app.create(pinned, USER, NEW_YORK).expect("create").tasks[0].due_tz,
            None
        );
    }

    /// A capture's "day only" deadline is the end of that day in the capture zone:
    /// the instant is fixed with that zone's offset, and the zone name rides with
    /// it. The days either side of the 2026-11-01 DST change still convert to
    /// exactly the end of their local days.
    #[test]
    fn a_deadline_near_the_dst_change_keeps_the_capture_zones_offset() {
        let now = Utc.with_ymd_and_hms(2026, 10, 30, 12, 0, 0).unwrap();
        let clock = Arc::new(FixedClock(now));
        let mut app = App::new(MemoryStore::new(), clock);
        let ctx = LocalContext { now, tz: NEW_YORK };
        let end = NaiveTime::from_hms_opt(23, 59, 59).expect("23:59:59");
        let halloween_eve = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let after_the_change = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();

        let mut before = draft("Before");
        before.when = When::DueBy(ctx.at(halloween_eve, end));
        let mut after = draft("After");
        after.when = When::DueBy(ctx.at(after_the_change, end));

        let (_, snapshot) = app
            .add_drafts(vec![before, after], USER, NEW_YORK)
            .expect("capture");
        let before = snapshot.tasks.iter().find(|t| t.title == "Before").unwrap();
        let after = snapshot.tasks.iter().find(|t| t.title == "After").unwrap();

        let When::DueBy(before_due) = before.when else {
            panic!("expected a deadline")
        };
        let When::DueBy(after_due) = after.when else {
            panic!("expected a deadline")
        };
        // 23:59:59 EDT is 03:59:59Z; 23:59:59 EST is 04:59:59Z.
        assert_eq!(
            before_due,
            Utc.with_ymd_and_hms(2026, 10, 31, 3, 59, 59).unwrap()
        );
        assert_eq!(
            after_due,
            Utc.with_ymd_and_hms(2026, 11, 3, 4, 59, 59).unwrap()
        );
        assert_eq!(
            before_due.with_timezone(&NEW_YORK).time(),
            end,
            "still the end of the local day it was set for"
        );
        assert_eq!(
            after_due.with_timezone(&NEW_YORK).time(),
            end,
            "still the end of the local day it was set for"
        );
        assert_eq!(before.due_tz.as_deref(), Some("America/New_York"));
        assert_eq!(after.due_tz.as_deref(), Some("America/New_York"));
    }

    /// A task keeps its stored zone whatever zone later asks for it: a snapshot from
    /// Los Angeles never rewrites a New York deadline, and the instant never moves.
    #[test]
    fn a_task_keeps_its_due_zone_whatever_zone_asks() {
        let clock = Arc::new(FixedClock(
            Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        ));
        let mut app = App::new(MemoryStore::new(), clock);
        let due = Utc.with_ymd_and_hms(2026, 10, 16, 21, 0, 0).unwrap();
        let mut draft = draft("Call home");
        draft.when = When::DueBy(due);
        app.create(draft, USER, NEW_YORK).expect("create");

        let from_la = app
            .snapshot(USER, chrono_tz::America::Los_Angeles)
            .expect("snapshot");
        assert_eq!(
            from_la.tasks[0].due_tz.as_deref(),
            Some("America/New_York"),
            "reading from another zone must not re-anchor the deadline"
        );
        assert_eq!(from_la.tasks[0].when, When::DueBy(due));
    }

    /// Editing `when` re-anchors a `DueBy` to the zone the edit was made in, and
    /// clears the zone for a task that stops being a deadline.
    #[test]
    fn editing_when_reanchors_or_clears_the_due_zone() {
        let clock = Arc::new(FixedClock(
            Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        ));
        let mut app = App::new(MemoryStore::new(), clock);
        let due = Utc.with_ymd_and_hms(2026, 10, 16, 21, 0, 0).unwrap();
        let mut draft = draft("Deadline");
        draft.when = When::DueBy(due);
        let created = app.create(draft, USER, NEW_YORK).expect("create");
        let id = created.tasks[0].id.0.clone();

        // Pinning it from another zone: no deadline, no zone.
        app.patch(
            &id,
            Patch {
                when: Some(When::At(due)),
                ..Patch::default()
            },
            USER,
            chrono_tz::America::Los_Angeles,
        )
        .expect("patched");
        assert_eq!(app.find(USER, &id).unwrap().due_tz, None);

        // A deadline edited from Los Angeles is anchored there from then on.
        app.patch(
            &id,
            Patch {
                when: Some(When::DueBy(due)),
                ..Patch::default()
            },
            USER,
            chrono_tz::America::Los_Angeles,
        )
        .expect("patched");
        let task = app.find(USER, &id).unwrap();
        assert_eq!(task.due_tz.as_deref(), Some("America/Los_Angeles"));
        assert_eq!(
            task.when,
            When::DueBy(due),
            "the instant itself did not move"
        );
    }

    /// Newest first (docs/ARCHITECTURE.md 4.1): each new task takes a smaller
    /// order than every held task, so the snapshot lists it on top.
    #[test]
    fn newest_task_sorts_first() {
        let (mut app, _) = app();
        let tz = chrono_tz::America::New_York;
        app.create(draft("First"), crate::v2::auth::LOCAL_USER, tz)
            .expect("create first");
        let second = app
            .create(draft("Second"), crate::v2::auth::LOCAL_USER, tz)
            .expect("create second");
        let titles: Vec<&str> = second
            .tasks
            .iter()
            .map(|task| task.title.as_str())
            .collect();
        assert_eq!(
            titles,
            ["Second", "First"],
            "the newer task is listed first"
        );

        // The capture path (add_drafts) lands its whole batch on top too.
        let drafts = vec![draft("Capture one"), draft("Capture two")];
        let (_, capture) = app
            .add_drafts(drafts, crate::v2::auth::LOCAL_USER, tz)
            .expect("capture drafts");
        let titles: Vec<&str> = capture
            .tasks
            .iter()
            .map(|task| task.title.as_str())
            .collect();
        assert_eq!(
            titles,
            ["Capture one", "Capture two", "Second", "First"],
            "a capture sits on top, in capture order"
        );
    }

    /// Manual reorder renumbers every task by position, so it must keep working
    /// when creation gives tasks negative orders, and a task created after a
    /// reorder still appears first.
    #[test]
    fn reorder_keeps_working_with_newest_first() {
        let (mut app, _) = app();
        let tz = chrono_tz::America::New_York;
        app.create(draft("A"), crate::v2::auth::LOCAL_USER, tz)
            .expect("create A");
        let created = app
            .create(draft("B"), crate::v2::auth::LOCAL_USER, tz)
            .expect("create B");
        assert_eq!(
            created
                .tasks
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            ["B", "A"],
            "newest first before any reorder"
        );

        // Put A back on top by hand.
        let ids: Vec<String> = created.tasks.iter().rev().map(|t| t.id.0.clone()).collect();
        let reordered = app
            .reorder(&ids, crate::v2::auth::LOCAL_USER, tz)
            .expect("reorder");
        assert_eq!(
            reordered
                .tasks
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            ["A", "B"],
            "manual reorder still wins"
        );

        // A fresh task goes above the manually ordered pair.
        let newest = app
            .create(draft("C"), crate::v2::auth::LOCAL_USER, tz)
            .expect("create C");
        assert_eq!(
            newest
                .tasks
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            ["C", "A", "B"],
            "after a reorder, the newest task still appears first"
        );
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

    /// A clock the test can move, so a task can be edited "later".
    struct SettableClock(std::sync::Mutex<DateTime<Utc>>);

    impl todo_core::Clock for SettableClock {
        fn now(&self) -> DateTime<Utc> {
            *self.0.lock().expect("clock mutex")
        }
    }

    const NEW_YORK: Tz = chrono_tz::America::New_York;
    const USER: &str = crate::v2::auth::LOCAL_USER;

    /// An app on a clock the test controls, holding one task created at `start`.
    fn app_with_one_task(start: DateTime<Utc>) -> (App<MemoryStore>, Arc<SettableClock>, String) {
        let clock = Arc::new(SettableClock(std::sync::Mutex::new(start)));
        let mut app = App::new(MemoryStore::new(), clock.clone());
        let snapshot = app
            .create(draft("Write the report"), USER, NEW_YORK)
            .unwrap();
        let id = snapshot.tasks[0].id.0.clone();
        (app, clock, id)
    }

    fn stored_updated_at(app: &App<MemoryStore>, id: &str) -> DateTime<Utc> {
        app.find(USER, id).unwrap().updated_at
    }

    fn done_patch(seen: Option<DateTime<Utc>>) -> Patch {
        Patch {
            done: Some(true),
            updated_at: seen,
            ..Patch::default()
        }
    }

    #[test]
    fn a_new_task_is_versioned_when_it_is_created() {
        let start = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        let (app, _, id) = app_with_one_task(start);
        assert_eq!(stored_updated_at(&app, &id), start);
    }

    #[test]
    fn an_accepted_edit_moves_the_version_to_now() {
        let start = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        let (mut app, clock, id) = app_with_one_task(start);
        let later = start + Duration::hours(1);
        *clock.0.lock().unwrap() = later;
        app.patch(&id, done_patch(Some(start)), USER, NEW_YORK)
            .unwrap();
        assert_eq!(stored_updated_at(&app, &id), later);
    }

    #[test]
    fn an_edit_made_against_an_older_version_is_refused_and_changes_nothing() {
        let start = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        let (mut app, clock, id) = app_with_one_task(start);
        // Another device edits at +1h; this one still holds the version from `start`.
        *clock.0.lock().unwrap() = start + Duration::hours(1);
        app.patch(&id, done_patch(Some(start)), USER, NEW_YORK)
            .unwrap();
        *clock.0.lock().unwrap() = start + Duration::hours(2);

        let stale = Patch {
            title: Some("Overwrite".into()),
            updated_at: Some(start),
            ..Patch::default()
        };
        let error = app.patch(&id, stale, USER, NEW_YORK).unwrap_err();

        assert_eq!(error.code, ErrorCode::StaleUpdate);
        let kept = app.find(USER, &id).unwrap();
        assert_eq!(
            kept.title, "Write the report",
            "a refused edit must not apply"
        );
        assert_eq!(kept.updated_at, start + Duration::hours(1));
    }

    #[test]
    fn an_edit_at_the_stored_version_or_newer_or_unversioned_is_accepted() {
        let start = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        for seen in [Some(start), Some(start + Duration::seconds(1)), None] {
            let (mut app, _, id) = app_with_one_task(start);
            app.patch(&id, done_patch(seen), USER, NEW_YORK)
                .unwrap_or_else(|error| panic!("{seen:?} was refused: {error:?}"));
            assert!(app.find(USER, &id).unwrap().done);
        }
    }

    #[test]
    fn a_revision_made_against_an_older_version_is_refused() {
        let start = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        let (mut app, clock, id) = app_with_one_task(start);
        *clock.0.lock().unwrap() = start + Duration::hours(1);
        app.patch(&id, done_patch(Some(start)), USER, NEW_YORK)
            .unwrap();

        let revision = Revision::default();
        let error = app
            .apply_revision(&id, &revision, Some(start), USER, NEW_YORK)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::StaleUpdate);
        app.apply_revision(
            &id,
            &revision,
            Some(start + Duration::hours(1)),
            USER,
            NEW_YORK,
        )
        .expect("the current version is accepted");
    }
}

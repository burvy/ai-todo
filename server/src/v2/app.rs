//! The v2 service. One method per endpoint. Each method loads, changes, re-plans and
//! returns a `Snapshot` in one call, so a snapshot is never half-updated. The HTTP
//! layer holds the `App` lock for the whole call.

use std::collections::HashMap;
use std::sync::Arc;

use capture::Revision;
use chrono::{DateTime, Duration, NaiveTime, Utc};
use chrono_tz::Tz;
use scheduler::{BusyBlock, ScheduledPlacement, WorkingHours};
use serde::Serialize;
use todo_core::{
    ApiError, Clock, ErrorCode, Label, LocalContext, Placement, Plan, Priority, Snapshot, Source,
    Task, TaskDraft, TaskId, When,
};

use super::store::{Store, StoreError};

pub const MIN_DURATION_MIN: u16 = 5;
pub const MAX_DURATION_MIN: u16 = 480;

/// Working hours used when placing floating tasks. Local time, every day.
fn working_hours(tz: Tz) -> WorkingHours {
    WorkingHours::every_day(
        tz,
        NaiveTime::from_hms_opt(9, 0, 0).expect("09:00 is valid"),
        NaiveTime::from_hms_opt(17, 0, 0).expect("17:00 is valid"),
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
            StoreError::Backend(_) => ErrorCode::Unavailable,
        };
        ApiError {
            code,
            message: error.to_string(),
        }
    }

    fn load(&self) -> Result<Vec<Task>, ApiError> {
        let mut tasks = self.store.tasks().map_err(Self::storage_error)?;
        tasks.sort_by(|a, b| a.order.cmp(&b.order).then(a.created.cmp(&b.created)));
        Ok(tasks)
    }

    /// The whole week in one value: tasks, their plan, the zone and the time it was made.
    pub fn snapshot(&self, tz: Tz) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let tasks = self.load()?;
        Ok(Snapshot {
            plan: plan(&tasks, &ctx),
            tasks,
            tz: tz.name().to_owned(),
            generated: ctx.now,
        })
    }

    pub fn create(&mut self, draft: TaskDraft, tz: Tz) -> Result<Snapshot, ApiError> {
        check_title(&draft.title)?;
        check_duration(draft.duration_min)?;
        let ctx = self.ctx(tz);
        let tasks = self.load()?;
        let order = tasks.iter().map(|task| task.order).max().unwrap_or(-1) + 1;
        let task = Task {
            id: self.fresh_id(ctx.now.timestamp_nanos_opt().unwrap_or_default())?,
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
        self.store.put(task).map_err(Self::storage_error)?;
        self.snapshot(tz)
    }

    pub fn patch(&mut self, id: &str, patch: Patch, tz: Tz) -> Result<Snapshot, ApiError> {
        let mut task = self.find(id)?;
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
        self.store.put(task).map_err(Self::storage_error)?;
        self.snapshot(tz)
    }

    /// Import calendar events as `At` tasks. An event whose UID is already a task
    /// updates that task; any other event becomes a new task.
    pub fn import(&mut self, events: Vec<ImportedEvent>, tz: Tz) -> Result<ImportResult, ApiError> {
        let ctx = self.ctx(tz);
        let tasks = self.load()?;
        let mut order = tasks.iter().map(|task| task.order).max().unwrap_or(-1) + 1;
        let mut created = Vec::new();
        let mut updated = Vec::new();
        for event in events {
            let duration_min = event_minutes(event.start, event.end);
            // The store owns the uid index, so this is one lookup per event
            // rather than a map built from every task on the server.
            let existing = match &event.uid {
                Some(uid) => self.store.find_by_uid(uid).map_err(Self::storage_error)?,
                None => None,
            };
            if let Some(mut task) = existing {
                task.title = event.title;
                task.duration_min = duration_min;
                task.when = When::At(event.start);
                updated.push(task.id.clone());
                self.store.put(task).map_err(Self::storage_error)?;
                continue;
            }
            let id = self.fresh_id(ctx.now.timestamp_nanos_opt().unwrap_or_default())?;
            let task = Task {
                id: id.clone(),
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
            created.push(id);
            self.store.put(task).map_err(Self::storage_error)?;
        }
        Ok(ImportResult {
            created,
            updated,
            snapshot: self.snapshot(tz)?,
        })
    }

    /// Adds the drafts a capture produced. Returns their ids and the new snapshot.
    pub fn add_drafts(
        &mut self,
        drafts: Vec<TaskDraft>,
        tz: Tz,
    ) -> Result<(Vec<TaskId>, Snapshot), ApiError> {
        let ctx = self.ctx(tz);
        let tasks = self.load()?;
        let first_order = tasks.iter().map(|task| task.order).max().unwrap_or(-1) + 1;
        let mut ids = Vec::new();
        for (offset, draft) in drafts.into_iter().enumerate() {
            let order = first_order + offset as i64;
            check_title(&draft.title)?;
            check_duration(draft.duration_min)?;
            let id = self.fresh_id(ctx.now.timestamp_nanos_opt().unwrap_or_default())?;
            let task = Task {
                id: id.clone(),
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
            self.store.put(task).map_err(Self::storage_error)?;
            ids.push(id);
        }
        Ok((ids, self.snapshot(tz)?))
    }

    /// Applies what a revision changed. Fields the instruction did not touch stay as they are.
    pub fn apply_revision(
        &mut self,
        id: &str,
        revision: &Revision,
        tz: Tz,
    ) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut task = self.find(id)?;
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
        self.store.put(task).map_err(Self::storage_error)?;
        self.snapshot(tz)
    }

    /// The task's title, for a revision prompt.
    pub fn title_of(&self, id: &str) -> Result<String, ApiError> {
        Ok(self.find(id)?.title)
    }

    /// The app's notion of now.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    pub fn delete(&mut self, id: &str, tz: Tz) -> Result<Snapshot, ApiError> {
        let task_id = self.find(id)?.id;
        self.store.delete(&task_id).map_err(Self::storage_error)?;
        self.snapshot(tz)
    }

    /// `ids` is the full new order: every task exactly once.
    pub fn reorder(&mut self, ids: &[String], tz: Tz) -> Result<Snapshot, ApiError> {
        let tasks = self.load()?;
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
            .set_order(&ordered)
            .map_err(Self::storage_error)?;
        self.snapshot(tz)
    }

    fn find(&self, id: &str) -> Result<Task, ApiError> {
        self.load()?
            .into_iter()
            .find(|task| task.id.0 == id)
            .ok_or_else(|| ApiError {
                code: ErrorCode::NotFound,
                message: "task not found".into(),
            })
    }

    /// An id that is not used yet. The first candidate is the clock in nanoseconds.
    fn fresh_id(&self, seed: i64) -> Result<TaskId, ApiError> {
        let taken: std::collections::HashSet<String> =
            self.load()?.into_iter().map(|task| task.id.0).collect();
        for attempt in 0.. {
            let id = TaskId(format!("t{}", seed + attempt));
            if !taken.contains(&id.0) {
                return Ok(id);
            }
        }
        unreachable!("an unused id exists")
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

fn priority_level(priority: Priority) -> i32 {
    match priority {
        Priority::Low => 0,
        Priority::Medium => 1,
        Priority::High => 2,
        Priority::Critical => 3,
    }
}

/// Where each open task goes. Pinned (`At`) tasks sit exactly where they are and
/// block time. Floating tasks are placed by the scheduler in the free time around them.
/// A floating task that needs more than one block is left unplaced, since v2 keeps one
/// placement per task.
pub fn plan(tasks: &[Task], ctx: &LocalContext) -> Plan {
    let open: Vec<&Task> = tasks.iter().filter(|task| !task.done).collect();
    let mut placements = Vec::new();
    let mut unplaced = Vec::new();
    let mut busy = Vec::new();

    for task in &open {
        if let When::At(start) = task.when {
            let end = start + Duration::minutes(i64::from(task.duration_min));
            busy.push(BusyBlock { start, end });
            placements.push(Placement {
                task: task.id.clone(),
                start,
                end,
            });
        }
    }

    let floating: Vec<scheduler::Task> = open
        .iter()
        .filter(|task| !matches!(task.when, When::At(_)))
        .map(|task| scheduler::Task {
            id: task.id.0.clone(),
            title: task.title.clone(),
            duration_minutes: i64::from(task.duration_min),
            deadline: match task.when {
                When::DueBy(due) => Some(due),
                _ => None,
            },
            priority: priority_level(task.priority),
            position: task.order,
            pinned_start: None,
        })
        .collect();

    let mut chunks: HashMap<String, Vec<ScheduledPlacement>> = HashMap::new();
    for placed in scheduler::schedule(&floating, &busy, &working_hours(ctx.tz), ctx.now) {
        match placed {
            scheduler::Placement::Scheduled(chunk) => {
                chunks.entry(chunk.task_id.clone()).or_default().push(chunk);
            }
            scheduler::Placement::Unschedulable { task_id, .. } => {
                unplaced.push(TaskId(task_id));
            }
        }
    }
    for (id, parts) in chunks {
        match parts.as_slice() {
            [only] => placements.push(Placement {
                task: TaskId(id),
                start: only.start,
                end: only.end,
            }),
            _ => unplaced.push(TaskId(id)),
        }
    }

    placements.sort_by(|a, b| a.start.cmp(&b.start).then(a.task.0.cmp(&b.task.0)));
    unplaced.sort_by(|a, b| a.0.cmp(&b.0));
    Plan {
        placements,
        unplaced,
    }
}

//! The v2 service. One method per endpoint. Each method loads, changes, re-plans and
//! returns a `Snapshot` in one call, so a snapshot is never half-updated. The HTTP
//! layer holds the `App` lock for the whole call.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use capture::Revision;
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use scheduler::{WorkingHours, expand};
use serde::{Deserialize, Serialize};
use todo_core::{
    ApiError, Clock, DaySettings, ErrorCode, ExceptionKind, Label, LocalContext, Occurrence,
    OccurrenceStatus, Plan, Priority, Recurrence, Series, SeriesException, SeriesId, Snapshot,
    Source, Task, TaskDraft, TaskId, When,
};

use super::ics_import::ParsedCalendar;
use super::store::{Store, StoreError};

pub const MIN_DURATION_MIN: u16 = 5;
pub const MAX_DURATION_MIN: u16 = 480;

/// How many days of series occurrences a snapshot carries when the caller did
/// not name a window: the same default the view settings use (today + 7).
const DEFAULT_EXPANSION_DAYS: u32 = 7;

/// Working hours used when placing floating tasks. Local time, every day.
fn working_hours(tz: Tz, settings: &DaySettings) -> WorkingHours {
    WorkingHours {
        day_only_time: settings.placement,
        ..WorkingHours::every_day(
            tz,
            NaiveTime::from_hms_opt(8, 0, 0).expect("08:00 is valid"),
            NaiveTime::from_hms_opt(23, 59, 59).expect("23:59:59 is valid"),
        )
    }
}

/// What a client may change about its day-only times with PATCH /settings. Absent
/// fields stay as they are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct DaySettingsPatch {
    pub deadline: Option<NaiveTime>,
    pub placement: Option<NaiveTime>,
}

/// A time of day to the minute: the settings are `HH:MM`, so seconds are dropped.
fn to_the_minute(time: NaiveTime) -> NaiveTime {
    time.with_second(0)
        .and_then(|time| time.with_nanosecond(0))
        .unwrap_or(time)
}

/// The zone a `DueBy` deadline was set in, and `None` for anything that is not a
/// deadline (docs/ARCHITECTURE.md 4.1): `Anytime` and `At` have no zone of their own.
fn due_zone(when: When, tz: Tz) -> Option<String> {
    match when {
        When::DueBy(_) => Some(tz.name().to_owned()),
        When::Anytime | When::At(_) => None,
    }
}

/// Moves a task to a new `when`, re-anchoring a deadline's `due_tz` only when the
/// instant actually changes (docs/ARCHITECTURE.md 4.1). Every edit path goes through
/// here, so a save that restates the instant the task already has — the edit dialog
/// always sends `when`, and a revision that repeats the same day — keeps the zone the
/// deadline was set in.
fn set_when(task: &mut Task, when: When, tz: Tz) {
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
    /// The `updated_at` of the task as the client last saw it. Absent means "don't check".
    pub updated_at: Option<DateTime<Utc>>,
}

/// The fields a client sends to create a series (ARCHITECTURE.md 7.1).
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct SeriesDraft {
    pub title: String,
    pub duration_min: u16,
    /// `chrono::Weekday` names on the wire: `["Mon", "Thu"]`.
    pub weekdays: Vec<Weekday>,
    pub time_of_day: Option<NaiveTime>,
    pub until: Option<NaiveDate>,
    /// Defaults to today when absent.
    pub start_day: Option<NaiveDate>,
}

/// The fields a client may change on a series. Absent fields stay as they are; for
/// `time_of_day` and `until`, an explicit `null` clears the field.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
pub struct SeriesPatch {
    pub title: Option<String>,
    pub duration_min: Option<u16>,
    pub weekdays: Option<Vec<Weekday>>,
    #[serde(default, deserialize_with = "present_or_null")]
    pub time_of_day: Option<Option<NaiveTime>>,
    #[serde(default, deserialize_with = "present_or_null")]
    pub until: Option<Option<NaiveDate>>,
    pub start_day: Option<NaiveDate>,
}

/// Tells a field that is `null` (`Some(None)`) from one that is missing (`None`),
/// which serde's plain `Option` folds together.
fn present_or_null<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// One occurrence's deviation: `done`, `skipped`, or `moved` to a new `When`.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct OccurrencePatch {
    pub kind: String,
    /// Required for `moved`.
    pub when: Option<When>,
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

    /// This user's day-only deadline and placement times (defaults until changed).
    pub fn day_settings(&self, user: &str) -> Result<DaySettings, ApiError> {
        self.store.day_settings(user).map_err(Self::storage_error)
    }

    /// Changes the fields the patch names and returns the settings that now hold.
    ///
    /// A deadline of 00:00 is refused: it would fall at the very start of the day it is
    /// meant to end, so a "by Friday" task would already be late on Friday morning.
    pub fn patch_day_settings(
        &mut self,
        user: &str,
        patch: DaySettingsPatch,
    ) -> Result<DaySettings, ApiError> {
        let held = self.day_settings(user)?;
        let settings = DaySettings {
            deadline: patch.deadline.map_or(held.deadline, to_the_minute),
            placement: patch.placement.map_or(held.placement, to_the_minute),
        };
        if settings.deadline == NaiveTime::MIN {
            return Err(ApiError {
                code: ErrorCode::BadRequest,
                message: "deadline cannot be 00:00; pick 00:01 to 23:59".into(),
            });
        }
        self.store
            .set_day_settings(user, settings)
            .map_err(Self::storage_error)?;
        Ok(settings)
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

    /// A snapshot over tasks already in hand, with the user's series expanded
    /// for the default window (ARCHITECTURE.md 7.1).
    ///
    /// Every mutating method loads the store once, changes its copy and returns a
    /// snapshot built from that copy. Loading again to return the result doubled the
    /// reads on every write and could show a value the caller never wrote.
    fn snapshot_now(
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
            today + Duration::days(i64::from(DEFAULT_EXPANSION_DAYS)),
        )
    }

    /// The same, with the series expanded over `[window_start, window_end)`.
    ///
    /// The window is the planner's requested range — the same days the view
    /// asks for — not a fixed horizon. Occurrences are ephemeral tasks: skipped
    /// days never appear, done ones appear marked done, moved ones sit at their
    /// new time. An undone past occurrence stays a fixed block on its own day;
    /// routines do not pile up as overdue — only one-shot `DueBy` tasks do.
    fn snapshot_in(
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
    fn occurrence_task(series: &Series, occurrence: &Occurrence) -> Task {
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
        task.updated_at = ctx.now;
        let task = task.clone();
        self.store.put(user, task).map_err(Self::storage_error)?;
        self.snapshot_now(user, tasks, &ctx, tz)
    }

    /// Import calendar events as `At` tasks. An event whose UID is already a task
    /// updates that task; any other event becomes a new task.
    pub fn import(
        &mut self,
        calendar: ParsedCalendar,
        user: &str,
        tz: Tz,
    ) -> Result<ImportResult, ApiError> {
        let ParsedCalendar {
            events,
            series,
            unmapped,
        } = calendar;
        let ctx = self.ctx(tz);
        let mut tasks = self.load(user)?;
        let mut created = Vec::new();
        let mut updated = Vec::new();
        let series_ids = self.import_series(series, user, &ctx)?;
        let held = tasks.len();
        for event in events {
            let duration_min = event_minutes(event.start, event.end);
            // A UID this import already added is not in the store yet: the later copy
            // wins, rather than a second insert breaking the unique index.
            let fresh = event.uid.as_ref().and_then(|uid| {
                tasks[held..]
                    .iter_mut()
                    .find(|task| task.external_uid.as_ref() == Some(uid))
            });
            if let Some(task) = fresh {
                task.title = event.title;
                task.duration_min = duration_min;
                task.when = When::At(event.start);
                continue;
            }
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
                if !updated.contains(&task.id) {
                    updated.push(task.id.clone());
                }
                if let Some(slot) = tasks.iter_mut().find(|held| held.id == task.id) {
                    *slot = task.clone();
                }
                self.store.put(user, task).map_err(Self::storage_error)?;
                continue;
            }
            check_room(tasks.len(), 1)?;
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
            series: series_ids,
            unmapped,
            snapshot: self.snapshot_now(user, tasks, &ctx, tz)?,
        })
    }

    /// Stores the weekly recurrences an import found, returning their ids.
    ///
    /// A UID that is already held updates that series instead of adding a second one,
    /// the same rule one-shot imports follow. An update keeps the series' id, list
    /// position and exceptions: a re-import must not throw away what the user did to
    /// past weeks. Excluded dates are recorded as skipped occurrences, which is how an
    /// `EXDATE` stays excluded after a restart.
    ///
    /// Validates every series before writing any, so a bad series in the middle
    /// does not leave a partial import.
    fn import_series(
        &mut self,
        imported: Vec<ImportedSeries>,
        user: &str,
        ctx: &LocalContext,
    ) -> Result<Vec<SeriesId>, ApiError> {
        let held = self.store.series(user).map_err(Self::storage_error)?;
        let mut next_order = held.iter().map(|series| series.order).min().unwrap_or(0) - 1;

        // Validate everything first — all or nothing.
        let validated: Vec<_> = imported
            .into_iter()
            .map(|series| {
                check_title(&series.title)?;
                check_duration(series.duration_min)?;
                let rule =
                    Recurrence::new(series.weekdays.clone(), series.until).ok_or_else(|| {
                        ApiError {
                            code: ErrorCode::BadRequest,
                            message: "a series must repeat on at least one day".into(),
                        }
                    })?;
                check_span(series.start_day, rule.until)?;
                let existing = series
                    .uid
                    .as_deref()
                    .and_then(|uid| {
                        held.iter()
                            .find(|other| other.external_uid.as_deref() == Some(uid))
                    })
                    .cloned();
                let id = existing
                    .as_ref()
                    .map_or_else(Self::fresh_series_id, |h| h.id.clone());
                let order = existing.as_ref().map_or_else(
                    || {
                        next_order -= 1;
                        next_order
                    },
                    |h| h.order,
                );
                Ok((
                    series,
                    rule,
                    id,
                    order,
                    existing.as_ref().map_or(ctx.now, |h| h.created),
                ))
            })
            .collect::<Result<Vec<_>, ApiError>>()?;

        // All validated — now write.
        let mut ids = Vec::new();
        for (series, rule, id, order, created) in validated {
            let stored = Series {
                id: id.clone(),
                title: series.title.trim().to_owned(),
                duration_min: series.duration_min,
                rule,
                start_day: series.start_day,
                time_of_day: series.time_of_day,
                zone: series.zone,
                external_uid: series.uid,
                created,
                order,
            };
            self.store
                .put_series(user, stored)
                .map_err(Self::storage_error)?;
            for date in series.exdates {
                self.store
                    .put_exception(
                        user,
                        &id,
                        SeriesException {
                            date,
                            kind: ExceptionKind::Skipped,
                        },
                    )
                    .map_err(Self::storage_error)?;
            }
            ids.push(id);
        }
        Ok(ids)
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
        self.snapshot_now(user, tasks, &ctx, tz)
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
        self.snapshot_now(user, tasks, &ctx, tz)
    }

    // ---- series (ARCHITECTURE.md 7.1) ----

    /// Create a weekly series. The zone is stamped from the request, like a
    /// deadline's `due_tz`, so every occurrence keeps its local time.
    pub fn create_series(
        &mut self,
        draft: SeriesDraft,
        user: &str,
        tz: Tz,
    ) -> Result<Snapshot, ApiError> {
        let rule = Self::recurrence_of(draft.weekdays, draft.until)?;
        check_title(&draft.title)?;
        check_duration(draft.duration_min)?;
        let ctx = self.ctx(tz);
        let held = self.store.series(user).map_err(Self::storage_error)?;
        let order = held.iter().map(|series| series.order).min().unwrap_or(0) - 1;
        let series = Series {
            id: Self::fresh_series_id(),
            title: draft.title.trim().to_owned(),
            duration_min: draft.duration_min,
            rule,
            start_day: draft.start_day.unwrap_or_else(|| ctx.today()),
            time_of_day: draft.time_of_day,
            zone: tz.name().to_owned(),
            external_uid: None,
            created: ctx.now,
            order,
        };
        check_span(series.start_day, series.rule.until)?;
        self.store
            .put_series(user, series.clone())
            .map_err(Self::storage_error)?;
        self.snapshot_now(user, self.load(user)?, &ctx, tz)
    }

    /// Edit a series. Past exceptions keep their dates; the rule changes apply
    /// to every occurrence the new rule covers.
    pub fn update_series(
        &mut self,
        id: &str,
        patch: SeriesPatch,
        user: &str,
        tz: Tz,
    ) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut held = self.store.series(user).map_err(Self::storage_error)?;
        let series = Self::held_series(&mut held, id)?;
        if let Some(title) = patch.title {
            check_title(&title)?;
            series.title = title.trim().to_owned();
        }
        if let Some(minutes) = patch.duration_min {
            check_duration(minutes)?;
            series.duration_min = minutes;
        }
        if let Some(weekdays) = patch.weekdays {
            series.rule = Self::recurrence_of(weekdays, series.rule.until)?;
        }
        if let Some(until) = patch.until {
            series.rule.until = until;
        }
        if let Some(time_of_day) = patch.time_of_day {
            series.time_of_day = time_of_day;
        }
        if let Some(start_day) = patch.start_day {
            series.start_day = start_day;
        }
        check_span(series.start_day, series.rule.until)?;
        let series = series.clone();
        self.store
            .put_series(user, series)
            .map_err(Self::storage_error)?;
        self.snapshot_now(user, self.load(user)?, &ctx, tz)
    }

    /// Delete a whole series, its exceptions with it.
    pub fn delete_series(&mut self, id: &str, user: &str, tz: Tz) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut held = self.store.series(user).map_err(Self::storage_error)?;
        let series_id = Self::held_series(&mut held, id)?.id.clone();
        self.store
            .delete_series(user, &series_id)
            .map_err(Self::storage_error)?;
        self.snapshot_now(user, self.load(user)?, &ctx, tz)
    }

    /// Mark one occurrence done, skip it, or move it: one exception row keyed
    /// by the occurrence's local date. The date must be a day the rule covers.
    pub fn patch_occurrence(
        &mut self,
        id: &str,
        date: NaiveDate,
        patch: OccurrencePatch,
        user: &str,
        tz: Tz,
    ) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut held = self.store.series(user).map_err(Self::storage_error)?;
        let series = Self::held_series(&mut held, id)?;
        if !series.rule.covers(series.start_day, date) {
            return Err(ApiError {
                code: ErrorCode::BadRequest,
                message: format!("{date} is not an occurrence of this series"),
            });
        }
        let kind = match patch.kind.as_str() {
            "done" => ExceptionKind::Done,
            "skipped" => ExceptionKind::Skipped,
            "moved" => {
                let to = patch.when.ok_or_else(|| ApiError {
                    code: ErrorCode::BadRequest,
                    message: "a moved occurrence needs a when".into(),
                })?;
                ExceptionKind::Moved {
                    to,
                    to_tz: due_zone(to, tz),
                }
            }
            other => {
                return Err(ApiError {
                    code: ErrorCode::BadRequest,
                    message: format!("unknown occurrence kind {other:?}"),
                });
            }
        };
        self.store
            .put_exception(user, &series.id, SeriesException { date, kind })
            .map_err(Self::storage_error)?;
        self.snapshot_now(user, self.load(user)?, &ctx, tz)
    }

    /// Clear one occurrence's exception, so the day falls back to the rule.
    pub fn clear_occurrence(
        &mut self,
        id: &str,
        date: NaiveDate,
        user: &str,
        tz: Tz,
    ) -> Result<Snapshot, ApiError> {
        let ctx = self.ctx(tz);
        let mut held = self.store.series(user).map_err(Self::storage_error)?;
        let series = Self::held_series(&mut held, id)?;
        let cleared = self
            .store
            .delete_exception(user, &series.id, date)
            .map_err(Self::storage_error)?;
        if !cleared {
            return Err(ApiError {
                code: ErrorCode::NotFound,
                message: "no exception for that date".into(),
            });
        }
        self.snapshot_now(user, self.load(user)?, &ctx, tz)
    }

    /// The user's series with that id, or 404.
    fn held_series<'a>(held: &'a mut [Series], id: &str) -> Result<&'a mut Series, ApiError> {
        held.iter_mut()
            .find(|series| series.id.0 == id)
            .ok_or_else(|| ApiError {
                code: ErrorCode::NotFound,
                message: "series not found".into(),
            })
    }

    /// A rule on a non-empty weekday set; empty is a bad request.
    fn recurrence_of(
        weekdays: Vec<Weekday>,
        until: Option<NaiveDate>,
    ) -> Result<Recurrence, ApiError> {
        Recurrence::new(weekdays, until).ok_or_else(|| ApiError {
            code: ErrorCode::BadRequest,
            message: "a series needs at least one weekday".into(),
        })
    }

    /// A new series id: 128 random bits, the same namespace tasks use. An
    /// occurrence's id is this plus `":{date}"`.
    fn fresh_series_id() -> SeriesId {
        SeriesId(format!("s{}", uuid::Uuid::new_v4().simple()))
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
    fn held<'a>(tasks: &'a mut [Task], id: &str) -> Result<&'a mut Task, ApiError> {
        check_not_occurrence(id)?;
        tasks
            .iter_mut()
            .find(|task| task.id.0 == id)
            .ok_or_else(|| ApiError {
                code: ErrorCode::NotFound,
                message: "task not found".into(),
            })
    }

    fn find(&self, user: &str, id: &str) -> Result<Task, ApiError> {
        check_not_occurrence(id)?;
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
    /// The series an import created or refreshed, by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<SeriesId>,
    /// One line per rule the import could not follow, naming the event and why. That
    /// event is still imported, as one task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmapped: Vec<String>,
    pub snapshot: Snapshot,
}

/// A weekly recurrence from a calendar export, before it becomes a [`Series`].
///
/// The zone is the event's own TZID, not the importing user's: a lecture kept at 09:00
/// New York stays at 09:00 New York for whoever imports it.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedSeries {
    pub uid: Option<String>,
    pub title: String,
    pub duration_min: u16,
    pub weekdays: Vec<Weekday>,
    /// The last local day an occurrence falls on, inclusive. `None` repeats forever.
    pub until: Option<NaiveDate>,
    /// The first local day an occurrence may fall on, inclusive.
    pub start_day: NaiveDate,
    /// The wall-clock time of each occurrence in `zone`; `None` floats it.
    pub time_of_day: Option<NaiveTime>,
    pub zone: String,
    /// Dates the export excludes, each one a skipped occurrence.
    pub exdates: Vec<NaiveDate>,
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

/// The longest title a task or series may have, in characters.
pub const MAX_TITLE_CHARS: usize = 500;
/// The most tasks one person may hold. Every call loads and plans the whole list under
/// one lock, so an unbounded list lets one account slow everyone down.
pub const MAX_TASKS_PER_USER: usize = 5000;

pub fn check_title(title: &str) -> Result<(), ApiError> {
    if title.trim().is_empty() {
        return Err(ApiError {
            code: ErrorCode::BadRequest,
            message: "title is required".into(),
        });
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(ApiError {
            code: ErrorCode::BadRequest,
            message: format!("title is too long (max {MAX_TITLE_CHARS} characters)"),
        });
    }
    Ok(())
}

/// Refuse to grow a list of `held` tasks by `adding` past [`MAX_TASKS_PER_USER`].
pub fn check_room(held: usize, adding: usize) -> Result<(), ApiError> {
    if held.saturating_add(adding) > MAX_TASKS_PER_USER {
        return Err(ApiError {
            code: ErrorCode::BadRequest,
            message: format!("too many tasks (max {MAX_TASKS_PER_USER}); finish or delete some"),
        });
    }
    Ok(())
}

pub fn check_duration(minutes: u16) -> Result<(), ApiError> {
    if !(MIN_DURATION_MIN..=MAX_DURATION_MIN).contains(&minutes) {
        return Err(ApiError {
            code: ErrorCode::BadRequest,
            message: format!("duration_min must be {MIN_DURATION_MIN} to {MAX_DURATION_MIN}"),
        });
    }
    Ok(())
}

/// A series must be able to fire: an end before its first day is a rule that never
/// holds an occurrence, which would sit in the store looking alive.
fn check_span(start_day: NaiveDate, until: Option<NaiveDate>) -> Result<(), ApiError> {
    match until {
        Some(until) if until < start_day => Err(ApiError {
            code: ErrorCode::BadRequest,
            message: "until is before the first day of the series".into(),
        }),
        _ => Ok(()),
    }
}

/// What a task endpoint says when it is handed an occurrence id
/// (`"{series_id}:{date}"`), which names a date rather than a stored task.
/// An occurrence row's id names a date, not a stored task. Finding no task would read
/// as "gone", so every task endpoint says where that row is changed instead.
fn check_not_occurrence(id: &str) -> Result<(), ApiError> {
    match id.contains(':') {
        true => Err(ApiError {
            code: ErrorCode::BadRequest,
            message: "that id names an occurrence; change it at /api/v1/series/{id}/dates/{date}"
                .into(),
        }),
        false => Ok(()),
    }
}

/// Where each open task goes: the scheduler fits floating tasks into the working
/// time around pinned (`At`) tasks, which stay exactly where they are.
pub fn plan(tasks: &[Task], settings: &DaySettings, ctx: &LocalContext) -> Plan {
    scheduler::plan(tasks, &working_hours(ctx.tz, settings), ctx)
}

#[cfg(test)]
mod room_tests {
    use super::*;

    #[test]
    fn a_list_may_fill_to_the_limit_and_no_further() {
        assert!(check_room(MAX_TASKS_PER_USER - 1, 1).is_ok());
        assert!(check_room(MAX_TASKS_PER_USER, 1).is_err());
        assert!(check_room(0, MAX_TASKS_PER_USER + 1).is_err());
        assert!(check_room(usize::MAX, 1).is_err(), "no overflow");
    }
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

        fn series(&self, user: &str) -> Result<Vec<Series>, StoreError> {
            self.inner.series(user)
        }

        fn put_series(&self, user: &str, series: Series) -> Result<(), StoreError> {
            self.inner.put_series(user, series)
        }

        fn delete_series(&self, user: &str, id: &SeriesId) -> Result<bool, StoreError> {
            self.inner.delete_series(user, id)
        }

        fn exceptions(
            &self,
            user: &str,
            id: &SeriesId,
        ) -> Result<Vec<SeriesException>, StoreError> {
            self.inner.exceptions(user, id)
        }

        fn put_exception(
            &self,
            user: &str,
            id: &SeriesId,
            exception: SeriesException,
        ) -> Result<(), StoreError> {
            self.inner.put_exception(user, id, exception)
        }

        fn delete_exception(
            &self,
            user: &str,
            id: &SeriesId,
            date: NaiveDate,
        ) -> Result<bool, StoreError> {
            self.inner.delete_exception(user, id, date)
        }

        fn upsert_user_and_session(
            &self,
            user: &crate::v2::auth::Me,
            token: &str,
            now: chrono::DateTime<Utc>,
            expires_at: chrono::DateTime<Utc>,
        ) -> Result<(), StoreError> {
            self.inner
                .upsert_user_and_session(user, token, now, expires_at)
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

        fn day_settings(&self, user: &str) -> Result<DaySettings, StoreError> {
            self.inner.day_settings(user)
        }

        fn set_day_settings(&self, user: &str, settings: DaySettings) -> Result<(), StoreError> {
            self.inner.set_day_settings(user, settings)
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
        fn usage_ranking(
            &self,
            order: crate::v2::store::UsageOrder,
            top: usize,
        ) -> Result<Vec<crate::v2::store::UsageRow>, StoreError> {
            self.inner.usage_ranking(order, top)
        }
        fn is_user_blocked(&self, user: &str) -> Result<bool, StoreError> {
            self.inner.is_user_blocked(user)
        }
        fn account_exists(&self, user: &str) -> Result<bool, StoreError> {
            self.inner.account_exists(user)
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

    /// The web dialog always sends `when` on save, so a patch that leaves the
    /// instant unchanged must keep the stored zone: a title-only edit made from
    /// another zone must not flip a deadline's `due_tz`.
    #[test]
    fn a_same_when_patch_keeps_a_deadlines_zone() {
        let clock = Arc::new(FixedClock(
            Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        ));
        let mut app = App::new(MemoryStore::new(), clock);
        let due = Utc.with_ymd_and_hms(2026, 10, 16, 21, 0, 0).unwrap();
        let mut draft = draft("Deadline");
        draft.when = When::DueBy(due);
        let created = app.create(draft, USER, NEW_YORK).expect("create");
        let id = created.tasks[0].id.0.clone();

        // A title-only edit from Los Angeles sends `when` unchanged: zone stays.
        app.patch(
            &id,
            Patch {
                title: Some("Renamed".into()),
                when: Some(When::DueBy(due)),
                ..Patch::default()
            },
            USER,
            chrono_tz::America::Los_Angeles,
        )
        .expect("patched");
        let task = app.find(USER, &id).unwrap();
        assert_eq!(task.title, "Renamed");
        assert_eq!(task.when, When::DueBy(due), "the instant did not move");
        assert_eq!(task.due_tz.as_deref(), Some("America/New_York"));

        // Editing the deadline itself from Los Angeles still re-anchors it there:
        // a genuinely new instant gets the zone of the edit.
        app.patch(
            &id,
            Patch {
                when: Some(When::DueBy(due + chrono::Duration::hours(2))),
                ..Patch::default()
            },
            USER,
            chrono_tz::America::Los_Angeles,
        )
        .expect("patched");
        let task = app.find(USER, &id).unwrap();
        assert_eq!(task.when, When::DueBy(due + chrono::Duration::hours(2)));
        assert_eq!(task.due_tz.as_deref(), Some("America/Los_Angeles"));
    }

    /// A revision re-stamps a deadline's zone the way an edit does: repeating the day
    /// the task already has leaves the instant alone, and the zone with it. Toronto and
    /// New York share their offset in October, so both revisions name 23:59 Friday at
    /// the same instant and the zone may only change when that instant really moves.
    #[test]
    fn a_revision_only_re_anchors_the_due_zone_when_it_moves_the_instant() {
        const TORONTO: Tz = chrono_tz::America::Toronto;
        let clock = Arc::new(FixedClock(
            Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        ));
        let mut app = App::new(MemoryStore::new(), clock);
        let due = Utc.with_ymd_and_hms(2026, 10, 17, 3, 59, 0).unwrap();
        let mut draft = draft("Deadline");
        draft.when = When::DueBy(due);
        let created = app.create(draft, USER, TORONTO).expect("create");
        let id = created.tasks[0].id.0.clone();
        assert_eq!(
            app.find(USER, &id).unwrap().due_tz.as_deref(),
            Some("America/Toronto")
        );

        // "Keep it due Friday", asked from New York: the same instant, so the zone the
        // deadline was set in stays put.
        let same_day = Revision {
            day: NaiveDate::from_ymd_opt(2026, 10, 16),
            ..Revision::default()
        };
        app.apply_revision(&id, &same_day, None, USER, NEW_YORK)
            .expect("revised");
        let task = app.find(USER, &id).unwrap();
        assert_eq!(task.when, When::DueBy(due), "the instant did not move");
        assert_eq!(task.due_tz.as_deref(), Some("America/Toronto"));

        // Friday 23:59 from Los Angeles is a different instant, so the deadline takes
        // the zone of the revision that moved it.
        app.apply_revision(&id, &same_day, None, USER, chrono_tz::America::Los_Angeles)
            .expect("revised");
        let task = app.find(USER, &id).unwrap();
        assert_eq!(
            task.when,
            When::DueBy(Utc.with_ymd_and_hms(2026, 10, 17, 6, 59, 0).unwrap())
        );
        assert_eq!(task.due_tz.as_deref(), Some("America/Los_Angeles"));
    }

    /// A revision that pins a deadline to a clock time makes it an `At` task, which has
    /// no zone of its own.
    #[test]
    fn a_revision_that_pins_a_deadline_clears_its_zone() {
        let clock = Arc::new(FixedClock(
            Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        ));
        let mut app = App::new(MemoryStore::new(), clock);
        let due = Utc.with_ymd_and_hms(2026, 10, 16, 21, 0, 0).unwrap();
        let mut draft = draft("Deadline");
        draft.when = When::DueBy(due);
        let created = app.create(draft, USER, NEW_YORK).expect("create");
        let id = created.tasks[0].id.0.clone();

        let pinned = Revision {
            day: NaiveDate::from_ymd_opt(2026, 10, 16),
            time: NaiveTime::from_hms_opt(9, 0, 0),
            ..Revision::default()
        };
        app.apply_revision(&id, &pinned, None, USER, NEW_YORK)
            .expect("revised");
        let task = app.find(USER, &id).unwrap();
        assert_eq!(
            task.when,
            When::At(Utc.with_ymd_and_hms(2026, 10, 16, 13, 0, 0).unwrap())
        );
        assert_eq!(task.due_tz, None);
    }

    /// A time with no day names no date to put it on, so the task keeps its own.
    #[test]
    fn a_revision_with_a_time_but_no_day_leaves_the_deadline_alone() {
        let clock = Arc::new(FixedClock(
            Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        ));
        let mut app = App::new(MemoryStore::new(), clock);
        let due = Utc.with_ymd_and_hms(2026, 10, 16, 21, 0, 0).unwrap();
        let mut draft = draft("Deadline");
        draft.when = When::DueBy(due);
        let created = app.create(draft, USER, NEW_YORK).expect("create");
        let id = created.tasks[0].id.0.clone();

        let time_only = Revision {
            time: NaiveTime::from_hms_opt(9, 0, 0),
            ..Revision::default()
        };
        app.apply_revision(&id, &time_only, None, USER, NEW_YORK)
            .expect("revised");
        assert_eq!(app.find(USER, &id).unwrap().when, When::DueBy(due));
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

    /// Rambo's "Edit" steps a task's priority with a PATCH, so the field must be read,
    /// not dropped as unknown (FEATURES 4a).
    #[test]
    fn a_patch_changes_the_priority() {
        let clock = Arc::new(FixedClock(
            Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
        ));
        let mut app = App::new(MemoryStore::new(), clock);
        let created = app.create(draft("Taxes"), USER, NEW_YORK).expect("create");
        let id = created.tasks[0].id.0.clone();
        let patch: Patch = serde_json::from_str(r#"{"priority":"high"}"#).expect("parses");
        app.patch(&id, patch, USER, NEW_YORK).expect("patched");
        assert_eq!(app.find(USER, &id).unwrap().priority, Priority::High);
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

    // -- series (docs/ARCHITECTURE.md 7.1)

    /// Sunday 2026-10-11 21:00 in New York, the suite's usual clock.
    fn sunday_night() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    /// A Monday and Thursday series at 07:00, starting Monday 2026-10-12.
    fn series_draft() -> SeriesDraft {
        SeriesDraft {
            title: "Gym".into(),
            duration_min: 60,
            weekdays: vec![Weekday::Mon, Weekday::Thu],
            time_of_day: NaiveTime::from_hms_opt(7, 0, 0),
            until: None,
            start_day: Some(day(2026, 10, 12)),
        }
    }

    fn mark(kind: &str, when: Option<When>) -> OccurrencePatch {
        OccurrencePatch {
            kind: kind.into(),
            when,
        }
    }

    /// An app on a fixed clock holding one Monday/Thursday series.
    fn app_with_one_series() -> (App<MemoryStore>, String) {
        let mut app = App::new(MemoryStore::new(), Arc::new(FixedClock(sunday_night())));
        let snapshot = app
            .create_series(series_draft(), USER, NEW_YORK)
            .expect("create series");
        let id = snapshot.series[0].id.0.clone();
        (app, id)
    }

    /// The occurrence rows of a snapshot, which are the ones a series produced.
    fn occurrences(snapshot: &Snapshot) -> Vec<&Task> {
        snapshot
            .tasks
            .iter()
            .filter(|task| task.source == Source::Series)
            .collect()
    }

    fn row<'a>(snapshot: &'a Snapshot, id: &str) -> &'a Task {
        snapshot
            .tasks
            .iter()
            .find(|task| task.id.0 == id)
            .unwrap_or_else(|| panic!("{id} is not in the snapshot"))
    }

    fn placed_at(snapshot: &Snapshot, id: &str) -> Option<DateTime<Utc>> {
        snapshot
            .plan
            .placements
            .iter()
            .find(|placement| placement.task.0 == id)
            .map(|placement| placement.start)
    }

    /// A series shows as one row per day it repeats on inside the window, each named
    /// after its own date, each placed at the series' local time.
    #[test]
    fn a_new_series_shows_the_occurrences_of_the_week() {
        let (app, id) = app_with_one_series();
        let snapshot = app.snapshot(USER, NEW_YORK).expect("snapshot");
        assert_eq!(
            occurrences(&snapshot)
                .iter()
                .map(|row| row.id.0.clone())
                .collect::<Vec<String>>(),
            vec![format!("{id}:2026-10-12"), format!("{id}:2026-10-15")],
            "one row per repeated day in the week, named after its own date"
        );

        let monday = row(&snapshot, &format!("{id}:2026-10-12"));
        assert_eq!(monday.title, "Gym");
        assert!(!monday.done);
        // 07:00 New York on 2026-10-12 is 11:00Z (EDT), and the row is pinned there.
        assert_eq!(monday.when, When::At(utc(2026, 10, 12, 11)));
        assert_eq!(
            placed_at(&snapshot, &monday.id.0),
            Some(utc(2026, 10, 12, 11))
        );
    }

    /// The expansion follows the range the planner asked for, not a fixed horizon.
    #[test]
    fn occurrences_follow_the_planner_window() {
        let (app, id) = app_with_one_series();

        let next_week = app
            .snapshot_in_window(USER, NEW_YORK, day(2026, 10, 19), 7)
            .expect("snapshot");
        assert_eq!(
            occurrences(&next_week)
                .iter()
                .map(|row| row.id.0.clone())
                .collect::<Vec<String>>(),
            vec![format!("{id}:2026-10-19"), format!("{id}:2026-10-22")],
            "the same two days, one week later"
        );

        let narrow = app
            .snapshot_in_window(USER, NEW_YORK, day(2026, 10, 13), 1)
            .expect("snapshot");
        assert!(
            occurrences(&narrow).is_empty(),
            "a Tuesday holds no occurrence of a Monday/Thursday series"
        );
    }

    /// Marking an occurrence done writes an exception, not a task: the row is still
    /// there, marked done and unplaced, and a fresh app over the same database still
    /// sees it that way.
    #[test]
    fn a_done_occurrence_stays_done_after_a_restart() {
        // Its own folder: removing it must never touch the shared temp dir.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("rambl.db");
        let store = crate::v2::store::SqliteStore::open(&path).expect("open database");
        let mut app = App::new(store, Arc::new(FixedClock(sunday_night())));
        let created = app
            .create_series(series_draft(), USER, NEW_YORK)
            .expect("create series");
        let id = created.series[0].id.0.clone();
        let marked = app
            .patch_occurrence(&id, day(2026, 10, 12), mark("done", None), USER, NEW_YORK)
            .expect("marked done");
        assert!(
            row(&marked, &format!("{id}:2026-10-12")).done,
            "the row is marked done"
        );
        assert_eq!(
            placed_at(&marked, &format!("{id}:2026-10-12")),
            None,
            "a done occurrence holds no slot"
        );
        assert_eq!(occurrences(&marked).len(), 2, "the week is unchanged");
        drop(app);

        // A fresh handle on the same file: what a restart sees.
        let restarted = App::new(
            crate::v2::store::SqliteStore::open(&path).expect("reopen"),
            Arc::new(FixedClock(sunday_night())),
        );
        let after = restarted.snapshot(USER, NEW_YORK).expect("snapshot");
        assert!(
            row(&after, &format!("{id}:2026-10-12")).done,
            "the exception survives a restart"
        );
    }

    /// A skipped day has no row at all; a moved one sits at its new time under its
    /// original date, and the slot the rule held is free.
    #[test]
    fn a_skipped_occurrence_is_gone_and_a_moved_one_sits_at_its_new_time() {
        let (mut app, id) = app_with_one_series();
        let skipped = app
            .patch_occurrence(
                &id,
                day(2026, 10, 12),
                mark("skipped", None),
                USER,
                NEW_YORK,
            )
            .expect("skipped");
        assert!(
            !occurrences(&skipped)
                .iter()
                .any(|row| row.id.0.ends_with("2026-10-12")),
            "a skipped day produces no row"
        );

        let moved_to = utc(2026, 10, 12, 18);
        let moved = app
            .patch_occurrence(
                &id,
                day(2026, 10, 12),
                mark("moved", Some(When::At(moved_to))),
                USER,
                NEW_YORK,
            )
            .expect("moved");
        assert_eq!(
            row(&moved, &format!("{id}:2026-10-12")).when,
            When::At(moved_to),
            "the row keeps its original date and takes the new time"
        );
        assert_eq!(
            placed_at(&moved, &format!("{id}:2026-10-12")),
            Some(moved_to)
        );
        assert_eq!(
            placed_at(&moved, &format!("{id}:2026-10-15")),
            Some(utc(2026, 10, 15, 11)),
            "the series' own time is untouched elsewhere"
        );

        // Clearing the exception puts the day back to the rule.
        let cleared = app
            .clear_occurrence(&id, day(2026, 10, 12), USER, NEW_YORK)
            .expect("cleared");
        assert_eq!(
            row(&cleared, &format!("{id}:2026-10-12")).when,
            When::At(utc(2026, 10, 12, 11)),
            "with no exception the rule applies again"
        );
    }

    /// A client cannot mark a day the series does not repeat, cannot reach another
    /// person's series by naming it, and cannot leave the kind open.
    #[test]
    fn an_occurrence_outside_the_rule_or_another_user_is_refused() {
        let (mut app, id) = app_with_one_series();
        for date in [day(2026, 10, 13), day(2026, 10, 5)] {
            let refused = app
                .patch_occurrence(&id, date, mark("done", None), USER, NEW_YORK)
                .expect_err("a day the series does not repeat");
            assert_eq!(refused.code, ErrorCode::BadRequest, "{date}");
        }
        assert_eq!(
            app.patch_occurrence(
                "s-not-mine",
                day(2026, 10, 12),
                mark("done", None),
                USER,
                NEW_YORK
            )
            .expect_err("another user's series")
            .code,
            ErrorCode::NotFound
        );
        assert_eq!(
            app.patch_occurrence(&id, day(2026, 10, 12), mark("moved", None), USER, NEW_YORK)
                .expect_err("a move with no instant")
                .code,
            ErrorCode::BadRequest
        );
        assert_eq!(
            app.patch_occurrence(
                &id,
                day(2026, 10, 12),
                mark("whenever", None),
                USER,
                NEW_YORK
            )
            .expect_err("a kind that is not one of the three")
            .code,
            ErrorCode::BadRequest
        );
    }

    /// A rule that can never fire is refused: no days at all, or an end before the
    /// first day. Nothing is stored either way.
    #[test]
    fn a_series_that_could_never_fire_is_refused() {
        let mut app = App::new(MemoryStore::new(), Arc::new(FixedClock(sunday_night())));
        let no_days = SeriesDraft {
            weekdays: vec![],
            ..series_draft()
        };
        assert_eq!(
            app.create_series(no_days, USER, NEW_YORK)
                .expect_err("no days")
                .code,
            ErrorCode::BadRequest
        );
        let backwards = SeriesDraft {
            until: Some(day(2026, 10, 1)),
            ..series_draft()
        };
        assert_eq!(
            app.create_series(backwards, USER, NEW_YORK)
                .expect_err("an end before the first day")
                .code,
            ErrorCode::BadRequest
        );
        assert!(app.store.series(USER).unwrap().is_empty());
    }

    /// An edit changes the rule every occurrence follows, and deleting a series takes
    /// its rows with it.
    #[test]
    fn editing_a_series_changes_its_occurrences_and_deleting_takes_them_all() {
        let (mut app, id) = app_with_one_series();
        let edited = app
            .update_series(
                &id,
                SeriesPatch {
                    title: Some("Yoga".into()),
                    weekdays: Some(vec![Weekday::Wed]),
                    ..SeriesPatch::default()
                },
                USER,
                NEW_YORK,
            )
            .expect("edited");
        assert_eq!(edited.series[0].title, "Yoga");
        assert_eq!(
            occurrences(&edited)
                .iter()
                .map(|row| row.id.0.clone())
                .collect::<Vec<String>>(),
            vec![format!("{id}:2026-10-14")],
            "only Wednesdays now"
        );

        let gone = app.delete_series(&id, USER, NEW_YORK).expect("deleted");
        assert!(gone.series.is_empty());
        assert!(occurrences(&gone).is_empty());
        assert_eq!(
            app.delete_series(&id, USER, NEW_YORK)
                .expect_err("the second time")
                .code,
            ErrorCode::NotFound
        );
    }

    /// A series keeps its local time across a clock change: 07:00 New York is 12:00Z
    /// before the March change and 11:00Z after it.
    #[test]
    fn a_series_keeps_its_local_time_across_the_dst_change() {
        let mut app = App::new(MemoryStore::new(), Arc::new(FixedClock(sunday_night())));
        app.create_series(
            SeriesDraft {
                weekdays: vec![Weekday::Sun],
                start_day: Some(day(2026, 3, 1)),
                ..series_draft()
            },
            USER,
            NEW_YORK,
        )
        .expect("create series");

        let around = app
            .snapshot_in_window(USER, NEW_YORK, day(2026, 3, 1), 14)
            .expect("snapshot");
        let sunday: Vec<DateTime<Utc>> = occurrences(&around)
            .iter()
            .map(|row| match row.when {
                When::At(start) => start,
                other => panic!("an occurrence at a time, got {other:?}"),
            })
            .collect();
        assert_eq!(
            sunday,
            vec![utc(2026, 3, 1, 12), utc(2026, 3, 8, 11)],
            "07:00 local is 12:00Z in EST and 11:00Z in EDT"
        );
    }

    /// One person's series never reaches another's snapshot, and cannot be deleted by
    /// name from the other side.
    #[test]
    fn a_series_is_scoped_to_its_owner() {
        let (mut app, id) = app_with_one_series();
        let mine = app.snapshot(USER, NEW_YORK).expect("mine");
        assert_eq!(occurrences(&mine).len(), 2);
        let theirs = app.snapshot("someone-else", NEW_YORK).expect("theirs");
        assert!(
            occurrences(&theirs).is_empty() && theirs.series.is_empty(),
            "another person sees nothing"
        );

        assert_eq!(
            app.delete_series(&id, "someone-else", NEW_YORK)
                .expect_err("another user cannot delete it")
                .code,
            ErrorCode::NotFound
        );
        assert_eq!(app.snapshot(USER, NEW_YORK).unwrap().series.len(), 1);
    }

    /// An occurrence row's id names a date, not a stored task, so a task endpoint handed
    /// one says where to send it rather than reporting a task that is not there.
    #[test]
    fn an_occurrence_id_cannot_be_patched_as_a_task() {
        let (mut app, id) = app_with_one_series();
        let occurrence = format!("{id}:2026-10-12");
        let errors = [
            app.patch(&occurrence, done_patch(None), USER, NEW_YORK)
                .expect_err("an occurrence is not a task"),
            app.delete(&occurrence, USER, NEW_YORK)
                .expect_err("nor can it be deleted as one"),
            app.title_of(USER, &occurrence)
                .expect_err("nor revised as one"),
        ];
        for error in errors {
            assert_eq!(error.code, ErrorCode::BadRequest);
            assert!(
                error.message.contains("/dates/"),
                "the message must name the endpoint: {}",
                error.message
            );
        }
    }

    /// An edit that leaves the series unable to fire is refused as well, not only one
    /// that creates it that way.
    #[test]
    fn an_edit_cannot_leave_a_series_that_could_never_fire() {
        let (mut app, id) = app_with_one_series();
        let error = app
            .update_series(
                &id,
                SeriesPatch {
                    // An end before the first day: no occurrence can ever fall inside it.
                    until: Some(Some(day(2026, 10, 20))),
                    start_day: Some(day(2026, 11, 1)),
                    ..SeriesPatch::default()
                },
                USER,
                NEW_YORK,
            )
            .expect_err("an end before the first day");
        assert_eq!(error.code, ErrorCode::BadRequest);
        let held = &app.snapshot(USER, NEW_YORK).unwrap().series[0];
        assert_eq!(
            held.start_day,
            day(2026, 10, 12),
            "the refused edit stored nothing"
        );
        assert_eq!(held.rule.until, None);
    }

    /// On the wire, `null` clears `time_of_day` and `until`, while a missing field
    /// leaves them as they are.
    #[test]
    fn a_null_series_field_clears_it_and_a_missing_one_keeps_it() {
        let patch: SeriesPatch = serde_json::from_str("{}").unwrap();
        assert_eq!((patch.time_of_day, patch.until), (None, None));
        let patch: SeriesPatch =
            serde_json::from_str(r#"{"time_of_day":null,"until":null}"#).unwrap();
        assert_eq!((patch.time_of_day, patch.until), (Some(None), Some(None)));
        let patch: SeriesPatch =
            serde_json::from_str(r#"{"time_of_day":"09:30:00","until":"2026-12-01"}"#).unwrap();
        assert_eq!(
            (patch.time_of_day, patch.until),
            (
                Some(NaiveTime::from_hms_opt(9, 30, 0)),
                Some(Some(day(2026, 12, 1)))
            )
        );
    }

    /// Clearing the end makes the series repeat forever again, and clearing the time
    /// makes its occurrences all-day.
    #[test]
    fn an_edit_can_clear_a_series_end_and_time() {
        let (mut app, id) = app_with_one_series();
        let set = SeriesPatch {
            until: Some(Some(day(2026, 11, 1))),
            ..SeriesPatch::default()
        };
        app.update_series(&id, set, USER, NEW_YORK).unwrap();
        let clear = SeriesPatch {
            time_of_day: Some(None),
            until: Some(None),
            ..SeriesPatch::default()
        };
        app.update_series(&id, clear, USER, NEW_YORK).unwrap();
        let held = &app.snapshot(USER, NEW_YORK).unwrap().series[0];
        assert_eq!(held.rule.until, None);
        assert_eq!(held.time_of_day, None);
        let keep = SeriesPatch {
            title: Some("Run".into()),
            ..SeriesPatch::default()
        };
        app.update_series(&id, keep, USER, NEW_YORK).unwrap();
        let held = &app.snapshot(USER, NEW_YORK).unwrap().series[0];
        assert_eq!(held.title, "Run");
        assert_eq!(held.time_of_day, None, "a missing field changes nothing");
    }

    /// An `until` sent with a new series is kept: a rule that ends is not one that
    /// repeats forever.
    #[test]
    fn a_series_is_created_with_the_end_it_was_given() {
        let mut app = App::new(MemoryStore::new(), Arc::new(FixedClock(sunday_night())));
        let until = day(2026, 10, 13);
        let snapshot = app
            .create_series(
                SeriesDraft {
                    until: Some(until),
                    ..series_draft()
                },
                USER,
                NEW_YORK,
            )
            .expect("created");
        assert_eq!(snapshot.series[0].rule.until, Some(until));
        // Only the Monday inside the span expands; the Thursday after it does not.
        assert_eq!(
            occurrences(&snapshot)
                .iter()
                .map(|row| row.id.0.clone())
                .collect::<Vec<String>>(),
            vec![format!("{}:2026-10-12", snapshot.series[0].id.0)],
        );
    }
}

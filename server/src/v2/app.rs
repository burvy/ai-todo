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
use super::tuning::{MAX_TASKS_PER_USER, MAX_TITLE_CHARS};
use todo_core::tuning::DEFAULT_WINDOW_DAYS;
use todo_core::tuning::{TASK_MAX_DURATION_MIN, TASK_MIN_DURATION_MIN};

/// Working hours used when placing floating tasks. Local time, every day.
fn working_hours(tz: Tz, settings: &DaySettings) -> WorkingHours {
    WorkingHours {
        day_only_time: settings.placement,
        ..WorkingHours::every_day(
            tz,
            todo_core::tuning::WORKDAY_START,
            todo_core::tuning::WORKDAY_END,
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

/// A restored task's place: where it was, kept within one step of the list as it is
/// now. Its relative place survives, and a hostile `i64::MIN` cannot make the next
/// capture's `min_order - n` overflow.
fn restored_order(order: i64, tasks: &[Task]) -> i64 {
    let orders = || tasks.iter().map(|task| task.order);
    let low = orders().min().unwrap_or(0).saturating_sub(1);
    let high = orders().max().unwrap_or(0).saturating_add(1);
    order.clamp(low, high)
}

/// A restored deadline keeps the zone it was set in when that names a real zone;
/// anything else is re-anchored where the user is now, as an edit would.
fn restored_zone(task: &Task, tz: Tz) -> Option<String> {
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
    /// Take the task off the planner, or put it back. Absent leaves it as it is, so
    /// this is the whole of "off the planner" for a client (ARCHITECTURE.md 4.2).
    pub off_planner: Option<bool>,
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
                off_planner: false,
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
                let rule = if series.weekdays.is_empty() {
                    Recurrence::daily(series.interval, series.until).ok_or_else(|| ApiError {
                        code: ErrorCode::BadRequest,
                        message: "a daily series needs an interval of 1 or more".into(),
                    })?
                } else {
                    Recurrence::with_interval(
                        series.weekdays.clone(),
                        series.interval,
                        series.until,
                    )
                    .ok_or_else(|| ApiError {
                        code: ErrorCode::BadRequest,
                        message: "a series must repeat on at least one day".into(),
                    })?
                };
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

/// A recurrence from a calendar export, before it becomes a [`Series`].
///
/// The zone is the event's own TZID, not the importing user's: a lecture kept at 09:00
/// New York stays at 09:00 New York for whoever imports it.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedSeries {
    pub uid: Option<String>,
    pub title: String,
    pub duration_min: u16,
    /// Weekdays for a weekly rule; empty for a daily rule.
    pub weekdays: Vec<Weekday>,
    /// How often the rule fires: every `interval` weeks (weekly) or days (daily).
    pub interval: u32,
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
    minutes.clamp(
        i64::from(TASK_MIN_DURATION_MIN),
        i64::from(TASK_MAX_DURATION_MIN),
    ) as u16
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
    if !(TASK_MIN_DURATION_MIN..=TASK_MAX_DURATION_MIN).contains(&minutes) {
        return Err(ApiError {
            code: ErrorCode::BadRequest,
            message: format!(
                "duration_min must be {TASK_MIN_DURATION_MIN} to {TASK_MAX_DURATION_MIN}"
            ),
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
mod tests;

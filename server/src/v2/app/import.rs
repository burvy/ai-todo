//! Calendar import: events into tasks and series.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc, Weekday};
use chrono_tz::Tz;
use serde::Serialize;
use todo_core::{
    ApiError, ErrorCode, ExceptionKind, Label, LocalContext, Priority, Recurrence, Series,
    SeriesException, SeriesId, Snapshot, Source, Task, TaskId, When,
};

use super::{App, check_duration, check_room, check_span, check_title};
use crate::v2::ics_import::ParsedCalendar;
use crate::v2::store::Store;
use todo_core::tuning::{TASK_MAX_DURATION_MIN, TASK_MIN_DURATION_MIN};

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

impl<S: Store> App<S> {
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
    pub(super) fn import_series(
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
}

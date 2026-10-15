//! The v2 service. One method per endpoint. Each method loads, changes, re-plans and
//! returns a `Snapshot` in one call, so a snapshot is never half-updated. The HTTP
//! layer holds the `App` lock for the whole call.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use todo_core::{ApiError, Clock, DaySettings, ErrorCode, LocalContext, Plan, Task};

mod import;
mod series;
mod settings;
mod snapshot;
mod tasks;

pub use import::{ImportResult, ImportedEvent, ImportedSeries};
pub use series::{OccurrencePatch, SeriesDraft, SeriesPatch};
pub use settings::DaySettingsPatch;
use settings::working_hours;
pub use tasks::Patch;

use super::store::{Store, StoreError};
use super::tuning::{MAX_TASKS_PER_USER, MAX_TITLE_CHARS};
use todo_core::tuning::{TASK_MAX_DURATION_MIN, TASK_MIN_DURATION_MIN};

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

    /// The app's notion of now.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// The store itself, for the account calls in [`super::auth`], which work on
    /// users and sessions rather than on tasks.
    pub fn store(&self) -> &S {
        &self.store
    }
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

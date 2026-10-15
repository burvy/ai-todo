//! Writing repeating series and their per-date exceptions (ARCHITECTURE.md 7.1).

use chrono::{NaiveDate, NaiveTime, Weekday};
use chrono_tz::Tz;
use serde::Deserialize;
use todo_core::{
    ApiError, ErrorCode, ExceptionKind, Recurrence, Series, SeriesException, SeriesId, Snapshot,
    When,
};

use super::tasks::due_zone;
use super::{App, check_duration, check_span, check_title};
use crate::v2::store::Store;

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

impl<S: Store> App<S> {
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
    pub(super) fn held_series<'a>(
        held: &'a mut [Series],
        id: &str,
    ) -> Result<&'a mut Series, ApiError> {
        held.iter_mut()
            .find(|series| series.id.0 == id)
            .ok_or_else(|| ApiError {
                code: ErrorCode::NotFound,
                message: "series not found".into(),
            })
    }

    /// A rule on a non-empty weekday set; empty is a bad request.
    pub(super) fn recurrence_of(
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
    pub(super) fn fresh_series_id() -> SeriesId {
        SeriesId(format!("s{}", uuid::Uuid::new_v4().simple()))
    }
}

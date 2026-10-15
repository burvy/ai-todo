//! The day-only times the gear menu sets, and the working hours they give the scheduler.

use chrono::{NaiveTime, Timelike};
use chrono_tz::Tz;
use scheduler::WorkingHours;
use todo_core::{ApiError, DaySettings, ErrorCode};

use super::App;
use crate::v2::store::Store;

/// Working hours used when placing floating tasks. Local time, every day.
pub(super) fn working_hours(tz: Tz, settings: &DaySettings) -> WorkingHours {
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

impl<S: Store> App<S> {
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
}

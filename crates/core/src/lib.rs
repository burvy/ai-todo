//! Shared v2 data model (ARCHITECTURE.md section 4). Pure: chrono, chrono-tz and serde
//! only, so it builds for wasm. The v1 types stay in [`v1`] until the v1 server is retired.

pub mod clock;
pub mod v1;

pub use clock::{Clock, FixedClock, SystemClock};

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// Every time-dependent step goes through this. Build it once per request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalContext {
    pub now: DateTime<Utc>,
    pub tz: Tz,
}

impl LocalContext {
    /// The user's local calendar date right now.
    pub fn today(&self) -> NaiveDate {
        self.local(self.now).date()
    }

    /// `t` as local wall-clock time in the user's zone.
    pub fn local(&self, t: DateTime<Utc>) -> NaiveDateTime {
        t.with_timezone(&self.tz).naive_local()
    }

    /// The instant at local wall-clock `d` `t`.
    ///
    /// On a DST overlap the earlier instant wins. A time that does not exist (the
    /// spring-forward gap) moves forward one hour, so 02:30 becomes 03:30.
    pub fn at(&self, d: NaiveDate, t: NaiveTime) -> DateTime<Utc> {
        let naive = d.and_time(t);
        self.tz
            .from_local_datetime(&naive)
            .earliest()
            .or_else(|| {
                self.tz
                    .from_local_datetime(&(naive + Duration::hours(1)))
                    .earliest()
            })
            .unwrap_or_else(|| self.tz.from_utc_datetime(&naive))
            .with_timezone(&Utc)
    }
}

/// When a task should happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "time", rename_all = "snake_case")]
pub enum When {
    /// Place in the first free slot.
    Anytime,
    /// Place before this instant.
    DueBy(DateTime<Utc>),
    /// Place exactly at this instant (pinned).
    At(DateTime<Utc>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    School,
    Work,
    Errands,
    Health,
    Personal,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Voice,
    Pdf,
    Email,
    Text,
    Calendar,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub duration_min: u16,
    pub when: When,
    pub done: bool,
    pub order: i64,
    pub label: Label,
    pub priority: Priority,
    pub source: Source,
    pub created: DateTime<Utc>,
    /// iCalendar UID for imported events. Re-importing the same UID updates this task.
    pub external_uid: Option<String>,
}

/// A task before the server gives it an id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskDraft {
    pub title: String,
    pub duration_min: u16,
    pub when: When,
    pub label: Label,
    pub priority: Priority,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub task: TaskId,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub placements: Vec<Placement>,
    pub unplaced: Vec<TaskId>,
}

/// Everything the client needs to draw the list and the week, from one call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub tasks: Vec<Task>,
    pub plan: Plan,
    pub tz: String,
    pub generated: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    NotFound,
    Conflict,
    /// A dependency (Jev, a database) is temporarily down: 503, try again later.
    Unavailable,
    Upstream,
    /// This server's own fault, such as a storage failure: 500.
    Internal,
}

/// The inner `error` object of a failed response: `{ "error": { "code", "message" } }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn context(now: DateTime<Utc>) -> LocalContext {
        LocalContext {
            now,
            tz: chrono_tz::America::New_York,
        }
    }

    #[test]
    fn today_is_the_local_date_not_the_utc_date() {
        // Sunday 21:00 in New York is already Monday in UTC.
        let now = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        assert_eq!(
            context(now).today(),
            NaiveDate::from_ymd_opt(2026, 10, 11).unwrap()
        );
    }

    #[test]
    fn spring_forward_gap_moves_forward_an_hour() {
        let ctx = context(Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap());
        let day = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let t = NaiveTime::from_hms_opt(2, 30, 0).unwrap();
        // 02:30 does not exist; 03:30 EDT is 07:30 UTC.
        assert_eq!(
            ctx.at(day, t),
            Utc.with_ymd_and_hms(2026, 3, 8, 7, 30, 0).unwrap()
        );
    }

    #[test]
    fn fall_back_overlap_takes_the_earlier_instant() {
        let ctx = context(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap());
        let day = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let t = NaiveTime::from_hms_opt(1, 30, 0).unwrap();
        // 01:30 happens twice; the first is still EDT, which is 05:30 UTC.
        assert_eq!(
            ctx.at(day, t),
            Utc.with_ymd_and_hms(2026, 11, 1, 5, 30, 0).unwrap()
        );
    }

    #[test]
    fn when_uses_a_kind_tag() {
        let at = Utc.with_ymd_and_hms(2026, 10, 15, 18, 0, 0).unwrap();
        let json = serde_json::to_value(When::Anytime).unwrap();
        assert_eq!(json, serde_json::json!({"kind": "anytime"}));
        let json = serde_json::to_value(When::At(at)).unwrap();
        assert_eq!(json["kind"], "at");
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let at = Utc.with_ymd_and_hms(2026, 10, 15, 18, 0, 0).unwrap();
        let task = Task {
            id: TaskId("t1".into()),
            title: "Meet".into(),
            duration_min: 60,
            when: When::At(at),
            done: false,
            order: 0,
            label: Label::Work,
            priority: Priority::High,
            source: Source::Text,
            created: at,
            external_uid: None,
        };
        let snapshot = Snapshot {
            tasks: vec![task],
            plan: Plan::default(),
            tz: "America/New_York".into(),
            generated: at,
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
    }
}

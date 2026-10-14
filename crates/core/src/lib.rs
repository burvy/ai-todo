//! Shared v2 data model (ARCHITECTURE.md section 4). Pure: chrono, chrono-tz and serde
//! only, so it builds for wasm.

pub mod clock;
pub mod tuning;

pub use clock::{Clock, FixedClock, SystemClock};

use chrono::{
    DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday,
};
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

/// The two times of day a task with a day but no time depends on, both per user
/// (ARCHITECTURE.md 4.1). They are local wall-clock times.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaySettings {
    /// When "by Friday" is due: the deadline lands at this time on that day.
    #[serde(with = "hh_mm")]
    pub deadline: NaiveTime,
    /// Where on that day the task is placed when the user has given no times of their
    /// own to learn from.
    #[serde(with = "hh_mm")]
    pub placement: NaiveTime,
}

/// A time of day on the wire as `HH:MM`, the shape the settings are typed and read in.
/// Reading also takes `HH:MM:SS`.
mod hh_mm {
    use chrono::NaiveTime;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(time: &NaiveTime, out: S) -> Result<S::Ok, S::Error> {
        out.collect_str(&time.format("%H:%M"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<NaiveTime, D::Error> {
        let text = String::deserialize(input)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl Default for DaySettings {
    fn default() -> Self {
        Self {
            deadline: NaiveTime::from_hms_opt(23, 59, 0).expect("23:59 is a valid time"),
            placement: NaiveTime::from_hms_opt(12, 0, 0).expect("12:00 is a valid time"),
        }
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
    /// One occurrence of a [`Series`]. The occurrence rows in a snapshot are
    /// built on the fly and never stored; a done occurrence is stored as a
    /// [`SeriesException`] on the series instead.
    Series,
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
    /// When the server last accepted a change to this task. Clients send back the value
    /// they last saw; an edit made against an older one is refused (`StaleUpdate`).
    pub updated_at: DateTime<Utc>,
    /// The IANA zone a `DueBy` deadline was set in, so it keeps its local time if the
    /// user later travels or changes zone. `None` for `Anytime`, `At` and pre-null
    /// tasks. `#[serde(default)]` keeps snapshots written before this field parse.
    #[serde(default)]
    pub due_tz: Option<String>,
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
    /// The user's weekly series (ARCHITECTURE.md 7.1). Their occurrences are
    /// already in `tasks`; this is the rules, for the edit dialogs.
    /// `#[serde(default)]` keeps snapshots written before this field parse.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<Series>,
}

/// A weekly series (ARCHITECTURE.md 7.1): one repeated thing.
///
/// The rows are never stored per week; the server expands occurrences for the
/// planner window at snapshot time. Ids are one namespace with [`TaskId`], and
/// an occurrence's id is `"{series_id}:{date}"` — a valid [`TaskId`], so a
/// placement can name it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Series {
    pub id: SeriesId,
    pub title: String,
    pub duration_min: u16,
    /// The repeat rule: which weekdays, until when.
    pub rule: Recurrence,
    /// The first local date an occurrence may fall on, inclusive.
    pub start_day: NaiveDate,
    /// The wall-clock time of each occurrence, in `zone`. `None` floats the
    /// occurrence into the week's free time.
    pub time_of_day: Option<NaiveTime>,
    /// The IANA zone the weekly time was set in, like `due_tz` for a deadline.
    pub zone: String,
    /// iCalendar UID for an imported recurring event.
    pub external_uid: Option<String>,
    pub created: DateTime<Utc>,
    /// List position, same key as [`Task::order`].
    pub order: i64,
}

/// Which weekdays a series repeats on, and until when.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recurrence {
    /// Kept sorted Monday-first and deduplicated; serde as `"Mon"`, `"Thu"`,
    /// ... (chrono's spelling). Empty never reaches the store:
    /// [`Recurrence::new`] refuses it. (`Weekday` is not `Ord`, so this cannot
    /// be a set type.)
    pub weekdays: Vec<Weekday>,
    /// The last local date an occurrence falls on, inclusive. `None` repeats
    /// forever.
    pub until: Option<NaiveDate>,
}

impl Recurrence {
    /// A rule on the given weekdays. Empty is a mistake the type system cannot
    /// hold, so the constructor refuses it here.
    pub fn new(
        weekdays: impl IntoIterator<Item = Weekday>,
        until: Option<NaiveDate>,
    ) -> Option<Self> {
        let mut weekdays: Vec<Weekday> = weekdays.into_iter().collect();
        weekdays.sort_by_key(|day| day.num_days_from_monday());
        weekdays.dedup();
        if weekdays.is_empty() {
            return None;
        }
        Some(Self { weekdays, until })
    }

    /// Does an occurrence exist on this local date?
    /// `start_day` bounds it from below, `until` from above, both inclusive.
    pub fn covers(&self, start_day: NaiveDate, date: NaiveDate) -> bool {
        self.weekdays.contains(&date.weekday())
            && start_day <= date
            && self.until.is_none_or(|until| date <= until)
    }
}

/// A series id. One namespace with [`TaskId`], so an id is unique across the
/// whole database, not just one user's.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeriesId(pub String);

/// One occurrence that deviates from its series' rule, keyed by local date.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesException {
    /// The occurrence's original local date.
    pub date: NaiveDate,
    pub kind: ExceptionKind,
}

/// What happened to one occurrence. A skipped day produces no occurrence at
/// all; a done one keeps its row, marked done; a moved one sits at the new
/// time, still named by its original date.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "when", rename_all = "snake_case")]
pub enum ExceptionKind {
    Done,
    Skipped,
    Moved {
        to: When,
        /// The zone the move was made in, so a `DueBy` target keeps its local
        /// time, like `due_tz` on a task.
        to_tz: Option<String>,
    },
}

/// The pure expansion result for one day of a series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub series: SeriesId,
    /// The occurrence's local date (the original one for a moved occurrence).
    pub date: NaiveDate,
    pub when: When,
    pub status: OccurrenceStatus,
    /// For a moved `DueBy` target: the zone the move was made in, like
    /// `due_tz` on a task. `None` for everything else.
    pub due_tz: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccurrenceStatus {
    Open,
    Done,
    Moved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    NotFound,
    Conflict,
    /// The task changed since the client last saw it (409): fetch it again and redo the edit.
    StaleUpdate,
    /// No valid session: 401. Separate from `BadRequest` because the fix is to sign
    /// in again, not to send a different request.
    Unauthorized,
    /// This account is paused (403): the session is valid and the request is
    /// well-formed, but the operator has stopped it using the app. Lift the pause
    /// in the admin console (`unpause <email|id>`) or on the admin page.
    Forbidden,
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

    #[test]
    fn day_settings_are_written_hh_mm_and_read_with_or_without_seconds() {
        let settings = DaySettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(json, r#"{"deadline":"23:59","placement":"12:00"}"#);
        assert_eq!(
            serde_json::from_str::<DaySettings>(&json).unwrap(),
            settings
        );
        let with_seconds = r#"{"deadline":"23:59:00","placement":"12:00:00"}"#;
        assert_eq!(
            serde_json::from_str::<DaySettings>(with_seconds).unwrap(),
            settings
        );
    }

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
            updated_at: at,
            due_tz: None,
            external_uid: None,
        };
        let snapshot = Snapshot {
            tasks: vec![task],
            plan: Plan::default(),
            tz: "America/New_York".into(),
            generated: at,
            series: Vec::new(),
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
    }

    fn series(id: &str, created: DateTime<Utc>) -> Series {
        Series {
            id: SeriesId(id.into()),
            title: "Gym".into(),
            duration_min: 45,
            rule: Recurrence::new([Weekday::Mon, Weekday::Thu], None).unwrap(),
            start_day: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            time_of_day: Some(NaiveTime::from_hms_opt(7, 0, 0).unwrap()),
            zone: "America/New_York".into(),
            external_uid: None,
            created,
            order: -1,
        }
    }

    #[test]
    fn a_weekday_serializes_as_chrono_spells_it() {
        let json =
            serde_json::to_string(&Recurrence::new([Weekday::Mon, Weekday::Thu], None).unwrap())
                .unwrap();
        assert_eq!(json, r#"{"weekdays":["Mon","Thu"],"until":null}"#);
    }

    #[test]
    fn a_series_round_trips_through_json() {
        let created = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        let mut held = series("s1", created);
        held.rule.until = Some(NaiveDate::from_ymd_opt(2026, 12, 15).unwrap());
        let json = serde_json::to_string(&held).unwrap();
        assert_eq!(serde_json::from_str::<Series>(&json).unwrap(), held);
    }

    #[test]
    fn an_empty_weekday_set_is_refused() {
        assert!(Recurrence::new([], None).is_none());
        assert!(Recurrence::new([Weekday::Fri], None).is_some());
    }

    #[test]
    fn the_rule_covers_only_its_own_days_inside_its_bounds() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(); // Monday
        let rule = Recurrence::new([Weekday::Mon, Weekday::Thu], None).unwrap();
        assert!(rule.covers(start, start));
        assert!(rule.covers(start, NaiveDate::from_ymd_opt(2026, 10, 15).unwrap())); // Thursday
        assert!(!rule.covers(start, NaiveDate::from_ymd_opt(2026, 10, 13).unwrap())); // Tuesday
        assert!(!rule.covers(start, NaiveDate::from_ymd_opt(2026, 10, 11).unwrap())); // before start_day
        let bounded = Recurrence::new(
            [Weekday::Mon],
            Some(NaiveDate::from_ymd_opt(2026, 10, 12).unwrap()),
        )
        .unwrap();
        assert!(bounded.covers(start, start));
        assert!(!bounded.covers(start, NaiveDate::from_ymd_opt(2026, 10, 19).unwrap())); // after until
    }

    #[test]
    fn an_exception_round_trips_through_json() {
        let exception = SeriesException {
            date: NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            kind: ExceptionKind::Moved {
                to: When::At(Utc.with_ymd_and_hms(2026, 10, 16, 14, 0, 0).unwrap()),
                to_tz: None,
            },
        };
        let json = serde_json::to_string(&exception).unwrap();
        assert_eq!(
            serde_json::from_str::<SeriesException>(&json).unwrap(),
            exception
        );
    }

    #[test]
    fn a_snapshot_with_a_series_round_trips_through_json() {
        let created = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        let snapshot = Snapshot {
            tasks: Vec::new(),
            plan: Plan::default(),
            tz: "UTC".into(),
            generated: created,
            series: vec![series("s1", created)],
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(serde_json::from_str::<Snapshot>(&json).unwrap(), snapshot);
    }

    #[test]
    fn a_snapshot_written_before_series_still_parses() {
        // The web may hold a snapshot from an older server; the field is
        // optional on the wire.
        let json = r#"{"tasks":[],"plan":{"placements":[],"unplaced":[]},"tz":"UTC","generated":"1970-01-01T00:00:00+00:00"}"#;
        let back: Snapshot = serde_json::from_str(json).unwrap();
        assert!(back.series.is_empty());
    }
}

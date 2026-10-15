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

    fn exceptions(&self, user: &str, id: &SeriesId) -> Result<Vec<SeriesException>, StoreError> {
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

#[test]
fn a_restored_task_comes_back_as_it_was_under_a_fresh_id() {
    let (mut app, _) = app();
    let snapshot = app
        .create(draft("Water the plants"), "u1", Tz::UTC)
        .unwrap();
    let mut gone = snapshot.tasks[0].clone();
    gone.done = true;
    gone.priority = Priority::High;
    app.delete(&gone.id.0, "u1", Tz::UTC).unwrap();

    let back = app.restore(gone.clone(), "u1", Tz::UTC).unwrap();
    let task = &back.tasks[0];
    assert_ne!(task.id, gone.id, "a client's id is never stored");
    assert_eq!(
        (
            task.title.as_str(),
            task.done,
            task.order,
            task.priority,
            task.created
        ),
        (
            "Water the plants",
            true,
            gone.order,
            Priority::High,
            gone.created
        )
    );
    assert_eq!(task.when, gone.when);
}

#[test]
fn a_restored_task_keeps_its_place_in_the_list() {
    let (mut app, _) = app();
    for title in ["Feed the cat", "Water the plants", "Post the letter"] {
        app.create(draft(title), "u1", Tz::UTC).unwrap();
    }
    let titles = |snapshot: &Snapshot| {
        let mut tasks = snapshot.tasks.clone();
        tasks.sort_by_key(|task| task.order);
        tasks.into_iter().map(|task| task.title).collect::<Vec<_>>()
    };
    let before = app.snapshot("u1", Tz::UTC).unwrap();
    for gone in before.tasks.clone() {
        app.delete(&gone.id.0, "u1", Tz::UTC).unwrap();
        let back = app.restore(gone, "u1", Tz::UTC).unwrap();
        assert_eq!(titles(&back), titles(&before));
    }
}

#[test]
fn hostile_restore_fields_are_tamed() {
    let (mut app, _) = app();
    let snapshot = app.create(draft("Pay rent"), "u1", Tz::UTC).unwrap();
    let mut hostile = snapshot.tasks[0].clone();
    hostile.order = i64::MIN;
    hostile.when = When::DueBy(hostile.created);
    hostile.due_tz = Some("Not/AZone".repeat(1000));
    hostile.external_uid = Some(format!("{}@import", "x".repeat(10_000)));
    hostile.created = DateTime::<Utc>::MAX_UTC;
    let back = app.restore(hostile, "u1", Tz::UTC).unwrap();
    let task = back
        .tasks
        .iter()
        .min_by_key(|task| task.order)
        .expect("restored");
    assert_eq!(
        task.order,
        snapshot.tasks[0].order - 1,
        "first, not i64::MIN"
    );
    assert_eq!(
        task.due_tz.as_deref(),
        Some("UTC"),
        "re-anchored where the user is"
    );
    assert_eq!(task.external_uid, None);
    assert!(
        task.created <= task.updated_at,
        "never created in the future"
    );
    // The next capture places itself above without overflowing.
    app.create(draft("Call the bank"), "u1", Tz::UTC).unwrap();

    let mut zoned = snapshot.tasks[0].clone();
    zoned.when = When::DueBy(zoned.created);
    zoned.due_tz = Some("Europe/Paris".into());
    let back = app.restore(zoned, "u1", Tz::UTC).unwrap();
    assert!(
        back.tasks
            .iter()
            .any(|task| task.due_tz.as_deref() == Some("Europe/Paris")),
        "a real zone is kept"
    );
}

#[test]
fn a_repeat_s_day_is_not_restored_as_a_task() {
    let (mut app, _) = app();
    let snapshot = app.create(draft("Gym"), "u1", Tz::UTC).unwrap();
    let mut day = snapshot.tasks[0].clone();
    day.source = Source::Series;
    let error = app.restore(day, "u1", Tz::UTC).unwrap_err();
    assert_eq!(error.code, ErrorCode::BadRequest);
    let mut blank = snapshot.tasks[0].clone();
    blank.title = "  ".into();
    assert!(
        app.restore(blank, "u1", Tz::UTC).is_err(),
        "the create checks apply"
    );
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
    let patch: SeriesPatch = serde_json::from_str(r#"{"time_of_day":null,"until":null}"#).unwrap();
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

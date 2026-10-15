use chrono::Duration;
const LABELS: [Label; 6] = [
    Label::School,
    Label::Work,
    Label::Errands,
    Label::Health,
    Label::Personal,
    Label::Other,
];
const PRIORITIES: [Priority; 4] = [
    Priority::Low,
    Priority::Medium,
    Priority::High,
    Priority::Critical,
];
const SOURCES: [Source; 6] = [
    Source::Voice,
    Source::Pdf,
    Source::Email,
    Source::Text,
    Source::Calendar,
    Source::Series,
];

use super::*;
use chrono::{TimeZone, Weekday};
use todo_core::Recurrence;

/// The user the contract suite writes as. Scoping is part of the contract now, so
/// the tests have to name an owner like a real caller does.
const USER: &str = "alice";
const OTHER: &str = "bob";

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 16, hour, 0, 0).unwrap()
}

fn task(id: &str, order: i64) -> Task {
    Task {
        id: TaskId(id.into()),
        title: format!("Task {id}"),
        duration_min: 30,
        when: When::Anytime,
        done: false,
        order,
        label: Label::Work,
        priority: Priority::Medium,
        source: Source::Text,
        created: at(9),
        updated_at: at(9),
        due_tz: None,
        external_uid: None,
        off_planner: false,
    }
}

/// One stored task, by id. Looked up by name so the checks read as claims
/// about behaviour rather than about where a row happened to sort.
fn read(store: &dyn Store, id: &TaskId) -> Task {
    store
        .tasks(USER)
        .expect("read tasks")
        .into_iter()
        .find(|task| &task.id == id)
        .unwrap_or_else(|| panic!("no stored task {id:?}"))
}

/// The deadline zone (docs/ARCHITECTURE.md 4.1) rides with the task through
/// whichever store.
fn due_tz_round_trips(store: &dyn Store) {
    let mut task = task("due", 0);
    task.when = When::DueBy(at(21));
    task.due_tz = Some("America/New_York".into());
    store.put(USER, task.clone()).unwrap();

    let stored = store.tasks(USER).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].when, When::DueBy(at(21)));
    assert_eq!(
        stored[0].due_tz.as_deref(),
        Some("America/New_York"),
        "a deadline's zone must survive a round trip"
    );
}

/// The task's "off the planner" flag (docs/ARCHITECTURE.md 4.2) rides with the task
/// through whichever store, and a task without it comes back on the planner.
fn off_planner_round_trips(store: &dyn Store) {
    let mut held = task("held", 0);
    held.off_planner = true;
    store.put(USER, held.clone()).unwrap();
    store.put(USER, task("planned", 1)).unwrap();

    assert!(read(store, &held.id).off_planner, "the flag must survive");
    assert!(
        !read(store, &TaskId("planned".into())).off_planner,
        "a task nobody took off is still planned"
    );
}

/// A Monday and Thursday series at 07:00, for the store contract.
fn series(id: &str, order: i64) -> Series {
    Series {
        id: SeriesId(id.into()),
        title: format!("Series {id}"),
        duration_min: 45,
        rule: Recurrence::new([Weekday::Mon, Weekday::Thu], None).unwrap(),
        start_day: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
        time_of_day: NaiveTime::from_hms_opt(7, 0, 0),
        zone: "America/New_York".into(),
        external_uid: None,
        created: at(9),
        order,
    }
}

fn on(ymd: (i32, u32, u32)) -> NaiveDate {
    NaiveDate::from_ymd_opt(ymd.0, ymd.1, ymd.2).unwrap()
}

/// A series and its exceptions survive a round trip through whichever store, one
/// exception is held per date, and deleting the series takes them with it
/// (docs/ARCHITECTURE.md 7.1).
fn series_round_trip(store: &dyn Store) {
    let mut full = series("s1", 0);
    full.rule = Recurrence::new(
        [Weekday::Tue, Weekday::Fri],
        Some(NaiveDate::from_ymd_opt(2026, 12, 18).unwrap()),
    )
    .unwrap();
    full.external_uid = Some("series-uid@rambl".into());
    store.put_series(USER, full.clone()).unwrap();

    assert_eq!(
        store.series(USER).unwrap(),
        vec![full.clone()],
        "every field of a series must survive"
    );

    let moved = ExceptionKind::Moved {
        to: When::At(at(18)),
        to_tz: Some("America/Los_Angeles".into()),
    };
    for (date, kind) in [
        (on((2026, 10, 13)), ExceptionKind::Done),
        (on((2026, 10, 16)), ExceptionKind::Skipped),
        (on((2026, 10, 20)), moved.clone()),
    ] {
        store
            .put_exception(USER, &full.id, SeriesException { date, kind })
            .unwrap();
    }
    let held = store.exceptions(USER, &full.id).unwrap();
    assert_eq!(held.len(), 3);
    assert_eq!(
        held[2].kind, moved,
        "a moved occurrence keeps its new instant and its zone"
    );

    // One row per date: the same day answered twice is one answer, the later one.
    store
        .put_exception(
            USER,
            &full.id,
            SeriesException {
                date: on((2026, 10, 13)),
                kind: ExceptionKind::Skipped,
            },
        )
        .unwrap();
    let held = store.exceptions(USER, &full.id).unwrap();
    assert_eq!(held.len(), 3, "one date is never held twice");
    assert_eq!(held[0].kind, ExceptionKind::Skipped);

    // And clearing one puts the day back to the rule.
    store
        .delete_exception(USER, &full.id, on((2026, 10, 16)))
        .unwrap();
    assert_eq!(
        store.exceptions(USER, &full.id).unwrap().len(),
        2,
        "a cleared exception is gone"
    );

    assert!(store.delete_series(USER, &full.id).unwrap());
    assert!(store.series(USER).unwrap().is_empty());
    assert!(
        store.exceptions(USER, &full.id).unwrap().is_empty(),
        "a deleted series leaves no exceptions behind"
    );
    assert!(!store.delete_series(USER, &full.id).unwrap());
}

/// A series is owned like a task: another person sees none of it, cannot delete it
/// by name, and cannot reach its exceptions.
fn series_are_isolated(store: &dyn Store) {
    store.put_series(USER, series("mine", 0)).unwrap();
    store.put_series(OTHER, series("theirs", 0)).unwrap();
    store
        .put_exception(
            USER,
            &SeriesId("mine".into()),
            SeriesException {
                date: on((2026, 10, 12)),
                kind: ExceptionKind::Done,
            },
        )
        .unwrap();

    assert_eq!(store.series(USER).unwrap().len(), 1);
    assert_eq!(store.series(OTHER).unwrap().len(), 1);
    assert!(
        store
            .exceptions(OTHER, &SeriesId("mine".into()))
            .unwrap()
            .is_empty(),
        "bob read alice's exceptions"
    );

    assert!(
        !store
            .delete_series(OTHER, &SeriesId("mine".into()))
            .unwrap(),
        "bob cannot delete alice's series by name"
    );
    assert_eq!(
        store.series(USER).unwrap().len(),
        1,
        "alice kept her series"
    );
    assert_eq!(
        store
            .exceptions(USER, &SeriesId("mine".into()))
            .unwrap()
            .len(),
        1,
        "alice kept her exceptions"
    );
}

/// Two people must never see each other's tasks. This is the promise the whole
/// user_id column exists for, and it is the one a missing filter would break.
fn users_are_isolated(store: &dyn Store) {
    store.put(USER, task("mine", 0)).unwrap();
    store.put(OTHER, task("theirs", 0)).unwrap();

    // An id held by one user cannot be written, and so cannot be taken over, by
    // another: the first user's task stays where it is, untouched.
    let mut hijack = task("mine", 0);
    hijack.title = "hijacked".into();
    assert!(matches!(
        store.put(OTHER, hijack),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(store.tasks(USER).unwrap()[0].title, "Task mine");
    assert_eq!(store.tasks(OTHER).unwrap().len(), 1);

    let mine = store.tasks(USER).unwrap();
    assert_eq!(
        mine.len(),
        1,
        "alice should see only her own task, saw {:?}",
        mine.iter().map(|t| &t.title).collect::<Vec<_>>()
    );
    assert_eq!(mine[0].id, TaskId("mine".into()));

    let theirs = store.tasks(OTHER).unwrap();
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0].id, TaskId("theirs".into()));

    // Another person's task must not be reachable by its id, either.
    assert!(
        store.find_by_uid(USER, "theirs-uid").unwrap().is_none(),
        "alice reached bob's task by uid"
    );

    // And deleting only removes the caller's own.
    store.delete(OTHER, &TaskId("mine".into())).unwrap();
    assert_eq!(store.tasks(USER).unwrap().len(), 1, "alice lost her task");
}
/// The suite both stores must pass. Everything here goes through the
/// [`Store`] trait, so a store that passes it behaves like the other one.
fn store_contract(store: &dyn Store) {
    // -- an empty store has nothing in it
    assert_eq!(store.tasks(USER).unwrap().len(), 0, "a new store is empty");

    // -- put then read back
    store.put(USER, task("t1", 0)).unwrap();
    store.put(USER, task("t2", 1)).unwrap();
    let stored = store.tasks(USER).unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].id, TaskId("t1".into()));
    assert_eq!(stored[1].id, TaskId("t2".into()));

    // -- every field survives the round trip
    let mut original = task("t3", 2);
    original.title = "Read; notes, again".into();
    original.duration_min = 95;
    original.when = When::DueBy(at(17));
    original.done = true;
    original.order = 5;
    original.label = Label::School;
    original.priority = Priority::Critical;
    original.source = Source::Calendar;
    original.created = at(8);
    original.updated_at = at(10);
    original.external_uid = Some("abc-123@rambl".into());
    store.put(USER, original.clone()).unwrap();
    assert_eq!(read(store, &original.id), original);

    // -- each When variant survives
    for (index, when) in [When::Anytime, When::DueBy(at(23)), When::At(at(14))]
        .into_iter()
        .enumerate()
    {
        let mut pinned = task(&format!("w{index}"), 100 + index as i64);
        pinned.when = when;
        store.put(USER, pinned.clone()).unwrap();
        let read = store
            .tasks(USER)
            .unwrap()
            .into_iter()
            .find(|t| t.id == pinned.id)
            .expect("task is stored");
        assert_eq!(read.when, when, "{when:?} did not survive");
    }

    // -- every enum spelling survives
    for (index, label) in LABELS.into_iter().enumerate() {
        let mut labelled = task(&format!("l{index}"), 200 + index as i64);
        labelled.label = label;
        store.put(USER, labelled.clone()).unwrap();
        assert_eq!(read(store, &labelled.id).label, label);
    }
    for (index, priority) in PRIORITIES.into_iter().enumerate() {
        let mut ranked = task(&format!("p{index}"), 300 + index as i64);
        ranked.priority = priority;
        store.put(USER, ranked.clone()).unwrap();
        assert_eq!(read(store, &ranked.id).priority, priority);
    }
    for (index, source) in SOURCES.into_iter().enumerate() {
        let mut sourced = task(&format!("s{index}"), 400 + index as i64);
        sourced.source = source;
        store.put(USER, sourced.clone()).unwrap();
        assert_eq!(read(store, &sourced.id).source, source);
    }

    // -- put on a known id replaces, and does not duplicate
    let before = store.tasks(USER).unwrap().len();
    let mut edited = original.clone();
    edited.title = "Renamed".into();
    edited.done = false;
    store.put(USER, edited.clone()).unwrap();
    let all = store.tasks(USER).unwrap();
    assert_eq!(all.len(), before, "put must replace, never duplicate");
    assert_eq!(read(store, &edited.id).title, "Renamed");
    assert!(!read(store, &edited.id).done);

    // -- tasks come back sorted by order
    let orders: Vec<i64> = store.tasks(USER).unwrap().iter().map(|t| t.order).collect();
    let mut ascending = orders.clone();
    ascending.sort_unstable();
    assert_eq!(orders, ascending, "tasks must come back in order");

    // -- delete reports whether there was something to delete
    let before_delete = store.tasks(USER).unwrap().len();
    assert!(store.delete(USER, &TaskId("t1".into())).unwrap());
    assert!(
        !store.delete(USER, &TaskId("t1".into())).unwrap(),
        "gone already"
    );
    assert!(!store.delete(USER, &TaskId("never-existed".into())).unwrap());
    assert_eq!(store.tasks(USER).unwrap().len(), before_delete - 1);
    assert!(
        !store.tasks(USER).unwrap().iter().any(|t| t.id.0 == "t1"),
        "the deleted task must not come back"
    );

    // -- set_order rewrites the order
    let before: Vec<String> = store
        .tasks(USER)
        .unwrap()
        .into_iter()
        .map(|t| t.id.0)
        .collect();
    let mut reversed = before.clone();
    reversed.reverse();
    let reversed_ids: Vec<TaskId> = reversed.iter().cloned().map(TaskId).collect();
    store.set_order(USER, &reversed_ids).unwrap();
    let after: Vec<String> = store
        .tasks(USER)
        .unwrap()
        .into_iter()
        .map(|t| t.id.0)
        .collect();
    assert_eq!(after, reversed, "set_order is the new order");

    // -- set_order makes the positions 0, 1, ... in the order given
    let positions: Vec<i64> = store.tasks(USER).unwrap().iter().map(|t| t.order).collect();
    assert_eq!(
        positions,
        (0..positions.len() as i64).collect::<Vec<_>>(),
        "positions must be gapless and start at zero"
    );

    // -- an id that is not stored is skipped, not invented
    let count = store.tasks(USER).unwrap().len();
    store.set_order(USER, &[TaskId("nope".into())]).unwrap();
    let after_unknown = store.tasks(USER).unwrap();
    assert_eq!(
        after_unknown.len(),
        count,
        "set_order must not create tasks"
    );
    assert!(
        !after_unknown.iter().any(|t| t.id.0 == "nope"),
        "an unknown id must not become a task"
    );

    // -- a full order is the normal case and must be exact
    let wanted: Vec<String> = store
        .tasks(USER)
        .unwrap()
        .into_iter()
        .map(|t| t.id.0)
        .collect();
    let wanted_ids: Vec<TaskId> = wanted.iter().cloned().map(TaskId).collect();
    store.set_order(USER, &wanted_ids).unwrap();
    let restored: Vec<String> = store
        .tasks(USER)
        .unwrap()
        .into_iter()
        .map(|t| t.id.0)
        .collect();
    assert_eq!(restored, wanted, "set_order is the new order");

    // -- find_by_uid
    assert!(
        store
            .find_by_uid(USER, "abc-123@rambl")
            .unwrap()
            .is_some_and(|t| t.id.0 == "t3")
    );
    assert!(store.find_by_uid(USER, "never-imported").unwrap().is_none());
    assert!(store.find_by_uid(USER, "").unwrap().is_none());

    // -- a task with no uid is not found by any uid
    assert!(
        store
            .tasks(USER)
            .unwrap()
            .iter()
            .filter(|t| t.external_uid.is_none())
            .count()
            > 0
    );
    assert!(store.find_by_uid(USER, "no-such-uid").unwrap().is_none());
}

fn anonymous_task(id: &str, uid: Option<&str>) -> Task {
    Task {
        external_uid: uid.map(str::to_owned),
        source: Source::Calendar,
        ..task(id, 0)
    }
}

/// Signing in keeps the signed-in copy of a duplicated calendar event, moves the rest,
/// and ends the anonymous id.
fn adoption_moves_anonymous_tasks_and_ends_the_id(store: &dyn Store) {
    const ANON: &str = "anon_0123456789abcdef0123456789abcdef";
    store.ensure_anonymous(ANON, at(9)).unwrap();
    store
        .put(ANON, anonymous_task("a1", Some("uid-1")))
        .unwrap();
    store.put(ANON, anonymous_task("a2", None)).unwrap();
    store
        .put(USER, anonymous_task("u1", Some("uid-1")))
        .unwrap();
    store.put_series(ANON, series("anon-series", 0)).unwrap();
    store
        .put_exception(
            ANON,
            &SeriesId("anon-series".into()),
            SeriesException {
                date: on((2026, 10, 12)),
                kind: ExceptionKind::Done,
            },
        )
        .unwrap();

    store.adopt_anonymous(ANON, USER).unwrap();

    let mine: Vec<_> = store
        .tasks(USER)
        .unwrap()
        .into_iter()
        .map(|task| task.id.0)
        .collect();
    assert!(
        mine.contains(&"u1".to_owned()),
        "the signed-in copy is kept"
    );
    assert!(
        mine.contains(&"a2".to_owned()),
        "the other anonymous task moves"
    );
    assert!(
        !mine.contains(&"a1".to_owned()),
        "a duplicate of a signed-in event is dropped, not doubled"
    );
    assert!(store.tasks(ANON).unwrap().is_empty());
    assert_eq!(
        store.series(USER).unwrap().len(),
        1,
        "a series made while anonymous moves with the tasks"
    );
    assert_eq!(
        store
            .exceptions(USER, &SeriesId("anon-series".into()))
            .unwrap()
            .len(),
        1,
        "and its exceptions move with it"
    );
    assert!(
        !store.anonymous_exists(ANON).unwrap(),
        "the anonymous id must stop working once it is adopted"
    );
}

/// A calendar imported while anonymous and again after signing in is one series,
/// not two: the signed-in copy stays and the anonymous copy goes, exceptions and all.
fn adoption_keeps_the_signed_in_copy_of_a_series(store: &dyn Store) {
    const ANON: &str = "anon_0123456789abcdef0123456789abcdef";
    let imported = |id: &str, uid: Option<&str>| Series {
        external_uid: uid.map(str::to_owned),
        ..series(id, 0)
    };
    store.ensure_anonymous(ANON, at(9)).unwrap();
    store
        .put_series(ANON, imported("anon-copy", Some("uid-1")))
        .unwrap();
    store
        .put_series(ANON, imported("anon-own", Some("uid-2")))
        .unwrap();
    store
        .put_exception(
            ANON,
            &SeriesId("anon-copy".into()),
            SeriesException {
                date: on((2026, 10, 12)),
                kind: ExceptionKind::Done,
            },
        )
        .unwrap();
    store
        .put_series(USER, imported("mine", Some("uid-1")))
        .unwrap();

    store.adopt_anonymous(ANON, USER).unwrap();

    let mut ids: Vec<_> = store
        .series(USER)
        .unwrap()
        .into_iter()
        .map(|held| held.id.0)
        .collect();
    ids.sort();
    assert_eq!(ids, ["anon-own", "mine"]);
    assert!(
        store
            .exceptions(USER, &SeriesId("anon-copy".into()))
            .unwrap()
            .is_empty(),
        "the dropped copy's exceptions must not follow it"
    );
}

/// An anonymous account that was adopted or deleted is gone, so expiry has
/// nothing left of it to count.
fn adopted_and_deleted_accounts_are_not_expired_again(store: &dyn Store) {
    const ADOPTED: &str = "anon_0123456789abcdef0123456789abcdef";
    const DELETED: &str = "anon_fedcba9876543210fedcba9876543210";
    store.ensure_anonymous(ADOPTED, at(9)).unwrap();
    store.ensure_anonymous(DELETED, at(9)).unwrap();

    store.adopt_anonymous(ADOPTED, USER).unwrap();
    store.delete_user(DELETED).unwrap();

    assert_eq!(
        store.expire_anonymous(at(9) + Duration::days(365)).unwrap(),
        0
    );
}

/// Only a real anonymous id adopts anything. A signed-in user's id, given as the
/// anonymous header, moves nothing.
fn adoption_ignores_ids_that_are_not_anonymous(store: &dyn Store) {
    store.put(OTHER, anonymous_task("b1", None)).unwrap();

    store.adopt_anonymous(OTHER, USER).unwrap();

    assert!(store.tasks(USER).unwrap().is_empty(), "bob's tasks moved");
    assert_eq!(store.tasks(OTHER).unwrap().len(), 1);
}

/// Blocks are per user and per address, and lifting one leaves the other.
fn blocks_hold_until_lifted(store: &dyn Store) {
    let user = Block::User(USER.into());
    let ip = Block::Ip("203.0.113.9".into());
    store.block(&user).unwrap();
    store.block(&ip).unwrap();
    store.block(&ip).unwrap();

    assert!(store.is_blocked(USER, "198.51.100.1").unwrap());
    assert!(store.is_blocked(OTHER, "203.0.113.9").unwrap());
    assert!(!store.is_blocked(OTHER, "198.51.100.1").unwrap());
    assert_eq!(
        store.blocks().unwrap().len(),
        2,
        "blocking twice keeps one row"
    );

    assert!(store.unblock(&user).unwrap());
    assert!(!store.unblock(&user).unwrap(), "lifting nothing says so");
    assert!(!store.is_blocked(USER, "198.51.100.1").unwrap());
    assert!(store.is_blocked(OTHER, "203.0.113.9").unwrap());
}

/// Captures count per user and per address, and usage ranks them for the day.
fn captures_are_counted_per_user_and_address(store: &dyn Store) {
    let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    let next = chrono::NaiveDate::from_ymd_opt(2026, 10, 13).unwrap();
    store.record_capture(USER, "203.0.113.1", day).unwrap();
    store.record_capture(USER, "203.0.113.1", day).unwrap();
    store.record_capture(USER, "203.0.113.2", day).unwrap();
    store.record_capture(OTHER, "203.0.113.1", day).unwrap();
    store.record_capture(USER, "203.0.113.1", next).unwrap();

    assert_eq!(store.captures_today(USER, day).unwrap(), 3);
    assert_eq!(store.captures_today(USER, next).unwrap(), 1);
    assert_eq!(store.ip_captures_today("203.0.113.1", day).unwrap(), 3);

    let usage = store.usage_today(day, 10).unwrap();
    assert_eq!(usage.users, vec![(USER.into(), 3), (OTHER.into(), 1)]);
    assert_eq!(
        usage.ips,
        vec![("203.0.113.1".into(), 3), ("203.0.113.2".into(), 1)]
    );
    assert_eq!(store.usage_today(day, 1).unwrap().users.len(), 1);
}

/// Anonymous ids are issued per address per day, and the count is what the caller
/// checks against its limit.
fn anonymous_ids_are_counted_per_address_per_day(store: &dyn Store) {
    let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    assert_eq!(store.record_anon_issued("203.0.113.1", day).unwrap(), 1);
    assert_eq!(store.record_anon_issued("203.0.113.1", day).unwrap(), 2);
    assert_eq!(store.record_anon_issued("203.0.113.2", day).unwrap(), 1);
}

/// Sessions, plans, account lookups, user blocks, spend rankings and account
/// deletion: the calls sign-in, the admin console and usage pages make.
fn accounts_sessions_and_rankings(store: &dyn Store) {
    use super::super::auth::{Me, Plan};
    let me = |id: &str, plan: Plan| Me {
        id: id.to_owned(),
        email: format!("{id}@example.com"),
        name: id.to_owned(),
        picture: None,
        plan,
    };
    let expires = at(23);
    store
        .upsert_user_and_session(&me(USER, Plan::Free), "tok-alice", at(0), expires)
        .unwrap();
    store
        .upsert_user_and_session(&me(USER, Plan::Free), "tok-alice-2", at(0), expires)
        .unwrap();
    store
        .upsert_user_and_session(&me(OTHER, Plan::Paid), "tok-bob", at(0), expires)
        .unwrap();

    // -- a live session resolves to its user; an unknown or expired one to nobody
    let owner = |token: &str, now| store.session_user(token, now).unwrap().map(|m| m.id);
    assert_eq!(owner("tok-alice", at(12)), Some(USER.into()));
    assert_eq!(owner("no-such-token", at(12)), None);
    assert_eq!(owner("tok-bob", at(23) + chrono::Duration::hours(1)), None);

    // -- signing out one session leaves the user's other sessions alone
    store.delete_session("tok-alice").unwrap();
    assert_eq!(
        owner("tok-alice", at(12)),
        None,
        "signed-out token still works"
    );
    assert_eq!(owner("tok-alice-2", at(12)), Some(USER.into()));

    // -- plans are per user
    assert_eq!(store.user_plan(USER).unwrap(), Plan::Free);
    store.set_user_plan(USER, Plan::Paid).unwrap();
    assert_eq!(store.user_plan(USER).unwrap(), Plan::Paid);
    assert_eq!(store.user_plan(OTHER).unwrap(), Plan::Paid);

    // -- day-only times are per user, and the defaults until someone changes them
    let defaults = todo_core::DaySettings::default();
    assert_eq!(store.day_settings(USER).unwrap(), defaults);
    let mine = todo_core::DaySettings {
        deadline: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        placement: NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
    };
    store.set_day_settings(USER, mine).unwrap();
    assert_eq!(store.day_settings(USER).unwrap(), mine);
    assert_eq!(store.day_settings(OTHER).unwrap(), defaults);

    // -- lookups: an account exists, and an email finds a signed-in user only
    assert!(store.account_exists(USER).unwrap());
    assert!(!store.account_exists("nobody").unwrap());
    assert_eq!(
        store.user_id_for_email("bob@example.com").unwrap(),
        Some(OTHER.into())
    );
    assert_eq!(store.user_id_for_email("nobody@example.com").unwrap(), None);
    store.ensure_anonymous(&anon_id(1), at(9)).unwrap();
    assert_eq!(
        store.user_id_for_email("").unwrap(),
        None,
        "an anonymous visitor has no email to find"
    );

    // -- a user block is not an address block
    store.block(&Block::Ip("203.0.113.9".into())).unwrap();
    assert!(!store.is_user_blocked(USER).unwrap());
    store.block(&Block::User(USER.into())).unwrap();
    assert!(store.is_user_blocked(USER).unwrap());
    assert!(!store.is_user_blocked(OTHER).unwrap());

    // -- rankings: all time, and today; ties go by id
    let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    let earlier = chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
    for _ in 0..3 {
        store.record_capture(USER, "203.0.113.1", day).unwrap();
    }
    store.record_capture(OTHER, "203.0.113.2", day).unwrap();
    for _ in 0..3 {
        store.record_capture(OTHER, "203.0.113.2", earlier).unwrap();
    }
    let ids = |rows: &[UsageRow]| rows.iter().map(|r| r.user.clone()).collect::<Vec<_>>();

    let all_time = store.usage_ranking(UsageOrder::AllTime, 10).unwrap();
    assert_eq!(
        ids(&all_time),
        vec![OTHER, USER],
        "bob has 4 calls, alice 3"
    );
    assert_eq!(all_time[0].calls, 4);
    assert_eq!(all_time[0].email, "bob@example.com");
    assert_eq!(all_time[0].plan, Plan::Paid);

    let today = store.usage_ranking(UsageOrder::Day(day), 10).unwrap();
    assert_eq!(ids(&today), vec![USER, OTHER]);
    assert_eq!(
        today.iter().map(|r| r.calls).collect::<Vec<_>>(),
        vec![3, 1]
    );

    assert_eq!(
        store.usage_ranking(UsageOrder::AllTime, 1).unwrap().len(),
        1,
        "top limits the rows"
    );

    // -- deleting a user takes their sessions and tasks, and nobody else's
    store.put(USER, task("alice-task", 0)).unwrap();
    store.put(OTHER, task("bob-task", 0)).unwrap();
    store.delete_user(USER).unwrap();
    assert!(
        !store.account_exists(USER).unwrap(),
        "deleted user still exists"
    );
    assert_eq!(
        owner("tok-alice-2", at(12)),
        None,
        "deleted user kept a session"
    );
    assert!(
        store.tasks(USER).unwrap().is_empty(),
        "deleted user kept tasks"
    );
    assert_eq!(
        store.tasks(OTHER).unwrap().len(),
        1,
        "deleting alice touched bob"
    );
    assert_eq!(owner("tok-bob", at(12)), Some(OTHER.into()));
}

/// Runs one contract on a fresh SQLite database in a folder of its own. The folder
/// is deleted when the call returns, after the store is closed.
fn on_disk(run: impl Fn(&dyn Store)) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = SqliteStore::open(&dir.path().join("rambl.db")).expect("open");
    run(&store);
}

#[test]
fn memory_store_accounts_sessions_and_rankings() {
    accounts_sessions_and_rankings(&MemoryStore::new());
}

#[test]
fn sqlite_store_accounts_sessions_and_rankings() {
    on_disk(accounts_sessions_and_rankings);
}

/// Every shared contract, each on a fresh file-backed SQLite store, so the on-disk
/// path is tested as well as the in-memory one.
#[test]
fn sqlite_file_store_passes_every_contract() {
    on_disk(users_are_isolated);
    on_disk(store_contract);
    on_disk(due_tz_round_trips);
    on_disk(off_planner_round_trips);
    on_disk(series_round_trip);
    on_disk(series_are_isolated);
    on_disk(adoption_moves_anonymous_tasks_and_ends_the_id);
    on_disk(adoption_ignores_ids_that_are_not_anonymous);
    on_disk(adoption_keeps_the_signed_in_copy_of_a_series);
    on_disk(adopted_and_deleted_accounts_are_not_expired_again);
    on_disk(blocks_hold_until_lifted);
    on_disk(captures_are_counted_per_user_and_address);
    on_disk(anonymous_ids_are_counted_per_address_per_day);
    on_disk(idle_anonymous_accounts_expire);
    on_disk(visits_keep_anonymous_ids_alive_with_a_throttle);
    on_disk(accounts_sessions_and_rankings);
}

#[test]
fn memory_store_adoption_and_scoping_rules() {
    adoption_moves_anonymous_tasks_and_ends_the_id(&MemoryStore::default());
    adoption_ignores_ids_that_are_not_anonymous(&MemoryStore::default());
    adoption_keeps_the_signed_in_copy_of_a_series(&MemoryStore::default());
    adopted_and_deleted_accounts_are_not_expired_again(&MemoryStore::default());
}

#[test]
fn sqlite_store_adoption_and_scoping_rules() {
    let open = || SqliteStore::open_in_memory().unwrap();
    adoption_moves_anonymous_tasks_and_ends_the_id(&open());
    adoption_ignores_ids_that_are_not_anonymous(&open());
}

#[test]
fn memory_store_blocks_usage_and_anonymous_counts() {
    let store = MemoryStore::default();
    blocks_hold_until_lifted(&store);
    captures_are_counted_per_user_and_address(&store);
    anonymous_ids_are_counted_per_address_per_day(&store);
}

#[test]
fn sqlite_store_blocks_usage_and_anonymous_counts() {
    let store = SqliteStore::open_in_memory().unwrap();
    blocks_hold_until_lifted(&store);
    captures_are_counted_per_user_and_address(&store);
    anonymous_ids_are_counted_per_address_per_day(&store);
}

/// An anonymous id is a database row, so a restart keeps it working.
#[test]
fn an_anonymous_id_survives_a_reopen() {
    let dir = unique_temp_dir();
    let path = dir.join("rambl.db");
    const ANON: &str = "anon_0123456789abcdef0123456789abcdef";
    {
        let store = SqliteStore::open(&path).unwrap();
        store.ensure_anonymous(ANON, at(9)).unwrap();
        store.put(ANON, anonymous_task("a1", None)).unwrap();
    }
    let reopened = SqliteStore::open(&path).unwrap();
    assert!(reopened.anonymous_exists(ANON).unwrap());
    assert_eq!(reopened.tasks(ANON).unwrap().len(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn memory_store_passes_the_contract() {
    users_are_isolated(&MemoryStore::new());
    store_contract(&MemoryStore::new());
    due_tz_round_trips(&MemoryStore::new());
    off_planner_round_trips(&MemoryStore::new());
    series_round_trip(&MemoryStore::new());
    series_are_isolated(&MemoryStore::new());
}

#[test]
fn sqlite_store_passes_the_contract() {
    users_are_isolated(&SqliteStore::open_in_memory().unwrap());
    store_contract(&SqliteStore::open_in_memory().unwrap());
    due_tz_round_trips(&SqliteStore::open_in_memory().unwrap());
    off_planner_round_trips(&SqliteStore::open_in_memory().unwrap());
    series_round_trip(&SqliteStore::open_in_memory().unwrap());
    series_are_isolated(&SqliteStore::open_in_memory().unwrap());
}

/// A session finds its own user, and only while it is live, on the real database.
#[test]
fn sqlite_session_finds_its_user_until_it_expires() {
    let store = SqliteStore::open_in_memory().unwrap();
    let me = super::super::auth::Me {
        id: "alice".to_owned(),
        email: "alice@example.com".to_owned(),
        name: "alice".to_owned(),
        picture: None,
        plan: super::super::auth::Plan::Free,
    };
    store
        .upsert_user_and_session(&me, "token-alice", at(0), at(12))
        .unwrap();
    let found = store.session_user("token-alice", at(1)).unwrap();
    assert_eq!(found.map(|me| me.id), Some("alice".to_owned()));
    assert!(store.session_user("token-alice", at(13)).unwrap().is_none());
    assert!(store.session_user("nope", at(1)).unwrap().is_none());
}

// -- SQLite specifics the contract cannot cover through the trait.

#[test]
fn a_file_store_keeps_its_tasks_across_a_reopen() {
    let path = unique_temp_dir().join("nested").join("rambl.db");
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());

    {
        let store = SqliteStore::open(&path).unwrap();
        store.put(USER, task("t1", 0)).unwrap();
        store.put(USER, task("t2", 1)).unwrap();
    }

    // Fresh handle, same file: this is the restart case.
    let store = SqliteStore::open(&path).unwrap();
    let stored = store.tasks(USER).unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].id, TaskId("t1".into()));
    assert_eq!(stored[1].id, TaskId("t2".into()));

    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
}

#[test]
fn reopening_a_current_database_applies_nothing_and_loses_nothing() {
    let path = unique_temp_dir().join("rambl.db");
    let _ = std::fs::remove_dir_all(&path);

    let store = SqliteStore::open(&path).unwrap();
    store.put(USER, task("t1", 0)).unwrap();
    drop(store);

    // Second open runs migrate() again; it must be a no-op, not a rebuild.
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.tasks(USER).unwrap().len(), 1);
    assert_eq!(store.count(), 1);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn the_schema_is_at_the_version_the_code_expects() {
    let store = SqliteStore::open_in_memory().unwrap();
    let conn = store.conn.lock_recover();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn the_column_list_matches_the_table() {
    let store = SqliteStore::open_in_memory().unwrap();
    let conn = store.conn.lock_recover();
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info('v2_tasks')")
        .unwrap();
    let actual: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(stmt);

    let listed: Vec<&str> = TASK_COLUMNS
        .split(',')
        .map(|name| name.trim().trim_matches('"'))
        .collect();
    // `user_id` is the one column on the table that is not a task field: it
    // scopes every read and write rather than describing a task, so it is bound
    // separately rather than read into a `Task`.
    let mut expected = actual.clone();
    expected.retain(|name| name != "user_id");
    assert_eq!(
        listed, expected,
        "TASK_COLUMNS must list exactly the task columns, in order"
    );
    assert!(
        actual.contains(&"user_id".to_owned()),
        "tasks must be scoped to a user"
    );
}

#[test]
fn two_tasks_cannot_claim_one_uid() {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut first = task("t1", 0);
    first.external_uid = Some("shared".into());
    store.put(USER, first).unwrap();

    let mut second = task("t2", 1);
    second.external_uid = Some("shared".into());
    let error = store.put(USER, second).unwrap_err();
    assert!(
        matches!(error, StoreError::Conflict(_)),
        "expected a conflict, got {error:?}"
    );
    assert_eq!(store.count(), 1, "the clashing task was not stored");
}

#[test]
fn memory_store_reports_the_same_uid_conflict() {
    let store = MemoryStore::new();
    let mut first = task("t1", 0);
    first.external_uid = Some("shared".into());
    store.put(USER, first).unwrap();

    let mut second = task("t2", 1);
    second.external_uid = Some("shared".into());
    assert!(matches!(
        store.put(USER, second),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(store.tasks(USER).unwrap().len(), 1);
}

#[test]
fn re_importing_a_uid_updates_that_task_in_place() {
    let store = MemoryStore::new();
    let mut imported = task("t1", 0);
    imported.external_uid = Some("evt-1".into());
    imported.when = When::At(at(14));
    store.put(USER, imported).unwrap();

    // The event moved to Friday: same UID, new time.
    let existing = store.find_by_uid(USER, "evt-1").unwrap().expect("found");
    let mut moved = existing.clone();
    moved.when = When::At(at(17));
    store.put(USER, moved).unwrap();

    assert_eq!(
        store.find_by_uid(USER, "evt-1").unwrap().unwrap().when,
        When::At(at(17))
    );
}

#[test]
fn re_importing_same_uid_updates_not_duplicates() {
    let store = MemoryStore::new();
    let mut first = task("first", 0);
    first.external_uid = Some("re-uid".into());
    store.put(USER, first).unwrap();
    let existing = store.find_by_uid(USER, "re-uid").unwrap().expect("found");
    let mut updated = existing.clone();
    updated.title = "updated".into();
    store.put(USER, updated).unwrap();
    assert_eq!(
        store.tasks(USER).unwrap().len(),
        1,
        "re-import must not duplicate"
    );
    assert_eq!(
        store.find_by_uid(USER, "re-uid").unwrap().unwrap().title,
        "updated"
    );
}

#[test]
fn v2_never_touches_the_v1_tasks_table() {
    let store = SqliteStore::open_in_memory().unwrap();
    // A v1-shaped table in the same file must survive, untouched.
    store
        .conn
        .lock_recover()
        .execute_batch(
            "CREATE TABLE tasks (id TEXT PRIMARY KEY, title TEXT NOT NULL);
             INSERT INTO tasks (id, title) VALUES ('v1-1', 'v1 task');",
        )
        .unwrap();

    store.put(USER, task("t1", 0)).unwrap();

    let conn = store.conn.lock_recover();
    let v1: String = conn
        .query_row("SELECT title FROM tasks WHERE id = 'v1-1'", [], |row| {
            row.get(0)
        })
        .expect("the v1 row is still there");
    assert_eq!(v1, "v1 task");
}

#[test]
fn a_hand_edited_row_degrades_instead_of_failing_the_load() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .conn
        .lock_recover()
        .execute_batch(
            "INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, \
                                    done, \"order\", label, priority, source, created, external_uid, user_id) \
             VALUES ('odd', 'Odd', 30, 'whenever', 'not-a-date', 0, 0, \
                     'mauve', 'enormous', 'telepathy', 'yesterday', NULL, 'alice');",
        )
        .unwrap();

    let stored = store.tasks(USER).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].when, When::Anytime);
    assert_eq!(stored[0].label, Label::Other);
    assert_eq!(stored[0].priority, Priority::Medium);
    assert_eq!(stored[0].source, Source::Text);
}

#[test]
fn store_errors_read_as_something_a_user_can_see() {
    for error in [
        StoreError::NotFound,
        StoreError::Conflict("two tasks claim one uid".into()),
        StoreError::Backend(Failure::plain(at!(), "disk is full".into())),
    ] {
        let message = error.to_string();
        assert!(!message.is_empty(), "{error:?} rendered nothing");
        assert!(!message.contains('{'), "{error:?} leaked a format brace");
    }
}

#[test]
fn a_backend_failure_names_the_call_and_the_sqlite_code() {
    let sqlite = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error {
            code: rusqlite::ffi::ErrorCode::SystemIoFailure,
            extended_code: 266,
        },
        Some("disk I/O error".into()),
    );
    assert_eq!(
        backend(sqlite, "store.rs:42").to_string(),
        "storage failed at store.rs:42 (sqlite 266): disk I/O error"
    );
    assert_eq!(
        StoreError::Backend(Failure::plain("store.rs:7", "disk is full".into())).to_string(),
        "storage failed at store.rs:7: disk is full"
    );
}

#[test]
fn when_round_trips_through_its_two_columns() {
    for when in [When::Anytime, When::DueBy(at(17)), When::At(at(14))] {
        let (kind, time) = when_to_parts(when);
        assert_eq!(when_from_parts(kind, time.map(format_ts).as_deref()), when);
    }
    assert_eq!(when_to_parts(When::Anytime), ("anytime", None));
}

const DAY: Duration = Duration::days(1);

fn anon_id(n: u32) -> String {
    format!("anon_{n:032x}")
}

/// A signed-in user (plan `free`) with one task, so expiry has something to spare.
fn sign_in(store: &dyn Store, id: &str) {
    let me = super::super::auth::Me {
        id: id.to_owned(),
        email: format!("{id}@example.com"),
        name: id.to_owned(),
        picture: None,
        plan: super::super::auth::Plan::Free,
    };
    store
        .upsert_user_and_session(&me, &format!("token-{id}"), at(0), at(23))
        .unwrap();
    store.put(id, task("keep", 0)).unwrap();
}

/// Idle anonymous accounts go, with their tasks and their series, and nobody else
/// does.
fn idle_anonymous_accounts_expire(store: &dyn Store) {
    let now = at(12);
    let (just_inside, exactly_old, long_gone) = (anon_id(1), anon_id(2), anon_id(3));
    for (id, last_seen) in [
        (&just_inside, now - DAY * 30 + Duration::hours(1)), // 29 days 23 hours
        (&exactly_old, now - DAY * 30),                      // 30 days
        (&long_gone, now - DAY * 90),
    ] {
        store.ensure_anonymous(id, last_seen).unwrap();
        store.put(id, task(&format!("{id}-1"), 0)).unwrap();
        store.put(id, task(&format!("{id}-2"), 1)).unwrap();
        store
            .put_series(id, series(&format!("{id}-series"), 0))
            .unwrap();
        store
            .put_exception(
                id,
                &SeriesId(format!("{id}-series")),
                SeriesException {
                    date: on((2026, 10, 12)),
                    kind: ExceptionKind::Done,
                },
            )
            .unwrap();
    }
    sign_in(store, "carol");

    let removed = store.expire_anonymous(now - DAY * 30).unwrap();

    assert_eq!(removed, 2, "30 days and older go; 29 days 23 hours stays");
    assert!(store.anonymous_exists(&just_inside).unwrap());
    assert_eq!(store.tasks(&just_inside).unwrap().len(), 2);
    assert_eq!(
        store.series(&just_inside).unwrap().len(),
        1,
        "a live visitor keeps their series"
    );
    for gone in [&exactly_old, &long_gone] {
        assert!(!store.anonymous_exists(gone).unwrap(), "{gone} survived");
        assert!(store.tasks(gone).unwrap().is_empty(), "{gone} kept tasks");
        assert!(
            store.series(gone).unwrap().is_empty(),
            "{gone} kept a series that nobody can reach"
        );
        assert!(
            store
                .exceptions(gone, &SeriesId(format!("{gone}-series")))
                .unwrap()
                .is_empty(),
            "{gone} kept the exceptions of a series it no longer has"
        );
    }
    assert_eq!(
        store.tasks("carol").unwrap().len(),
        1,
        "a signed-in user is never expired"
    );
    assert_eq!(store.expire_anonymous(now - DAY * 30).unwrap(), 0);
}

/// A visit keeps an anonymous id alive, but is written at most once an hour.
fn visits_keep_anonymous_ids_alive_with_a_throttle(store: &dyn Store) {
    let start = at(0);
    let id = anon_id(7);
    store.ensure_anonymous(&id, start).unwrap();

    // Within the hour nothing is written, so the old time still expires it.
    store
        .touch_anonymous(&id, start + Duration::minutes(30))
        .unwrap();
    let throttled = store
        .expire_anonymous(start + Duration::minutes(10))
        .unwrap();
    assert_eq!(
        throttled, 1,
        "a visit inside the throttle must not be written"
    );

    let id = anon_id(8);
    store.ensure_anonymous(&id, start).unwrap();
    store.touch_anonymous(&id, start + DAY * 20).unwrap();
    assert_eq!(
        store.expire_anonymous(start + DAY * 19).unwrap(),
        0,
        "a visit after the throttle keeps the id alive"
    );
    assert!(store.anonymous_exists(&id).unwrap());

    // Signed-in users and unknown ids are ignored, not an error.
    sign_in(store, "dave");
    store.touch_anonymous("dave", at(1)).unwrap();
    store.touch_anonymous("nobody", at(1)).unwrap();
}

#[test]
fn both_stores_expire_idle_anonymous_accounts() {
    idle_anonymous_accounts_expire(&MemoryStore::new());
    idle_anonymous_accounts_expire(&SqliteStore::open_in_memory().unwrap());
}

#[test]
fn both_stores_throttle_activity_writes() {
    visits_keep_anonymous_ids_alive_with_a_throttle(&MemoryStore::new());
    visits_keep_anonymous_ids_alive_with_a_throttle(&SqliteStore::open_in_memory().unwrap());
}

/// A sign-in records the app's time, not the wall clock (AGENTS.md rule 4), so a
/// fixed clock in tests and a restored clock in production both hold.
#[test]
fn a_sign_in_records_the_app_clock() {
    let store = SqliteStore::open_in_memory().unwrap();
    let me = super::super::auth::Me {
        id: "carol".to_owned(),
        email: "carol@example.com".to_owned(),
        name: "carol".to_owned(),
        picture: None,
        plan: super::super::auth::Plan::Free,
    };
    store
        .upsert_user_and_session(&me, "token-carol", at(3), at(23))
        .unwrap();
    let conn = store.conn.lock_recover();
    let created = |sql: &str| -> String { conn.query_row(sql, [], |row| row.get(0)).unwrap() };
    assert_eq!(
        created("SELECT created_at FROM v2_users WHERE id = 'carol'"),
        format_ts(at(3))
    );
    assert_eq!(
        created("SELECT created_at FROM v2_sessions"),
        format_ts(at(3))
    );
}

/// A timestamp or date that will not parse (a hand-edited row) reads the same way
/// every time: a task's version that followed the wall clock would refuse every
/// edit as stale, and an exception guessed onto today would mark a day the user
/// never touched.
#[test]
fn unreadable_times_read_the_same_every_time() {
    let store = SqliteStore::open_in_memory().unwrap();
    store.put(USER, task("t1", 0)).unwrap();
    store.put_series(USER, series("s1", 0)).unwrap();
    store
        .put_exception(
            USER,
            &SeriesId("s1".into()),
            SeriesException {
                date: on((2026, 10, 12)),
                kind: ExceptionKind::Done,
            },
        )
        .unwrap();
    store
        .conn
        .lock_recover()
        .execute_batch(
            "UPDATE v2_tasks SET created = 'junk', updated_at = 'junk';
             UPDATE v2_series SET start_day = 'junk';
             UPDATE v2_series_exceptions SET date = 'junk';",
        )
        .unwrap();

    let task = store.tasks(USER).unwrap().remove(0);
    assert_eq!(task.created, DateTime::UNIX_EPOCH);
    assert_eq!(task.updated_at, DateTime::UNIX_EPOCH);
    assert_eq!(store.tasks(USER).unwrap()[0], task);
    let held = store.series(USER).unwrap();
    assert_eq!(
        held[0].start_day,
        at(9).date_naive(),
        "an unreadable start day is the day the series was made"
    );
    assert!(
        store
            .exceptions(USER, &SeriesId("s1".into()))
            .unwrap()
            .is_empty(),
        "an exception with no readable day applies to no day"
    );
}

#[test]
fn a_signed_in_user_is_kept_however_long_since_their_last_activity() {
    let store = SqliteStore::open_in_memory().unwrap();
    sign_in(&store, "erin");
    store
        .conn
        .lock_recover()
        .execute(
            "UPDATE v2_users SET last_activity_at = ?1 WHERE id = 'erin'",
            params![format_ts(at(12) - DAY * 60)],
        )
        .unwrap();

    assert_eq!(store.expire_anonymous(at(12) - DAY * 30).unwrap(), 0);
    assert_eq!(store.tasks("erin").unwrap().len(), 1);
}

/// `plan <email> anon` changes what an account may spend, not whether it is
/// anonymous: a signed-in account on that plan is still never expired.
fn a_signed_in_user_on_the_anon_plan_is_kept(store: &dyn Store) {
    sign_in(store, "frank");
    store
        .set_user_plan("frank", super::super::auth::Plan::Anon)
        .unwrap();

    assert_eq!(store.expire_anonymous(at(12) + DAY * 365).unwrap(), 0);
    assert_eq!(store.tasks("frank").unwrap().len(), 1);
}

#[test]
fn both_stores_keep_signed_in_users_on_the_anon_plan() {
    a_signed_in_user_on_the_anon_plan_is_kept(&MemoryStore::new());
    a_signed_in_user_on_the_anon_plan_is_kept(&SqliteStore::open_in_memory().unwrap());
}

/// A database as an earlier build left it: the first `version` migrations applied.
fn database_at(path: &Path, version: usize) -> Connection {
    let conn = Connection::open(path).unwrap();
    for migration in &MIGRATIONS[..version] {
        conn.execute_batch(migration).unwrap();
    }
    conn.pragma_update(None, "user_version", version as i64)
        .unwrap();
    conn
}

fn deleting_an_account_takes_its_day_settings_with_it(store: &dyn Store) {
    use super::super::auth::{Me, Plan};
    let dana = Me {
        id: "dana".into(),
        email: "dana@example.com".into(),
        name: "dana".into(),
        picture: None,
        plan: Plan::Free,
    };
    store
        .upsert_user_and_session(&dana, "tok-dana", at(0), at(23))
        .unwrap();
    let mine = todo_core::DaySettings {
        deadline: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        ..todo_core::DaySettings::default()
    };
    store.set_day_settings("dana", mine).unwrap();
    store.delete_user("dana").unwrap();
    assert_eq!(
        store.day_settings("dana").unwrap(),
        todo_core::DaySettings::default()
    );
}

/// Settings need an account to belong to, and an anonymous id's settings end when
/// the id is adopted: the same id issued again starts from the defaults.
fn day_settings_need_an_account_and_end_with_an_adopted_id(store: &dyn Store) {
    const ANON: &str = "anon_0123456789abcdef0123456789abcdef";
    let mine = todo_core::DaySettings {
        deadline: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        ..todo_core::DaySettings::default()
    };
    assert!(matches!(
        store.set_day_settings("nobody", mine),
        Err(StoreError::NotFound)
    ));
    store
        .set_day_settings(super::super::auth::LOCAL_USER, mine)
        .expect("the local user always has an account");

    store.ensure_anonymous(ANON, at(9)).unwrap();
    store.set_day_settings(ANON, mine).unwrap();
    store.adopt_anonymous(ANON, USER).unwrap();
    store.ensure_anonymous(ANON, at(10)).unwrap();
    assert_eq!(
        store.day_settings(ANON).unwrap(),
        todo_core::DaySettings::default()
    );
}

#[test]
fn both_stores_need_an_account_for_day_settings() {
    day_settings_need_an_account_and_end_with_an_adopted_id(&MemoryStore::new());
    day_settings_need_an_account_and_end_with_an_adopted_id(
        &SqliteStore::open_in_memory().unwrap(),
    );
}

#[test]
fn both_stores_forget_day_settings_with_the_account() {
    deleting_an_account_takes_its_day_settings_with_it(&MemoryStore::new());
    deleting_an_account_takes_its_day_settings_with_it(&SqliteStore::open_in_memory().unwrap());
}

#[test]
fn a_version_10_database_gains_day_settings_at_the_defaults() {
    let path = unique_temp_dir().join("rambl.db");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    {
        let conn = database_at(&path, 10);
        conn.execute_batch(
            "INSERT INTO v2_users (id, email, name, picture, created_at, plan)
                 VALUES ('dana', 'd@example.com', 'Dana', NULL, '2026-01-01T00:00:00+00:00', 'free');",
        )
        .unwrap();
    }
    let store = SqliteStore::open(&path).expect("a v10 database must open and migrate");
    assert_eq!(
        store.day_settings("dana").unwrap(),
        todo_core::DaySettings::default()
    );
    // Someone with no account row has the defaults too, but nothing to write to.
    assert_eq!(
        store.day_settings("nobody").unwrap(),
        todo_core::DaySettings::default()
    );
    assert!(matches!(
        store.set_day_settings("nobody", todo_core::DaySettings::default()),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn a_version_5_database_migrates_and_keeps_its_data() {
    let path = unique_temp_dir().join("rambl.db");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let before = Utc::now();
    {
        let conn = database_at(&path, 5);
        conn.execute_batch(
            "INSERT INTO v2_users (id, email, name, picture, created_at, plan) VALUES
                 ('anon_old', '', '', NULL, '2026-01-01T00:00:00+00:00', 'anon'),
                 ('carol', 'c@example.com', 'Carol', NULL, '2026-01-01T00:00:00+00:00', 'free');
             INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, done,
                 \"order\", label, priority, source, created, external_uid, user_id) VALUES
                 ('a1', 'Anon task', 30, 'anytime', NULL, 0, 0, 'work', 'medium', 'text',
                  '2026-02-03T04:05:06+00:00', NULL, 'anon_old'),
                 ('c1', 'Carol task', 30, 'anytime', NULL, 0, 0, 'work', 'medium', 'text',
                  '2026-02-03T04:05:06+00:00', NULL, 'carol');",
        )
        .unwrap();
    }

    let store = SqliteStore::open(&path).expect("a v5 database must open and migrate");

    let carol = store.tasks("carol").unwrap();
    assert_eq!(carol.len(), 1);
    assert_eq!(
        carol[0].updated_at, carol[0].created,
        "an existing task's version is when it was created"
    );
    assert_eq!(store.tasks("anon_old").unwrap().len(), 1);
    // Nobody is expired for the time before idleness was tracked: the migration
    // counts as their last visit.
    assert_eq!(store.expire_anonymous(before - DAY * 30).unwrap(), 0);
    assert!(store.anonymous_exists("anon_old").unwrap());
    assert_eq!(store.expire_anonymous(Utc::now() + DAY).unwrap(), 1);
    let version: i64 = store
        .conn
        .lock_recover()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// A version-7 database gains the two series tables, keeps what it had, and can hold a
/// series from then on.
#[test]
fn a_version_7_database_gains_the_series_tables() {
    let path = unique_temp_dir().join("rambl.db");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    {
        let conn = database_at(&path, 7);
        conn.execute_batch(
            "INSERT INTO v2_users (id, email, name, picture, created_at, plan) VALUES
                 ('dana', 'd@example.com', 'Dana', NULL, '2026-01-01T00:00:00+00:00', 'free');
             INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, done,
                 \"order\", label, priority, source, created, external_uid, updated_at,
                 due_tz, user_id) VALUES
                 ('d1', 'Dana task', 30, 'anytime', NULL, 0, 0, 'work', 'medium', 'text',
                  '2026-02-03T04:05:06+00:00', NULL, '2026-02-03T04:05:06+00:00', NULL,
                  'dana');",
        )
        .unwrap();
    }

    let store = SqliteStore::open(&path).expect("a v7 database must open and migrate");
    assert_eq!(store.tasks("dana").unwrap().len(), 1, "her task survives");
    assert!(
        store.series("dana").unwrap().is_empty(),
        "a database written before series existed starts with none"
    );
    store.put_series("dana", series("s1", 0)).unwrap();
    assert_eq!(store.series("dana").unwrap().len(), 1, "and holds one now");
    let version: i64 = store
        .conn
        .lock_recover()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// A version-8 database gains the v2_usage_totals table, keeps tasks and series.
#[test]
fn a_version_8_database_gains_usage_totals() {
    let path = unique_temp_dir().join("rambl.db");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    {
        let conn = database_at(&path, 8);
        conn.execute_batch(
            "INSERT INTO v2_users (id, email, name, picture, created_at, plan) VALUES
                 ('eve', 'e@example.com', 'Eve', NULL, '2026-01-01T00:00:00+00:00', 'free');
             INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, done,
                 \"order\", label, priority, source, created, external_uid, updated_at,
                 due_tz, user_id) VALUES
                 ('e1', 'Eve task', 30, 'anytime', NULL, 0, 0, 'work', 'medium', 'text',
                  '2026-02-03T04:05:06+00:00', NULL, '2026-02-03T04:05:06+00:00', NULL,
                  'eve');
             INSERT INTO v2_series (id, user_id, title, duration_min, weekdays, until,
                 start_day, time_of_day, zone, external_uid, created, \"order\") VALUES
                 ('ser1', 'eve', 'Weekly meeting', 60, '1', NULL, '2026-01-01',
                  '09:00:00', 'UTC', NULL, '2026-01-01T00:00:00+00:00', 0);",
        )
        .unwrap();
    }

    let store = SqliteStore::open(&path).expect("a v8 database must open and migrate");
    assert_eq!(store.tasks("eve").unwrap().len(), 1, "her task survives");
    assert_eq!(store.series("eve").unwrap().len(), 1, "her series survives");
    // v2_usage_totals table exists and is empty
    let has_table: bool = store
        .conn
        .lock_recover()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='v2_usage_totals')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(has_table, "v2_usage_totals table was created");
    let version: i64 = store
        .conn
        .lock_recover()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_version_11_database_migrates_onto_the_planner() {
    let path = unique_temp_dir().join("rambl.db");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    {
        let conn = database_at(&path, 11);
        // A v11 row has no off_planner column at all: this is the shape an install
        // running before the flag wrote, and every one of its tasks was planned.
        conn.execute_batch(
            "INSERT INTO v2_users (id, email, name, picture, created_at, plan) VALUES
                 ('dan', 'd@example.com', 'Dan', NULL, '2026-01-01T00:00:00+00:00', 'free');
             INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, done,
                 \"order\", label, priority, source, created, external_uid, updated_at,
                 due_tz, user_id) VALUES
                 ('d1', 'Dan task', 30, 'anytime', NULL, 0, 0,
                  'work', 'medium', 'text', '2026-02-03T04:05:06+00:00', NULL,
                  '2026-02-03T04:05:06+00:00', NULL, 'dan');",
        )
        .unwrap();
    }

    let store = SqliteStore::open(&path).expect("a v11 database must open and migrate");
    let dan = store.tasks("dan").unwrap();
    assert_eq!(dan.len(), 1, "the pre-flag task survives");
    assert!(
        !dan[0].off_planner,
        "a task written before the flag existed is still planned"
    );

    // And the column is there to be set afterwards, not just read as a default.
    let mut take_off = dan[0].clone();
    take_off.off_planner = true;
    store.put("dan", take_off).unwrap();
    assert!(store.tasks("dan").unwrap()[0].off_planner);

    let version: i64 = store
        .conn
        .lock_recover()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_version_6_database_migrates_its_tasks_without_a_due_zone() {
    let path = unique_temp_dir().join("rambl.db");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    {
        let conn = database_at(&path, 6);
        // A v6 row has no due_tz column at all: this is the shape an install
        // running before the feature wrote.
        conn.execute_batch(
            "INSERT INTO v2_users (id, email, name, picture, created_at, plan) VALUES
                 ('carol', 'c@example.com', 'Carol', NULL, '2026-01-01T00:00:00+00:00', 'free');
             INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, done,
                 \"order\", label, priority, source, created, external_uid, updated_at,
                 user_id) VALUES
                 ('c1', 'Carol task', 30, 'due_by', '2026-11-03T04:59:59+00:00', 0, 0,
                  'work', 'medium', 'text', '2026-02-03T04:05:06+00:00', NULL,
                  '2026-02-03T04:05:06+00:00', 'carol');",
        )
        .unwrap();
    }

    let store = SqliteStore::open(&path).expect("a v6 database must open and migrate");
    let carol = store.tasks("carol").unwrap();
    assert_eq!(carol.len(), 1, "the pre-feature task survives");
    assert_eq!(
        carol[0].when,
        When::DueBy(Utc.with_ymd_and_hms(2026, 11, 3, 4, 59, 59).unwrap())
    );
    assert_eq!(
        carol[0].due_tz, None,
        "a task written before due_tz existed has no zone"
    );
    let version: i64 = store
        .conn
        .lock_recover()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_database_in_the_middle_of_the_history_applies_only_what_it_lacks() {
    let path = unique_temp_dir().join("rambl.db");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    drop(database_at(&path, 3));

    SqliteStore::open(&path).expect("migrating from version 3 must not redo version 2");

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// An upgrade must not sign anyone out: a session issued before tokens were hashed
/// keeps working, is stored hashed afterwards, and is not hashed again on a reopen.
#[test]
fn a_version_9_session_survives_the_move_to_hashed_tokens() {
    let path = unique_temp_dir().join("rambl.db");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let plain = "0123456789abcdef0123456789abcdef";
    {
        let conn = database_at(&path, 9);
        conn.execute_batch(&format!(
            "INSERT INTO v2_users (id, email, name, picture, created_at, plan) VALUES
                 ('dana', 'dana@example.com', 'Dana', NULL, '2026-01-01T00:00:00Z', 'free');
             INSERT INTO v2_sessions (token, user_id, expires_at, created_at) VALUES
                 ('{plain}', 'dana', '2099-01-01T00:00:00Z', '2026-01-01T00:00:00Z');"
        ))
        .unwrap();
    }

    for _ in 0..2 {
        let store = SqliteStore::open(&path).unwrap();
        let me = store.session_user(plain, at(12)).unwrap();
        assert_eq!(me.map(|me| me.id), Some("dana".to_owned()));
        let stored: String = store
            .conn
            .lock_recover()
            .query_row("SELECT token FROM v2_sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stored, hash_token(plain));
        assert_ne!(stored, plain);
    }

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_token_hashes_to_its_known_sha256() {
    assert_eq!(
        hash_token("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

/// A folder of its own, so two tests writing at once cannot collide.
fn unique_temp_dir() -> std::path::PathBuf {
    let stem = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("rambl-store-{}-{stem}", std::process::id()))
}

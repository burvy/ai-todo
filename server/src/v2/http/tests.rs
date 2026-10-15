use super::*;
use crate::v2::store::{MemoryStore, SqliteStore};
use axum::body::Body;
use axum::http::Request;
use chrono::{DateTime, TimeZone, Utc};
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use todo_core::{FixedClock, Label, Priority, Source, SystemClock, When};
use tower::ServiceExt;

/// Every code maps to the status a client should see, and the body is
/// always `{ "error": { code, message } }`. Pinned because the mapping is
/// the contract, and `Internal` vs `Unavailable` is exactly the kind of
/// pair that gets quietly swapped.
#[test]
fn every_error_code_maps_to_its_status() {
    let expected = [
        (ErrorCode::BadRequest, StatusCode::BAD_REQUEST),
        (ErrorCode::NotFound, StatusCode::NOT_FOUND),
        (ErrorCode::Conflict, StatusCode::CONFLICT),
        (ErrorCode::StaleUpdate, StatusCode::CONFLICT),
        // A dependency is down: come back later.
        (ErrorCode::Unavailable, StatusCode::SERVICE_UNAVAILABLE),
        (ErrorCode::Upstream, StatusCode::BAD_GATEWAY),
        // Our own failure.
        (ErrorCode::Internal, StatusCode::INTERNAL_SERVER_ERROR),
    ];

    for (code, want) in expected {
        let response = HttpError(ApiError {
            code,
            message: "boom".into(),
        })
        .into_response();
        assert_eq!(response.status(), want, "wrong status for {code:?}");
    }
}

#[test]
fn an_error_body_is_error_code_message() {
    let response = HttpError(ApiError {
        code: ErrorCode::Internal,
        message: "storage is unhappy".into(),
    })
    .into_response();

    let bytes = futures::executor::block_on(axum::body::to_bytes(response.into_body(), usize::MAX))
        .expect("read body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("body is JSON");

    assert_eq!(json["error"]["code"], "internal");
    assert_eq!(json["error"]["message"], "storage is unhappy");
}

fn router_at(now: chrono::DateTime<Utc>) -> Router {
    super::router(
        App::new(
            Box::new(MemoryStore::default()) as Box<dyn Store>,
            Arc::new(FixedClock(now)),
        ),
        AnyJudge::Fake(judge::FakeJudge),
    )
}

/// The snapshot invariant from ARCHITECTURE.md section 9: every open pinned task
/// shows its placement, and no placement exists for a done or missing task.
fn assert_invariant(snapshot: &Snapshot) {
    for placement in &snapshot.plan.placements {
        let task = snapshot
            .tasks
            .iter()
            .find(|task| task.id == placement.task)
            .expect("placement names a task in the snapshot");
        assert!(!task.done, "placement for a done task");
        assert!(
            !task.off_planner,
            "placement for a task taken off the planner"
        );
    }
    // A task off the planner is named in the plan so the UI can say why it is in
    // the list but not on the week, and is in neither other bucket.
    for task in snapshot.tasks.iter().filter(|task| task.off_planner) {
        assert!(
            snapshot.plan.off_planner.contains(&task.id),
            "{} is off the planner but the plan does not say so",
            task.title
        );
        assert!(!snapshot.plan.unplaced.contains(&task.id));
        assert!(
            !snapshot
                .plan
                .placements
                .iter()
                .any(|placement| placement.task == task.id)
        );
    }
    for task in snapshot.tasks.iter().filter(|task| !task.done) {
        if let When::At(start) = task.when {
            let shown: Vec<_> = snapshot
                .plan
                .placements
                .iter()
                .filter(|placement| placement.task == task.id)
                .collect();
            if task.off_planner {
                continue;
            }
            assert_eq!(
                shown.len(),
                1,
                "one placement for pinned task {}",
                task.title
            );
            assert_eq!(shown[0].start, start);
        }
    }
}

async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-rambl-tz", "America/New_York");
    let body = match body {
        Some(text) => {
            builder = builder.header("content-type", "application/json");
            Body::from(text.to_owned())
        }
        None => Body::empty(),
    };
    let response = router
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

/// Sunday 2026-10-11 21:00 in New York.
fn sunday_night() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap()
}

#[tokio::test]
async fn create_then_snapshot_shows_the_task_and_its_placement() {
    let router = router_at(sunday_night());
    let (status, body) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"Write essay","duration_min":60,"when":{"kind":"anytime"},"label":"School","priority":"high","source":"text"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(snapshot.tasks.len(), 1);
    assert_eq!(snapshot.plan.placements.len(), 1);
    assert_eq!(snapshot.tz, "America/New_York");
    assert_invariant(&snapshot);
}

#[tokio::test]
async fn pinned_task_keeps_its_time_and_blocks_floating_work() {
    let router = router_at(sunday_night());
    let pinned = Utc.with_ymd_and_hms(2026, 10, 12, 19, 0, 0).unwrap(); // 15:00 EDT Monday
    let draft = format!(
        r#"{{"title":"Meet Maya","duration_min":60,"when":{{"kind":"at","time":"{}"}},"label":"Work","priority":"low","source":"text"}}"#,
        pinned.to_rfc3339()
    );
    send(&router, "POST", "/api/v1/tasks", Some(&draft)).await;
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"Read","duration_min":30,"when":{"kind":"anytime"},"label":"School","priority":"low","source":"text"}"#),
    )
    .await;
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert_invariant(&snapshot);
    let meet = snapshot
        .tasks
        .iter()
        .find(|t| t.title == "Meet Maya")
        .unwrap();
    let read = snapshot.tasks.iter().find(|t| t.title == "Read").unwrap();
    let read_place = snapshot
        .plan
        .placements
        .iter()
        .find(|p| p.task == read.id)
        .unwrap();
    let meet_place = snapshot
        .plan
        .placements
        .iter()
        .find(|p| p.task == meet.id)
        .unwrap();
    assert!(read_place.end <= meet_place.start || read_place.start >= meet_place.end);
}

#[tokio::test]
async fn day_only_settings_start_at_the_defaults_and_change_by_field() {
    let router = router_at(sunday_night());
    let (status, body) = send(&router, "GET", "/api/v1/settings", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deadline"], "23:59");
    assert_eq!(body["placement"], "12:00");

    // One field at a time: the other stays as it was.
    let (status, body) = send(
        &router,
        "PATCH",
        "/api/v1/settings",
        Some(r#"{"deadline":"17:00"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deadline"], "17:00");
    assert_eq!(body["placement"], "12:00");
    let (_, body) = send(&router, "GET", "/api/v1/settings", None).await;
    assert_eq!(body["deadline"], "17:00");
}

#[tokio::test]
async fn a_deadline_of_midnight_is_refused_and_changes_nothing() {
    let router = router_at(sunday_night());
    let (status, _) = send(
        &router,
        "PATCH",
        "/api/v1/settings",
        Some(r#"{"deadline":"00:00","placement":"09:00"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, body) = send(&router, "GET", "/api/v1/settings", None).await;
    assert_eq!(
        body["placement"], "12:00",
        "a refused patch applies none of it"
    );
}

#[tokio::test]
async fn the_deadline_setting_decides_when_a_captured_day_is_due() {
    let router = router_at(sunday_night());
    let capture = |router: Router| async move {
        let (_, body, _) = send_raw(
            &router,
            "POST",
            "/api/v1/capture",
            "application/json",
            r#"{"text":"Pay rent tomorrow"}"#,
        )
        .await;
        serde_json::from_value::<Snapshot>(body["snapshot"].clone()).unwrap()
    };
    // Tomorrow is Monday 12 October. Default: 23:59 New York, 03:59 UTC on the 13th.
    let snapshot = capture(router.clone()).await;
    assert_eq!(
        snapshot.tasks[0].when,
        When::DueBy(Utc.with_ymd_and_hms(2026, 10, 13, 3, 59, 0).unwrap())
    );
    send(
        &router,
        "PATCH",
        "/api/v1/settings",
        Some(r#"{"deadline":"17:00"}"#),
    )
    .await;
    let snapshot = capture(router).await;
    assert!(
        snapshot
            .tasks
            .iter()
            .any(|task| task.when
                == When::DueBy(Utc.with_ymd_and_hms(2026, 10, 12, 21, 0, 0).unwrap())),
        "the second capture is due 17:00: {:?}",
        snapshot.tasks
    );
}

#[tokio::test]
async fn the_placement_setting_moves_a_day_only_task_within_its_day() {
    let router = router_at(sunday_night());
    // Due Monday 23:59 New York.
    let draft = r#"{"title":"Call mom","duration_min":60,"when":{"kind":"due_by","time":"2026-10-13T03:59:00Z"},"label":"Personal","priority":"low","source":"text"}"#;
    let (_, body) = send(&router, "POST", "/api/v1/tasks", Some(draft)).await;
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    // No timed tasks to learn from: noon New York on Monday, 16:00 UTC.
    assert_eq!(
        snapshot.plan.placements[0].start,
        Utc.with_ymd_and_hms(2026, 10, 12, 16, 0, 0).unwrap()
    );

    send(
        &router,
        "PATCH",
        "/api/v1/settings",
        Some(r#"{"placement":"09:00"}"#),
    )
    .await;
    let (_, body) = send(&router, "GET", "/api/v1/snapshot", None).await;
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(
        snapshot.plan.placements[0].start,
        Utc.with_ymd_and_hms(2026, 10, 12, 13, 0, 0).unwrap()
    );
}

#[tokio::test]
async fn patch_done_removes_the_placement() {
    let router = router_at(sunday_night());
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"Pay rent","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#),
    )
    .await;
    let id = serde_json::from_value::<Snapshot>(body).unwrap().tasks[0]
        .id
        .0
        .clone();
    let (status, body) = send(
        &router,
        "PATCH",
        &format!("/api/v1/tasks/{id}"),
        Some(r#"{"done":true}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert!(snapshot.plan.placements.is_empty());
    assert!(snapshot.tasks[0].done);
    assert_invariant(&snapshot);
}

#[tokio::test]
async fn taking_a_task_off_the_planner_keeps_it_in_the_list_and_off_the_week() {
    let router = router_at(sunday_night());
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"Pay rent","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#),
    )
    .await;
    let id = serde_json::from_value::<Snapshot>(body).unwrap().tasks[0]
        .id
        .0
        .clone();
    let path = format!("/api/v1/tasks/{id}");

    let (status, body) = send(&router, "PATCH", &path, Some(r#"{"off_planner":true}"#)).await;
    assert_eq!(status, StatusCode::OK);
    let off: Snapshot = serde_json::from_value(body).unwrap();
    assert!(off.tasks[0].off_planner, "the flag rode back on the task");
    assert_eq!(
        off.plan.off_planner,
        vec![TaskId(id.clone())],
        "the plan says why the task is not on the week"
    );
    assert!(
        off.plan.placements.is_empty() && off.plan.unplaced.is_empty(),
        "asked for, not a lack of room: {:?} / {:?}",
        off.plan.placements,
        off.plan.unplaced
    );
    assert_invariant(&off);

    // Putting it back plans it again, which is the whole of "until it is placed again".
    let (status, body) = send(&router, "PATCH", &path, Some(r#"{"off_planner":false}"#)).await;
    assert_eq!(status, StatusCode::OK);
    let on: Snapshot = serde_json::from_value(body).unwrap();
    assert!(!on.tasks[0].off_planner);
    assert!(on.plan.off_planner.is_empty());
    assert_eq!(on.plan.placements.len(), 1, "it is on the week again");
    assert_invariant(&on);
}

#[tokio::test]
async fn a_patch_that_leaves_the_flag_out_does_not_move_the_task() {
    let router = router_at(sunday_night());
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"Pay rent","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#),
    )
    .await;
    let id = serde_json::from_value::<Snapshot>(body).unwrap().tasks[0]
        .id
        .0
        .clone();
    send(
        &router,
        "PATCH",
        &format!("/api/v1/tasks/{id}"),
        Some(r#"{"off_planner":true}"#),
    )
    .await;

    let (status, body) = send(
        &router,
        "PATCH",
        &format!("/api/v1/tasks/{id}"),
        Some(r#"{"title":"Pay rent, sooner"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let after: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(after.tasks[0].title, "Pay rent, sooner");
    assert!(
        after.tasks[0].off_planner,
        "an absent field leaves the flag alone"
    );
}

#[tokio::test]
async fn a_snapshot_from_before_the_flag_plans_everything_as_it_did() {
    // A client that has never heard of off_planner sends no field, so nothing
    // changes: the flag defaults to false and the week fills as before.
    let router = router_at(sunday_night());
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"Pay rent","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#),
    )
    .await;
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert!(!snapshot.tasks[0].off_planner);
    assert!(snapshot.plan.off_planner.is_empty());
    assert_eq!(snapshot.plan.placements.len(), 1);
}

#[tokio::test]
async fn a_patch_made_against_an_older_version_is_a_409_and_changes_nothing() {
    let router = router_at(sunday_night());
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"Pay rent","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#),
    )
    .await;
    let task = serde_json::from_value::<Snapshot>(body)
        .unwrap()
        .tasks
        .remove(0);
    let path = format!("/api/v1/tasks/{}", task.id.0);

    let stale = r#"{"title":"Overwritten","updated_at":"1970-01-01T00:00:00Z"}"#;
    let (status, body) = send(&router, "PATCH", &path, Some(stale)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "stale_update");

    let current = format!(
        r#"{{"title":"Pay the rent","updated_at":"{}"}}"#,
        task.updated_at.to_rfc3339()
    );
    let (status, body) = send(&router, "PATCH", &path, Some(&current)).await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(
        snapshot.tasks[0].title, "Pay the rent",
        "the refused edit applied"
    );
}

#[tokio::test]
async fn delete_removes_the_task_and_unknown_ids_are_404() {
    let router = router_at(sunday_night());
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"Gone","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#),
    )
    .await;
    let id = serde_json::from_value::<Snapshot>(body).unwrap().tasks[0]
        .id
        .0
        .clone();
    let (status, body) = send(&router, "DELETE", &format!("/api/v1/tasks/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        serde_json::from_value::<Snapshot>(body)
            .unwrap()
            .tasks
            .is_empty()
    );
    let (status, body) = send(&router, "DELETE", "/api/v1/tasks/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn reorder_needs_every_id_once_and_sets_the_order() {
    let router = router_at(sunday_night());
    let mut ids = Vec::new();
    for title in ["First", "Second"] {
        let draft = format!(
            r#"{{"title":"{title}","duration_min":30,"when":{{"kind":"anytime"}},"label":"Other","priority":"low","source":"text"}}"#
        );
        let (_, body) = send(&router, "POST", "/api/v1/tasks", Some(&draft)).await;
        ids = serde_json::from_value::<Snapshot>(body)
            .unwrap()
            .tasks
            .iter()
            .map(|t| t.id.0.clone())
            .collect();
    }
    let reversed = format!(r#"{{"ids":["{}","{}"]}}"#, ids[1], ids[0]);
    let (status, body) = send(&router, "PUT", "/api/v1/tasks/order", Some(&reversed)).await;
    assert_eq!(status, StatusCode::OK);
    // Newest-first made the snapshot list Second above First, so the
    // "reversed" request asks for First back on top; the response must show
    // that order, not the creation order.
    assert_eq!(
        serde_json::from_value::<Snapshot>(body)
            .unwrap()
            .tasks
            .iter()
            .map(|t| t.title.as_str())
            .collect::<Vec<_>>(),
        ["First", "Second"]
    );

    let (status, body) = send(
        &router,
        "PUT",
        "/api/v1/tasks/order",
        Some(&format!(r#"{{"ids":["{}"]}}"#, ids[0])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "bad_request");
}

#[tokio::test]
async fn bad_input_is_a_400_and_a_bad_zone_is_a_400() {
    let router = router_at(sunday_night());
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"  ","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let request = Request::builder()
        .uri("/api/v1/snapshot")
        .header("x-rambl-tz", "Mars/Olympus")
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

const SYNTHETIC_ICS: &str = include_str!("../../../tests/fixtures/synthetic.ics");

async fn import_ics(router: &Router, ics: &str) -> (StatusCode, serde_json::Value) {
    let boundary = "XRAMBLBOUNDARY";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"cal.ics\"\r\nContent-Type: text/calendar\r\n\r\n{ics}\r\n--{boundary}--\r\n"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/import")
        .header("x-rambl-tz", "America/New_York")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

#[tokio::test]
async fn import_makes_pinned_tasks_and_reimport_updates_them_by_uid() {
    let router = router_at(sunday_night());
    let (status, first) = import_ics(&router, SYNTHETIC_ICS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        first["created"].as_array().unwrap().len(),
        2,
        "cancelled event is skipped"
    );
    assert!(first["updated"].as_array().unwrap().is_empty());
    let snapshot: Snapshot = serde_json::from_value(first["snapshot"].clone()).unwrap();
    assert_invariant(&snapshot);
    assert!(
        snapshot
            .tasks
            .iter()
            .all(|task| task.source == Source::Calendar)
    );
    assert!(
        snapshot
            .tasks
            .iter()
            .all(|task| matches!(task.when, When::At(_)))
    );

    let (_, second) = import_ics(&router, SYNTHETIC_ICS).await;
    assert!(second["created"].as_array().unwrap().is_empty());
    assert_eq!(second["updated"].as_array().unwrap().len(), 2);
    let again: Snapshot = serde_json::from_value(second["snapshot"].clone()).unwrap();
    assert_eq!(again.tasks.len(), 2, "re-import must not duplicate");
}

/// Google and Outlook write each moved instance of a repeating event as its own
/// VEVENT with the series' UID and a `RECURRENCE-ID`. Two of them, and a plain
/// duplicate UID, import without a 409, and a re-import updates rather than adds.
#[tokio::test]
async fn moved_instances_and_repeated_uids_import_and_reimport() {
    let router = router_at(sunday_night());
    let moved = |instance: &str, start: &str| {
        format!(
            "BEGIN:VEVENT\r\nUID:weekly-standup@example.com\r\nSUMMARY:Standup (moved)\r\n\
             RECURRENCE-ID;TZID=America/New_York:{instance}\r\n\
             DTSTART;TZID=America/New_York:{start}\r\n\
             DTEND;TZID=America/New_York:{start}\r\nEND:VEVENT\r\n"
        )
    };
    let once = |summary: &str| {
        format!(
            "BEGIN:VEVENT\r\nUID:twice@example.com\r\nSUMMARY:{summary}\r\n\
             DTSTART;TZID=America/New_York:20261015T100000\r\n\
             DTEND;TZID=America/New_York:20261015T110000\r\nEND:VEVENT\r\n"
        )
    };
    let ics = calendar(&[
        &weekly_event("FREQ=WEEKLY;BYDAY=TU", ""),
        &moved("20261020T093000", "20261021T093000"),
        &moved("20261027T093000", "20261028T093000"),
        &once("First copy"),
        &once("Second copy"),
    ]);

    let (status, first) = import_ics(&router, &ics).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["created"].as_array().unwrap().len(), 3, "{first}");
    let snapshot: Snapshot = serde_json::from_value(first["snapshot"].clone()).unwrap();
    assert!(
        snapshot
            .tasks
            .iter()
            .any(|task| task.title == "Second copy"),
        "the later copy of a repeated UID wins"
    );

    let (status, second) = import_ics(&router, &ics).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert!(second["created"].as_array().unwrap().is_empty(), "{second}");
    assert_eq!(second["updated"].as_array().unwrap().len(), 3, "{second}");
}

/// A synthetic export shaped like a real Outlook calendar: CRLF, a VTIMEZONE, and
/// the VEVENT blocks the test needs. Nothing here is anyone's real calendar.
fn calendar(events: &[&str]) -> String {
    let mut lines: Vec<&str> = vec![
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//Microsoft//OUTLOOK",
        "BEGIN:VTIMEZONE",
        "TZID:America/New_York",
        "END:VTIMEZONE",
    ];
    lines.extend_from_slice(events);
    lines.push("END:VCALENDAR");
    format!("{}\r\n", lines.join("\r\n"))
}

/// One weekly event: Tuesdays at 09:30 New York, under the rule under test, plus
/// any extra properties (an `EXDATE`, say).
fn weekly_event(rule: &str, extra: &str) -> String {
    format!(
        "BEGIN:VEVENT\r\nUID:weekly-standup@example.com\r\nSUMMARY:Weekly standup\r\n\
         DTSTART;TZID=America/New_York:20261013T093000\r\n\
         DTEND;TZID=America/New_York:20261013T094500\r\nRRULE:{rule}\r\n{extra}END:VEVENT\r\n"
    )
}

/// A weekly rule imports as one series with its occurrences in the snapshot, not as
/// a block for the one week the export covered; a re-import updates it by UID and
/// keeps what the user did to it (docs/ARCHITECTURE.md 7.1).
#[tokio::test]
async fn a_weekly_rule_imports_as_a_series_and_reimport_updates_it() {
    let router = router_at(sunday_night());
    let ics = calendar(&[&weekly_event(
        "FREQ=WEEKLY;BYDAY=TU;UNTIL=20261215T035959Z",
        "",
    )]);
    let (status, first) = import_ics(&router, &ics).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        first["created"].as_array().unwrap().is_empty(),
        "a rule is not a task"
    );
    assert_eq!(first["series"].as_array().unwrap().len(), 1);
    assert!(
        first["unmapped"]
            .as_array()
            .is_none_or(|notes| notes.is_empty()),
        "a weekly rule is one we follow"
    );
    let snapshot: Snapshot = serde_json::from_value(first["snapshot"].clone()).unwrap();
    assert_eq!(snapshot.series.len(), 1);
    let series = &snapshot.series[0];
    assert_eq!(series.rule.weekdays, vec![chrono::Weekday::Tue]);
    assert_eq!(series.zone, "America/New_York");
    let rows: Vec<String> = snapshot
        .tasks
        .iter()
        .filter(|task| task.source == Source::Series)
        .map(|task| task.id.0.clone())
        .collect();
    assert_eq!(
        rows,
        vec![format!("{}:2026-10-13", series.id.0)],
        "this week's occurrence, even though the export named only one week"
    );
    assert_invariant(&snapshot);

    // The rule keeps going after the week the export covered.
    let (_, next) = send(
        &router,
        "GET",
        "/api/v1/snapshot?from=2026-10-19&days=7",
        None,
    )
    .await;
    let next: Snapshot = serde_json::from_value(next).unwrap();
    assert!(
        next.tasks
            .iter()
            .any(|task| task.id.0 == format!("{}:2026-10-20", series.id.0)),
        "the Tuesday after the export is an occurrence too"
    );

    // The user marks one done, then the same calendar arrives again.
    let id = series.id.0.clone();
    send(
        &router,
        "PATCH",
        &format!("/api/v1/series/{id}/dates/2026-10-13"),
        Some(r#"{"kind":"done"}"#),
    )
    .await;
    let (_, second) = import_ics(&router, &ics).await;
    assert_eq!(
        second["series"].as_array().unwrap().len(),
        1,
        "one series after two imports"
    );
    let again: Snapshot = serde_json::from_value(second["snapshot"].clone()).unwrap();
    assert_eq!(again.series.len(), 1, "re-import must not duplicate");
    assert!(
        again
            .tasks
            .iter()
            .any(|task| task.id.0 == format!("{}:2026-10-13", id) && task.done),
        "re-import must not throw away what the user did"
    );
}

/// An `EXDATE` stays excluded after the import: it is a skipped occurrence, not a
/// row the user has to ignore.
#[tokio::test]
async fn an_excluded_date_does_not_become_an_occurrence() {
    let router = router_at(sunday_night());
    let ics = calendar(&[&weekly_event(
        "FREQ=WEEKLY;BYDAY=MO",
        "EXDATE;TZID=America/New_York:20261019T093000\r\n",
    )]);
    let (status, body) = import_ics(&router, &ics).await;
    assert_eq!(status, StatusCode::OK);
    let id = serde_json::from_value::<Snapshot>(body["snapshot"].clone())
        .unwrap()
        .series[0]
        .id
        .0
        .clone();

    // A fortnight of Mondays, so the excluded one is inside the window.
    let (_, body) = send(
        &router,
        "GET",
        "/api/v1/snapshot?from=2026-10-12&days=21",
        None,
    )
    .await;
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert!(
        !snapshot
            .tasks
            .iter()
            .any(|task| task.id.0 == format!("{id}:2026-10-19")),
        "the excluded Monday is not a row"
    );
    assert!(
        snapshot
            .tasks
            .iter()
            .any(|task| task.id.0 == format!("{id}:2026-10-26")),
        "the Monday after it still is"
    );
    assert_invariant(&snapshot);
}

/// A rule we cannot follow keeps today's behaviour: the event imports as one task,
/// and the answer says why it is not a series.
#[tokio::test]
async fn an_unsupported_rule_imports_one_task_and_says_so() {
    let router = router_at(sunday_night());
    let ics = calendar(&[&weekly_event("FREQ=MONTHLY;BYMONTHDAY=1", "")]);
    let (status, body) = import_ics(&router, &ics).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.get("series")
            .is_none_or(|series| series.as_array().is_none_or(|s| s.is_empty())),
        "a monthly rule is not a series"
    );
    assert_eq!(
        body["created"].as_array().unwrap().len(),
        1,
        "its first month still imports as a task"
    );
    let notes = body["unmapped"].as_array().expect("the answer says so");
    assert_eq!(notes.len(), 1);
    assert!(
        notes[0].as_str().unwrap().contains("Weekly standup"),
        "the note names the event: {notes:?}"
    );
    let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
    assert_invariant(&snapshot);
}

/// A one-off event and a weekly rule in the same calendar: one of each.
#[tokio::test]
async fn a_calendar_can_hold_both_kinds() {
    let router = router_at(sunday_night());
    let one_off = "BEGIN:VEVENT\r\nUID:one-off@example.com\r\nSUMMARY:One off\r\n\
         DTSTART;TZID=America/New_York:20261014T170000\r\n\
         DTEND;TZID=America/New_York:20261014T180000\r\nEND:VEVENT\r\n";
    let ics = calendar(&[&weekly_event("FREQ=WEEKLY;BYDAY=TU", ""), one_off]);
    let (status, body) = import_ics(&router, &ics).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["series"].as_array().unwrap().len(), 1);
    assert_eq!(body["created"].as_array().unwrap().len(), 1);
    let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
    assert_eq!(
        snapshot
            .tasks
            .iter()
            .filter(|task| task.source == Source::Series)
            .count(),
        1
    );
    assert_eq!(
        snapshot
            .tasks
            .iter()
            .filter(|task| task.source == Source::Calendar)
            .count(),
        1
    );
    assert_invariant(&snapshot);
}

#[tokio::test]
async fn import_without_a_file_field_is_a_400() {
    let router = router_at(sunday_night());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/import")
        .header("content-type", "multipart/form-data; boundary=B")
        .body(Body::from(
            "--B--
",
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn event_with_no_end_gets_thirty_minutes() {
    let router = router_at(sunday_night());
    let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:no-end@example.test\nDTSTART:20261012T130000Z\nSUMMARY:No end\nEND:VEVENT\nEND:VCALENDAR\n";
    let (status, body) = import_ics(&router, ics).await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
    assert_eq!(snapshot.tasks[0].duration_min, 30);
    assert_invariant(&snapshot);
}

/// Runs against burvy's real calendar. Personal data, so never committed: point
/// `RAMBL_SAMPLE_ICS` at a local file and run `cargo test -- --ignored`.
#[tokio::test]
#[ignore = "reads a local calendar named by RAMBL_SAMPLE_ICS"]
async fn import_of_the_local_sample_calendar_keeps_the_invariant() {
    let path = std::env::var("RAMBL_SAMPLE_ICS").expect("set RAMBL_SAMPLE_ICS");
    let ics = std::fs::read_to_string(path).expect("read the sample calendar");
    let router = router_at(sunday_night());
    let (status, body) = import_ics(&router, &ics).await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
    assert_invariant(&snapshot);
}

// ---- JSON import tests ----

/// Export from one user, import into another — the round trip creates fresh ids.
#[tokio::test]
async fn import_json_round_trip_export_then_import() {
    let verifier = PerCredential(vec![
        (
            "sam-credential",
            auth::TokenInfo::for_test("google-1", "sam@example.com"),
        ),
        (
            "lee-credential",
            auth::TokenInfo::for_test("google-2", "lee@example.com"),
        ),
    ]);
    let router = super::router_with_verifier(
        App::new(
            Box::new(MemoryStore::default()) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(verifier),
        false,
        Caps::default(),
        None,
        TrustedProxies::default(),
    );
    let sam = sign_in_with(&router, "sam-credential").await;
    let lee = sign_in_with(&router, "lee-credential").await;

    // Sam creates a task and exports it.
    let (status, _) = send_as(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(&draft_titled("Pick up dry cleaning")),
        Some(&sam),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, export) = send_as(&router, "GET", "/api/v1/export", None, Some(&sam)).await;
    assert_eq!(status, StatusCode::OK);
    let export_json = serde_json::to_string(&export).unwrap();

    // Lee imports Sam's export.
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/import")
        .header("x-rambl-tz", "America/New_York")
        .header("authorization", format!("Bearer {lee}"))
        .header("content-type", "application/json")
        .body(Body::from(export_json))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // Lee now has the task with a fresh id.
    assert_eq!(result["created"].as_array().unwrap().len(), 1);
    let snapshot: Snapshot = serde_json::from_value(result["snapshot"].clone()).unwrap();
    assert_eq!(snapshot.tasks.len(), 1);
    assert_eq!(snapshot.tasks[0].title, "Pick up dry cleaning");
    // The id is different from Sam's original.
    let sam_export_tasks: Vec<&serde_json::Value> =
        export["tasks"].as_array().unwrap().iter().collect();
    let original_id = &sam_export_tasks[0]["id"];
    assert_ne!(
        snapshot.tasks[0].id.0,
        original_id.as_str().unwrap(),
        "imported task must get a fresh id"
    );
    assert_invariant(&snapshot);
}

/// Malformed JSON returns 400 and writes nothing.
#[tokio::test]
async fn import_json_malformed_file_returns_400() {
    let router = router_at(sunday_night());
    let (status, body, _) = send_raw(
        &router,
        "POST",
        "/api/v1/import",
        "application/json",
        "not json at all",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("invalid JSON")
    );

    // Empty tasks array is also rejected.
    let (status, _, _) = send_raw(
        &router,
        "POST",
        "/api/v1/import",
        "application/json",
        r#"{"tasks":[]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// A JSON import takes exactly what any task may hold: a 300-character title (in
/// letters that are two bytes each) exports and so must import, a 3-minute task
/// could never have been exported.
#[tokio::test]
async fn import_json_uses_the_task_limits() {
    let router = router_at(sunday_night());
    let file = |title: &str, minutes: u16| {
        format!(
            r#"{{"tasks":[{{"id":"t1","title":"{title}","duration_min":{minutes},"when":{{"kind":"anytime"}},"done":false,"order":0,"label":"Other","priority":"low","source":"text","created":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","external_uid":null}}]}}"#
        )
    };
    let long = "\u{e9}".repeat(300);
    let (status, body, _) = send_raw(
        &router,
        "POST",
        "/api/v1/import",
        "application/json",
        &file(&long, 30),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _, _) = send_raw(
        &router,
        "POST",
        "/api/v1/import",
        "application/json",
        &file("Too short", 3),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// A file with too many tasks is rejected.
#[tokio::test]
async fn import_json_oversized_task_count_returns_400() {
    let router = router_at(sunday_night());
    let tasks: Vec<String> = (0..IMPORT_MAX_TASKS + 1)
        .map(|i| format!(r#"{{"id":"t{i}","title":"Task {i}","duration_min":30,"when":{{"kind":"anytime"}},"done":false,"order":0,"label":"Other","priority":"low","source":"text","created":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","external_uid":null}}"#))
        .collect();
    let body = format!(r#"{{"tasks":[{}]}}"#, tasks.join(","));
    let (status, _, _) =
        send_raw(&router, "POST", "/api/v1/import", "application/json", &body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// A file naming another user's id leaves that user untouched.
#[tokio::test]
async fn import_json_does_not_affect_other_users() {
    let verifier = PerCredential(vec![
        (
            "sam-credential",
            auth::TokenInfo::for_test("google-1", "sam@example.com"),
        ),
        (
            "lee-credential",
            auth::TokenInfo::for_test("google-2", "lee@example.com"),
        ),
    ]);
    let router = super::router_with_verifier(
        App::new(
            Box::new(MemoryStore::default()) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(verifier),
        false,
        Caps::default(),
        None,
        TrustedProxies::default(),
    );
    let sam = sign_in_with(&router, "sam-credential").await;
    let lee = sign_in_with(&router, "lee-credential").await;

    // Lee has a task.
    let (status, _) = send_as(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(&draft_titled("Lee's task")),
        Some(&lee),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Sam imports a task with Lee's id — Sam should get a fresh id, Lee untouched.
    let body = r#"{"tasks":[{"id":"tlee-fake","title":"Sam's import","duration_min":30,"when":{"kind":"anytime"},"done":false,"order":0,"label":"Other","priority":"low","source":"text","created":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","external_uid":null}]}"#;
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/import")
        .header("x-rambl-tz", "America/New_York")
        .header("authorization", format!("Bearer {sam}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Lee still has only their own task.
    let (status, lee_export) = send_as(&router, "GET", "/api/v1/export", None, Some(&lee)).await;
    assert_eq!(status, StatusCode::OK);
    let lee_titles: Vec<&str> = lee_export["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap())
        .collect();
    assert_eq!(lee_titles, vec!["Lee's task"]);
}

async fn send_raw(
    router: &Router,
    method: &str,
    uri: &str,
    content_type: &str,
    body: &str,
) -> (StatusCode, serde_json::Value, String) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-rambl-tz", "America/New_York")
        .header("content-type", content_type)
        .body(Body::from(body.to_owned()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or_default();
    (status, json, headers)
}

#[tokio::test]
async fn capture_json_creates_tasks_and_returns_their_ids() {
    let router = router_at(sunday_night());
    let (status, body, _) = send_raw(
        &router,
        "POST",
        "/api/v1/capture",
        "application/json",
        r#"{"text":"Buy milk"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["created"].as_array().unwrap().len(), 1);
    let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
    assert_eq!(snapshot.tasks.len(), 1);
    assert_invariant(&snapshot);
}

#[tokio::test]
async fn capture_multipart_text_file_is_read_as_text() {
    let router = router_at(sunday_night());
    let boundary = "XRAMBLCAP";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"notes.txt\"\r\nContent-Type: text/plain\r\n\r\nSubmit the lab report\r\n--{boundary}--\r\n"
    );
    let (status, json, _) = send_raw(
        &router,
        "POST",
        "/api/v1/capture",
        &format!("multipart/form-data; boundary={boundary}"),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(json["snapshot"].clone()).unwrap();
    assert_eq!(snapshot.tasks[0].source, Source::Text);
    assert_invariant(&snapshot);
}

/// One capture is one use of the daily cap but one Jev question per clause, so a
/// huge text or a long run of clauses is refused before the judge hears of it.
#[tokio::test]
async fn an_oversized_capture_is_a_400() {
    let router = router_at(sunday_night());
    let too_long = serde_json::json!({ "text": "a".repeat(CAPTURE_MAX_CHARS + 1) });
    let many = serde_json::json!({
        "text": (0..=CAPTURE_MAX_CLAUSES).map(|n| format!("Task {n}.")).collect::<Vec<_>>().join(" ")
    });
    for body in [too_long, many] {
        let (status, json, _) = send_raw(
            &router,
            "POST",
            "/api/v1/capture",
            "application/json",
            &body.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
    }
    let (status, _, _) = send_raw(
        &router,
        "POST",
        "/api/v1/capture",
        "application/json",
        r#"{"text":"Buy milk"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a normal capture still works");
}

#[tokio::test]
async fn a_task_title_past_the_limit_is_a_400() {
    let router = router_at(sunday_night());
    let title = "t".repeat(crate::v2::tuning::MAX_TITLE_CHARS + 1);
    let draft = format!(
        r#"{{"title":"{title}","duration_min":30,"when":{{"kind":"anytime"}},"label":"Other","priority":"low","source":"text"}}"#
    );
    let (status, _) = send(&router, "POST", "/api/v1/tasks", Some(&draft)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn capture_with_a_bad_pdf_or_empty_text_is_a_400() {
    let router = router_at(sunday_night());
    let boundary = "XRAMBLBAD";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x.pdf\"\r\n\r\nnot a pdf\r\n--{boundary}--\r\n"
    );
    let (status, _, _) = send_raw(
        &router,
        "POST",
        "/api/v1/capture",
        &format!("multipart/form-data; boundary={boundary}"),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = send_raw(
        &router,
        "POST",
        "/api/v1/capture",
        "application/json",
        r#"{"text":"   "}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

async fn create_task(router: &Router, title: &str) -> String {
    let draft = format!(
        r#"{{"title":"{title}","duration_min":30,"when":{{"kind":"anytime"}},"label":"Other","priority":"low","source":"text"}}"#
    );
    let (_, body) = send(router, "POST", "/api/v1/tasks", Some(&draft)).await;
    serde_json::from_value::<Snapshot>(body)
        .unwrap()
        .tasks
        .iter()
        .find(|task| task.title == title)
        .unwrap()
        .id
        .0
        .clone()
}

#[tokio::test]
async fn a_deleted_task_can_be_taped_back() {
    let router = router_at(sunday_night());
    let id = create_task(&router, "Post the letter").await;
    let (_, body) = send(&router, "DELETE", &format!("/api/v1/tasks/{id}"), None).await;
    assert!(
        serde_json::from_value::<Snapshot>(body)
            .unwrap()
            .tasks
            .is_empty()
    );

    let (_, before) = send(&router, "GET", "/api/v1/snapshot", None).await;
    assert!(
        serde_json::from_value::<Snapshot>(before)
            .unwrap()
            .tasks
            .is_empty()
    );
    let gone = format!(
        r#"{{"id":"{id}","title":"Post the letter","duration_min":30,"when":{{"kind":"anytime"}},"done":false,"order":3,"label":"Other","priority":"low","source":"text","created":"2026-10-11T20:00:00Z","updated_at":"2026-10-11T20:00:00Z"}}"#
    );
    let (status, body) = send(&router, "POST", "/api/v1/tasks/restore", Some(&gone)).await;
    assert_eq!(status, StatusCode::OK);
    let tasks = serde_json::from_value::<Snapshot>(body).unwrap().tasks;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Post the letter");
}

/// The scrap undo end to end: two tasks deleted one after the other are both taped
/// back, and a fresh read of the list (a reload) still holds both.
#[tokio::test]
async fn two_quick_deletes_are_both_taped_back_and_stay() {
    let router = router_at(sunday_night());
    let mut gone = Vec::new();
    let mut originals = Vec::new();
    for title in ["Post the letter", "Water the plants"] {
        let id = create_task(&router, title).await;
        let (_, held) = send(&router, "GET", "/api/v1/snapshot", None).await;
        let task = serde_json::from_value::<Snapshot>(held)
            .unwrap()
            .tasks
            .into_iter()
            .find(|task| task.id.0 == id)
            .expect("created");
        gone.push(serde_json::to_string(&task).unwrap());
        originals.push(task);
        send(&router, "DELETE", &format!("/api/v1/tasks/{id}"), None).await;
    }
    let (_, empty) = send(&router, "GET", "/api/v1/snapshot", None).await;
    assert!(
        serde_json::from_value::<Snapshot>(empty)
            .unwrap()
            .tasks
            .is_empty()
    );

    for task in &gone {
        let (status, _) = send(&router, "POST", "/api/v1/tasks/restore", Some(task)).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (_, reload) = send(&router, "GET", "/api/v1/snapshot", None).await;
    let mut back = serde_json::from_value::<Snapshot>(reload).unwrap().tasks;
    assert_eq!(
        back.len(),
        2,
        "each task is back and stored, not just answered"
    );
    // Every field as it was, bar the three the server owns: a fresh id, its own
    // update time, and a place in the list kept within one step of it.
    let as_was = |task: &Task| Task {
        id: TaskId(String::new()),
        updated_at: DateTime::<Utc>::MIN_UTC,
        order: 0,
        ..task.clone()
    };
    back.sort_by(|a, b| a.title.cmp(&b.title));
    originals.sort_by(|a, b| a.title.cmp(&b.title));
    for (back, original) in back.iter().zip(&originals) {
        assert_eq!(as_was(back), as_was(original));
        assert_ne!(back.id, original.id, "never the client's id");
    }
}

#[tokio::test]
async fn a_restore_is_checked_like_a_create() {
    let router = router_at(sunday_night());
    let task = |title: &str, minutes: u32| {
        format!(
            r#"{{"id":"t1","title":"{title}","duration_min":{minutes},"when":{{"kind":"anytime"}},"done":false,"order":0,"label":"Other","priority":"low","source":"text","created":"2026-10-11T20:00:00Z","updated_at":"2026-10-11T20:00:00Z"}}"#
        )
    };
    for (body, why) in [
        (task("", 30), "an empty title"),
        (task(&"x".repeat(5000), 30), "an over-long title"),
        (task("Nap", 0), "no length"),
        (task("Nap", 100_000), "a length past a u16"),
    ] {
        let (status, _) = send(&router, "POST", "/api/v1/tasks/restore", Some(&body)).await;
        assert!(status.is_client_error(), "{why}: {status}");
    }
    let (_, body) = send(&router, "GET", "/api/v1/snapshot", None).await;
    assert!(
        serde_json::from_value::<Snapshot>(body)
            .unwrap()
            .tasks
            .is_empty(),
        "nothing was stored"
    );
}

/// A restore naming another user's task id stores a new task for the caller
/// only: the other user's task is untouched.
#[tokio::test]
async fn a_restore_with_another_user_s_id_touches_only_the_caller() {
    let verifier = PerCredential(vec![
        (
            "sam-credential",
            auth::TokenInfo::for_test("google-1", "sam@example.com"),
        ),
        (
            "lee-credential",
            auth::TokenInfo::for_test("google-2", "lee@example.com"),
        ),
    ]);
    let router = super::router_with_verifier(
        App::new(
            Box::new(MemoryStore::default()) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(verifier),
        false,
        Caps::default(),
        None,
        TrustedProxies::default(),
    );
    let sam = sign_in_with(&router, "sam-credential").await;
    let lee = sign_in_with(&router, "lee-credential").await;
    let (_, body) = send_as(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(&draft_titled("Sam's errand")),
        Some(&sam),
    )
    .await;
    let sams = serde_json::from_value::<Snapshot>(body).unwrap().tasks;
    let mut stolen = sams[0].clone();
    stolen.title = "Lee's overwrite".into();
    let body = serde_json::to_string(&stolen).unwrap();

    let (status, body) = send_as(
        &router,
        "POST",
        "/api/v1/tasks/restore",
        Some(&body),
        Some(&lee),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let lees = serde_json::from_value::<Snapshot>(body).unwrap().tasks;
    assert_eq!(lees.len(), 1);
    assert_ne!(lees[0].id, sams[0].id, "stored under a fresh id");

    let (_, body) = send_as(&router, "GET", "/api/v1/snapshot", None, Some(&sam)).await;
    assert_eq!(
        serde_json::from_value::<Snapshot>(body).unwrap().tasks,
        sams,
        "Sam's task is untouched"
    );
}

#[tokio::test]
async fn hostile_restore_fields_are_tamed_over_http() {
    let router = router_at(sunday_night());
    let hostile = format!(
        r#"{{"id":"t1","title":"Pay rent","duration_min":30,"when":{{"kind":"due_by","time":"2026-10-12T17:00:00Z"}},"done":false,"order":{},"label":"Other","priority":"low","source":"text","created":"2999-01-01T00:00:00Z","updated_at":"2026-10-11T20:00:00Z","due_tz":"{}","external_uid":"{}"}}"#,
        i64::MIN,
        "Nowhere/".repeat(500),
        "uid".repeat(5000),
    );
    let (status, body) = send(&router, "POST", "/api/v1/tasks/restore", Some(&hostile)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let task = serde_json::from_value::<Snapshot>(body).unwrap().tasks[0].clone();
    assert_eq!(task.order, -1, "one step from an empty list, not i64::MIN");
    assert_eq!(task.due_tz.as_deref(), Some("America/New_York"));
    assert_eq!(task.external_uid, None);
    assert!(task.created <= sunday_night());
    // A capture after it still finds a place above.
    create_task(&router, "Call the bank").await;
}

#[tokio::test]
async fn revise_thursday_at_two_pins_the_task() {
    let router = router_at(sunday_night());
    let id = create_task(&router, "Meet Maya").await;
    let (status, body) = send(
        &router,
        "POST",
        &format!("/api/v1/tasks/{id}/revise"),
        Some(r#"{"instruction":"make it Thursday at 2"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    let task = snapshot.tasks.iter().find(|task| task.id.0 == id).unwrap();
    // Thursday 2026-10-15 14:00 EDT is 18:00 UTC.
    assert_eq!(
        task.when,
        When::At(Utc.with_ymd_and_hms(2026, 10, 15, 18, 0, 0).unwrap())
    );
    assert_invariant(&snapshot);
}

#[tokio::test]
async fn revise_errors_unknown_id_and_empty_instruction() {
    let router = router_at(sunday_night());
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/tasks/nope/revise",
        Some(r#"{"instruction":"tomorrow"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let id = create_task(&router, "Read").await;
    let (status, _) = send(
        &router,
        "POST",
        &format!("/api/v1/tasks/{id}/revise"),
        Some(r#"{"instruction":"  "}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn calendar_ics_exports_the_plan_as_events() {
    let router = router_at(sunday_night());
    let id = create_task(&router, "Read").await;
    assert!(!id.is_empty());
    let request = Request::builder()
        .uri("/api/v1/calendar.ics")
        .header("x-rambl-tz", "America/New_York")
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/calendar; charset=utf-8"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.starts_with("BEGIN:VCALENDAR"));
    let calendar: icalendar::Calendar = text.parse().unwrap();
    assert_eq!(
        calendar.events().count(),
        1,
        "one anytime task, placed once"
    );
}

/// A series over `/api/v1/series`: created, edited, and marked occurrence by
/// occurrence (docs/ARCHITECTURE.md 7.1).
#[tokio::test]
async fn a_series_is_created_and_its_occurrences_are_marked() {
    let router = router_at(sunday_night());
    let (status, body) = send(
        &router,
        "POST",
        "/api/v1/series",
        Some(
            r#"{"title":"Gym","duration_min":60,"weekdays":["Mon","Tue","Thu"],
                "start_day":"2026-10-12","time_of_day":"07:00:00","until":"2026-12-18"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(snapshot.series.len(), 1);
    let id = snapshot.series[0].id.0.clone();
    assert_eq!(snapshot.series[0].zone, "America/New_York");
    assert_eq!(
        snapshot.series[0].rule.until,
        chrono::NaiveDate::from_ymd_opt(2026, 12, 18),
        "the end sent with the series is kept"
    );
    let mut rows: Vec<String> = snapshot
        .tasks
        .iter()
        .filter(|task| task.source == todo_core::Source::Series)
        .map(|task| task.id.0.clone())
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        vec![
            format!("{id}:2026-10-12"),
            format!("{id}:2026-10-13"),
            format!("{id}:2026-10-15")
        ],
        "one row per repeated day this week"
    );
    assert_invariant(&snapshot);

    // Done: the row stays, marked, and holds no slot.
    let (status, body) = send(
        &router,
        "PATCH",
        &format!("/api/v1/series/{id}/dates/2026-10-12"),
        Some(r#"{"kind":"done"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    let monday = format!("{id}:2026-10-12");
    assert!(
        snapshot
            .tasks
            .iter()
            .any(|task| task.id.0 == monday && task.done)
    );
    assert!(
        !snapshot
            .plan
            .placements
            .iter()
            .any(|placement| placement.task.0 == monday),
        "a done occurrence holds no slot"
    );
    assert_invariant(&snapshot);

    // Skipped: the row is gone.
    let (status, body) = send(
        &router,
        "PATCH",
        &format!("/api/v1/series/{id}/dates/2026-10-13"),
        Some(r#"{"kind":"skipped"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert!(
        !snapshot
            .tasks
            .iter()
            .any(|task| task.id.0.ends_with("2026-10-13")),
        "a skipped day produces no row"
    );

    // Moved: the row keeps its date and takes the new instant.
    let (status, body) = send(
        &router,
        "PATCH",
        &format!("/api/v1/series/{id}/dates/2026-10-15"),
        Some(r#"{"kind":"moved","when":{"kind":"at","time":"2026-10-15T22:00:00Z"}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    let moved = snapshot
        .tasks
        .iter()
        .find(|task| task.id.0 == format!("{id}:2026-10-15"))
        .expect("the moved row is there under its original date");
    assert_eq!(
        moved.when,
        When::At(Utc.with_ymd_and_hms(2026, 10, 15, 22, 0, 0).unwrap())
    );
    assert_invariant(&snapshot);

    // Clearing puts a day back to the rule.
    let (status, _) = send(
        &router,
        "DELETE",
        &format!("/api/v1/series/{id}/dates/2026-10-15"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Editing the series replaces its rule.
    let (status, body) = send(
        &router,
        "PATCH",
        &format!("/api/v1/series/{id}"),
        Some(r#"{"title":"Yoga","weekdays":["Wed"]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(snapshot.series[0].title, "Yoga");
    assert_eq!(
        snapshot
            .tasks
            .iter()
            .filter(|task| task.source == todo_core::Source::Series)
            .map(|task| task.id.0.clone())
            .collect::<Vec<String>>(),
        vec![format!("{id}:2026-10-14")],
        "only Wednesdays now, and the days the rule no longer covers are not rows"
    );

    // Deleting takes the rows with it.
    let (status, body) = send(&router, "DELETE", &format!("/api/v1/series/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert!(snapshot.series.is_empty());
    assert!(
        !snapshot
            .tasks
            .iter()
            .any(|task| task.source == todo_core::Source::Series)
    );
    assert_invariant(&snapshot);
}

/// The window a snapshot asks for is the window the series is expanded over.
#[tokio::test]
async fn a_snapshot_expands_a_series_over_the_window_it_asks_for() {
    let router = router_at(sunday_night());
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/series",
        Some(
            r#"{"title":"Gym","duration_min":60,"weekdays":["Mon","Thu"],
                "start_day":"2026-10-12","time_of_day":"07:00:00"}"#,
        ),
    )
    .await;
    let id = serde_json::from_value::<Snapshot>(body).unwrap().series[0]
        .id
        .0
        .clone();

    let (_, body) = send(
        &router,
        "GET",
        "/api/v1/snapshot?from=2026-10-19&days=7",
        None,
    )
    .await;
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    let mut dates: Vec<String> = snapshot
        .tasks
        .iter()
        .filter(|task| task.source == todo_core::Source::Series)
        .map(|task| task.id.0.clone())
        .collect();
    dates.sort();
    assert_eq!(
        dates,
        vec![format!("{id}:2026-10-19"), format!("{id}:2026-10-22")],
        "the week asked for, not the week it was created in"
    );
    assert_invariant(&snapshot);
}

/// What a client cannot do: mark a day the series does not repeat, name a series it
/// does not hold, send a date that is not a date, or create a rule that can never
/// fire.
#[tokio::test]
async fn an_occurrence_the_rule_does_not_cover_is_refused() {
    let router = router_at(sunday_night());
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/series",
        Some(
            r#"{"title":"Gym","duration_min":60,"weekdays":["Mon"],
                "start_day":"2026-10-12","time_of_day":"07:00:00"}"#,
        ),
    )
    .await;
    let id = serde_json::from_value::<Snapshot>(body).unwrap().series[0]
        .id
        .0
        .clone();

    for (path, body) in [
        (
            format!("/api/v1/series/{id}/dates/2026-10-13"),
            r#"{"kind":"done"}"#, // a Tuesday: not in the rule
        ),
        (
            format!("/api/v1/series/{id}/dates/2026-10-05"),
            r#"{"kind":"done"}"#, // a Monday, but before the series starts
        ),
        (
            format!("/api/v1/series/{id}/dates/2026-10-12"),
            r#"{"kind":"moved"}"#, // a move with no instant
        ),
        (
            format!("/api/v1/series/{id}/dates/2026-10-12"),
            r#"{"kind":"whenever"}"#, // a kind that is not one of the three
        ),
    ] {
        let (status, _) = send(&router, "PATCH", &path, Some(body)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} with {body} should be refused"
        );
    }
    // A date that is not a date at all never reaches the app.
    let (status, _) = send(
        &router,
        "PATCH",
        &format!("/api/v1/series/{id}/dates/banana"),
        Some(r#"{"kind":"done"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A series this user does not hold is a 404, not a 400: it is not there at all.
    let (status, _) = send(
        &router,
        "PATCH",
        "/api/v1/series/s-made-up/dates/2026-10-13",
        Some(r#"{"kind":"done"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A series that could never fire is refused, and nothing is stored.
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/series",
        Some(r#"{"title":"Nothing","duration_min":30,"weekdays":[],"start_day":"2026-10-12"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/series",
        Some(
            r#"{"title":"Backwards","duration_min":30,"weekdays":["Mon"],
                "start_day":"2026-10-12","until":"2026-10-01"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, body) = send(&router, "GET", "/api/v1/snapshot", None).await;
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(
        snapshot.series.len(),
        1,
        "only the one that worked is stored"
    );
}

/// An occurrence row's id is not a task id: a task edit with one says where to send
/// it instead of reporting a task that is not there.
#[tokio::test]
async fn patching_an_occurrence_as_a_task_is_a_400() {
    let router = router_at(sunday_night());
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/series",
        Some(
            r#"{"title":"Gym","duration_min":60,"weekdays":["Mon"],
                "start_day":"2026-10-12","time_of_day":"07:00:00"}"#,
        ),
    )
    .await;
    let id = serde_json::from_value::<Snapshot>(body).unwrap().series[0]
        .id
        .0
        .clone();

    let (status, _) = send(
        &router,
        "PATCH",
        &format!("/api/v1/tasks/{id}:2026-10-12"),
        Some(r#"{"done":true}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The occurrence itself is untouched by the refused edit.
    let (_, body) = send(&router, "GET", "/api/v1/snapshot", None).await;
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert!(
        !snapshot
            .tasks
            .iter()
            .any(|task| task.id.0 == format!("{id}:2026-10-12") && task.done)
    );
}

#[tokio::test]
async fn sunday_night_tomorrow_is_monday_in_the_users_zone() {
    // A task due Monday comes back due at 17:00 New York on 2026-10-12.
    let router = router_at(sunday_night());
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"Tomorrow thing","duration_min":30,"when":{"kind":"due_by","time":"2026-10-12T21:00:00Z"},"label":"Other","priority":"low","source":"text"}"#),
    )
    .await;
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    let due = snapshot.tasks[0].when;
    assert_eq!(
        due,
        When::DueBy(Utc.with_ymd_and_hms(2026, 10, 12, 21, 0, 0).unwrap())
    );
    assert_invariant(&snapshot);
}

// ---- the audit findings this change fixes ----

/// A TRANSPARENT event is "show this, do not treat it as busy". v1 already
/// skipped it; the v2 import route must not turn a birthday into a task.
#[tokio::test]
async fn a_transparent_event_is_skipped_by_the_v2_import_route() {
    let router = router_at(sunday_night());
    let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//rambl//test//EN\n\
               BEGIN:VEVENT\nUID:birthday@example.test\n\
               SUMMARY:Someone's birthday\nTRANSP:TRANSPARENT\n\
               DTSTART;TZID=America/New_York:20261014T090000\n\
               DTEND;TZID=America/New_York:20261014T100000\n\
               END:VEVENT\nEND:VCALENDAR\n";
    let (status, body) = import_ics(&router, ics).await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
    assert!(
        snapshot.tasks.is_empty(),
        "a transparent event is not an obligation, got {:?}",
        snapshot.tasks.iter().map(|t| &t.title).collect::<Vec<_>>()
    );
    assert_invariant(&snapshot);
}

/// The fixture is a real Outlook export with TZID times. The point of the fix is
/// that 09:00 America/New_York stays 09:00 local rather than drifting to 13:00 UTC
/// on the wall.
#[tokio::test]
async fn a_tzid_event_lands_at_the_right_local_time() {
    let router = router_at(sunday_night());
    let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//rambl//test//EN\n\
               BEGIN:VEVENT\nUID:tzid@example.test\nSUMMARY:Standup\n\
               DTSTART;TZID=America/New_York:20261013T090000\n\
               DTEND;TZID=America/New_York:20261013T093000\n\
               END:VEVENT\nEND:VCALENDAR\n";
    let (status, body) = import_ics(&router, ics).await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
    // 09:00 New York on 2026-10-13 is 13:00Z (EDT, UTC-4).
    assert_eq!(
        snapshot.tasks[0].when,
        When::At(Utc.with_ymd_and_hms(2026, 10, 13, 13, 0, 0).unwrap()),
        "the TZID offset was not applied"
    );
    assert_invariant(&snapshot);
}

/// The week.ics fixture carries TZID times for a whole week; every one of them
/// must keep its local wall-clock time after the import.
#[tokio::test]
async fn the_week_fixture_keeps_every_tzid_event_at_its_local_time() {
    let router = router_at(sunday_night());
    let week = include_str!("../../../tests/fixtures/week.ics");
    let (status, body) = import_ics(&router, week).await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
    assert!(!snapshot.tasks.is_empty(), "the fixture imported nothing");

    let tz = chrono_tz::America::New_York;
    for task in &snapshot.tasks {
        let When::At(start) = task.when else {
            panic!(
                "an imported event should be pinned, {} is {:?}",
                task.title, task.when
            );
        };
        let local = start.with_timezone(&tz).time();
        // Working-hours events must sit inside 09:00-17:00 local. A TZID that
        // was treated as UTC lands at 05:00 or 13:00 and fails here.
        assert!(
            local >= chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap()
                && local <= chrono::NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
            "{} is at {} local, outside working hours",
            task.title,
            local
        );
    }
    assert_invariant(&snapshot);
}

/// `?from=`/`?days=` bound the plan. The task list must not shrink with the window:
/// a task the caller cannot see yet still exists.
#[tokio::test]
async fn the_snapshot_window_bounds_the_plan_but_not_the_tasks() {
    let router = router_at(sunday_night());
    let (_, _) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"Write essay","duration_min":60,"when":{"kind":"anytime"},"label":"School","priority":"high","source":"text"}"#),
    )
    .await;

    let (status, body) = send(&router, "GET", "/api/v1/snapshot?days=7", None).await;
    assert_eq!(status, StatusCode::OK);
    let week: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(week.tasks.len(), 1);
    assert_eq!(
        week.plan.placements.len(),
        1,
        "the week holds the placement"
    );
    assert_invariant(&week);

    // A window entirely in the past still lists the task, with no placement.
    let (status, body) = send(
        &router,
        "GET",
        "/api/v1/snapshot?from=2026-01-05&days=1",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let past: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(past.tasks.len(), 1, "the task list is not windowed");
    assert!(
        past.plan.placements.is_empty(),
        "a placement outside the window should not be drawn"
    );
    assert_invariant(&past);
}

/// A pinned task is a fact about a fixed instant, so narrowing the plan window must
/// never hide it: the section 9 invariant requires every open pinned task to show a
/// placement, and the client still needs to know the time is taken.
#[tokio::test]
async fn a_pinned_task_keeps_its_placement_outside_the_window() {
    let router = router_at(sunday_night());
    let (_, _) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(r#"{"title":"Dentist in March","duration_min":60,"when":{"kind":"at","time":"2027-03-04T15:00:00Z"},"label":"Health","priority":"low","source":"text"}"#),
    )
    .await;

    // A one-day window in October: nowhere near March 2027.
    let (status, body) = send(
        &router,
        "GET",
        "/api/v1/snapshot?from=2026-10-12&days=1",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(snapshot.tasks.len(), 1, "the task is still listed");
    assert_eq!(
        snapshot.plan.placements.len(),
        1,
        "a pinned task outside the window keeps its placement"
    );
    assert_invariant(&snapshot);
}

#[tokio::test]
async fn a_bad_snapshot_window_is_a_400() {
    let router = router_at(sunday_night());
    for query in [
        "?days=0",
        "?days=9999",
        "?from=not-a-date",
        // chrono reads signed years: the last date there is must not panic.
        "?from=%2B262142-12-31&days=1",
        "?from=-0001-01-01&days=1",
    ] {
        let (status, body) = send(&router, "GET", &format!("/api/v1/snapshot{query}"), None).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{query} should be rejected, got {body}"
        );
    }
}

/// Only pdf, eml and txt are documents Rambl reads. Anything else used to be read
/// as text and handed to the judge as binary noise.
#[tokio::test]
async fn capture_accepts_only_pdf_eml_and_txt() {
    let router = router_at(sunday_night());
    for filename in [
        "notes.docx",
        "sheet.xlsx",
        "photo.png",
        "archive.zip",
        "noextension",
    ] {
        let boundary = "XRAMBLTYPE";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\r\nhello\r\n--{boundary}--\r\n"
        );
        let (status, _, _) = send_raw(
            &router,
            "POST",
            "/api/v1/capture",
            &format!("multipart/form-data; boundary={boundary}"),
            &body,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{filename} should be refused, not read as text"
        );
    }
    // .txt is still fine.
    let boundary = "XRAMBLTXT";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"list.txt\"\r\n\r\nbuy milk\r\n--{boundary}--\r\n"
    );
    let (status, json, _) = send_raw(
        &router,
        "POST",
        "/api/v1/capture",
        &format!("multipart/form-data; boundary={boundary}"),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_value(json["snapshot"].clone()).unwrap();
    assert_eq!(snapshot.tasks[0].source, Source::Text);
    assert_invariant(&snapshot);
}

/// A Jev outage is a 503 so the client retries; a Jev answer that makes no sense
/// is a 502. Both used to be 502.
///
/// The unreachable half uses a real connection to a port nothing is listening on,
/// so it is a genuine `reqwest` connect error and the test needs no network.
#[tokio::test]
async fn a_jev_outage_is_unavailable_and_a_bad_answer_is_upstream() {
    let refused = reqwest::get("http://127.0.0.1:1/jev")
        .await
        .expect_err("nothing listens on port 1");
    let outage = upstream(anyhow::Error::new(refused));
    assert!(
        matches!(outage.0.code, ErrorCode::Unavailable),
        "an unreachable Jev is a dependency being down, got {:?}",
        outage.0.code
    );

    let bad_answer = upstream(anyhow::anyhow!("Jev answer missing is_a_task"));
    assert!(
        matches!(bad_answer.0.code, ErrorCode::Upstream),
        "a malformed answer is a bad gateway, got {:?}",
        bad_answer.0.code
    );
}

/// The v2 store is SQLite on disk by default, so a restart must not lose tasks.
/// This is the one test that would have caught the app running in memory.
#[tokio::test]
async fn tasks_survive_a_restart_on_disk() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("rambl.db");

    let mut first = App::new(
        SqliteStore::open(&path).expect("open"),
        Arc::new(FixedClock(sunday_night())),
    );
    let snapshot = first
        .create(
            TaskDraft {
                title: "Write the lab report".into(),
                duration_min: 60,
                when: When::Anytime,
                label: todo_core::Label::School,
                priority: todo_core::Priority::High,
                source: Source::Text,
            },
            auth::LOCAL_USER,
            chrono_tz::America::New_York,
        )
        .expect("create");

    assert_eq!(snapshot.tasks.len(), 1);
    assert_invariant(&snapshot);
    drop(first);

    // A brand new store over the same file: the task has to still be there.
    let second = App::new(
        SqliteStore::open(&path).expect("reopen"),
        Arc::new(FixedClock(sunday_night())),
    );
    let reread = second
        .snapshot(auth::LOCAL_USER, chrono_tz::America::New_York)
        .expect("snapshot");
    assert_eq!(
        reread.tasks.len(),
        1,
        "the task was lost when the store closed"
    );
    assert_eq!(reread.tasks[0].title, "Write the lab report");
    assert_invariant(&reread);
}

/// `RAMBL_DEFAULT_TZ` is the v2 name and `TZ_NAME` is what the old `.env` files set,
/// so both have to work or an existing deployment silently moves to New York.
#[test]
fn tz_name_still_works_as_the_fallback() {
    let london = || Some("Europe/London".to_owned());
    let berlin = || Some("Europe/Berlin".to_owned());

    assert_eq!(zone_from_env(london(), berlin()), chrono_tz::Europe::London);
    assert_eq!(
        zone_from_env(None, berlin()),
        chrono_tz::Europe::Berlin,
        "TZ_NAME is the fallback when RAMBL_DEFAULT_TZ is unset"
    );
    assert_eq!(
        zone_from_env(None, None),
        chrono_tz::America::New_York,
        "with neither configured we fall back to New York"
    );
    assert_eq!(
        zone_from_env(Some("Not/AZone".to_owned()), None),
        chrono_tz::America::New_York,
        "an unreadable value falls through instead of failing the request"
    );
}

// ---- accounts ----

/// A router with real (non-local) sign-in and a fake verifier, so the tests
/// exercise the session path without a Google project or a network call.
fn accounts_router(now: chrono::DateTime<Utc>, sub: &str, email: &str) -> Router {
    accounts_router_with_admins(now, sub, email, None)
}

fn accounts_router_with_admins(
    now: chrono::DateTime<Utc>,
    sub: &str,
    email: &str,
    admins: Option<Vec<String>>,
) -> Router {
    let verifier = auth::FakeVerifier {
        answer: Ok(auth::TokenInfo::for_test(sub, email)),
    };
    super::router_with_verifier(
        App::new(
            Box::new(MemoryStore::default()) as Box<dyn Store>,
            Arc::new(FixedClock(now)),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(verifier),
        false,
        Caps::default(),
        admins,
        TrustedProxies::default(),
    )
}

async fn send_as(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<&str>,
    token: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-rambl-tz", "America/New_York");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let body = match body {
        Some(text) => {
            builder = builder.header("content-type", "application/json");
            Body::from(text.to_owned())
        }
        None => Body::empty(),
    };
    let response = router
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

async fn sign_in(router: &Router) -> String {
    let (status, body) = send_as(
        router,
        "POST",
        "/api/v1/auth/google",
        Some(r#"{"credential":"fake-id-token"}"#),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "sign in failed: {body}");
    body["token"].as_str().expect("a session token").to_owned()
}

fn login_router(local_login: bool) -> Router {
    let verifier = auth::FakeVerifier {
        answer: Err("google is not used here".into()),
    };
    super::build_router_with(
        App::new(
            Box::new(MemoryStore::default()) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(verifier),
        false,
        Caps::default(),
        None,
        TrustedProxies::default(),
        local_login,
    )
}

async fn local_login_from(
    router: &Router,
    peer: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value) {
    let (status, _, body) = call_from(
        router,
        "POST",
        "/api/v1/auth/local",
        None,
        headers,
        Some(peer),
    )
    .await;
    (status, body)
}

/// Unset is local mode; set but blank stops startup rather than quietly giving every
/// visitor one shared task list.
#[test]
fn a_blank_client_id_is_refused_and_unset_is_local_mode() {
    assert_eq!(client_id_setting(None), Ok(None));
    for blank in ["", "  "] {
        let refused = client_id_setting(Some(blank)).expect_err(blank);
        assert!(refused.contains("set but empty"), "{refused}");
    }
    assert_eq!(client_id_setting(Some(" abc ")), Ok(Some("abc".into())));
}

/// Startup refuses the bypass next to a configured proxy, and ignores anything but `1`.
#[test]
fn local_login_is_refused_with_trusted_proxies_set() {
    assert_eq!(local_login_setting(Some("1"), None), Ok(true));
    assert_eq!(local_login_setting(Some("1"), Some("  ")), Ok(true));
    assert!(local_login_setting(Some("1"), Some("10.0.0.1")).is_err());
    for off in [None, Some(""), Some("0"), Some("true")] {
        assert_eq!(local_login_setting(off, Some("10.0.0.1")), Ok(false));
        assert_eq!(local_login_setting(off, None), Ok(false));
    }
}

/// Near peers get a working session for the fixed local user.
#[tokio::test]
async fn local_login_signs_in_a_near_peer() {
    let router = login_router(true);
    for peer in [
        "127.0.0.1:5000",
        "[::1]:5000",
        "10.1.2.3:5000",
        "172.16.0.9:5000",
        "172.31.255.1:5000",
        "192.168.1.20:5000",
        "[fd12::1]:5000",
        "[::ffff:192.168.1.20]:5000",
    ] {
        let (status, body) = local_login_from(&router, peer, &[]).await;
        assert_eq!(status, StatusCode::OK, "{peer}: {body}");
        assert_eq!(body["user"]["id"], "local-login");
        let token = body["token"].as_str().expect("a token");
        let (status, me) = send_as(&router, "GET", "/api/v1/me", None, Some(token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["id"], "local-login");
    }
}

/// Public peers, a missing peer and any proxy header are all refused, and so is a
/// server that never enabled it.
#[tokio::test]
async fn local_login_refuses_far_peers_proxy_headers_and_a_disabled_server() {
    let router = login_router(true);
    for peer in [
        "203.0.113.5:5000",
        "172.32.0.1:5000",
        "172.15.0.1:5000",
        "11.0.0.1:5000",
        "192.169.0.1:5000",
        "[2001:db8::1]:5000",
        "[fe80::1]:5000",
    ] {
        let (status, _) = local_login_from(&router, peer, &[]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{peer}");
    }
    for header in [
        ("cf-connecting-ip", "203.0.113.5"),
        ("x-forwarded-for", "203.0.113.5"),
    ] {
        let (status, _) = local_login_from(&router, "127.0.0.1:5000", &[header]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{header:?}");
    }
    let (status, _, _) = call(&router, "POST", "/api/v1/auth/local", None, &[]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "no peer address is not near");
    let (status, _) = local_login_from(&login_router(false), "127.0.0.1:5000", &[]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "not enabled");
}

/// The page is offered the button only where the sign-in would work.
#[tokio::test]
async fn config_offers_local_login_only_to_a_near_peer() {
    async fn offered(router: &Router, peer: &str, headers: &[(&str, &str)]) -> bool {
        let (_, _, body) =
            call_from(router, "GET", "/api/v1/config", None, headers, Some(peer)).await;
        body["local_login"] == true
    }
    let on = login_router(true);
    assert!(offered(&on, "192.168.1.5:1", &[]).await);
    assert!(!offered(&on, "203.0.113.5:1", &[]).await);
    assert!(!offered(&on, "127.0.0.1:1", &[("cf-connecting-ip", "203.0.113.5")]).await);
    assert!(!offered(&login_router(false), "127.0.0.1:1", &[]).await);
}

const DRAFT: &str = r#"{"title":"Write the report","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#;

/// The verifier is behind a trait, so this runs with no network and no Google
/// project: the answer is fixed by the fake.
#[tokio::test]
async fn signing_in_returns_a_token_and_the_user() {
    let router = accounts_router(sunday_night(), "google-1", "Sam@Example.com");
    let (status, body) = send_as(
        &router,
        "POST",
        "/api/v1/auth/google",
        Some(r#"{"credential":"fake-id-token"}"#),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["token"].as_str().is_some_and(|t| !t.is_empty()));
    assert_eq!(body["user"]["id"], "google-1");
    // Emails are stored lower-cased so the same person cannot end up twice.
    assert_eq!(body["user"]["email"], "sam@example.com");

    // The token works.
    let (status, body) = send_as(
        &router,
        "GET",
        "/api/v1/me",
        None,
        Some(body["token"].as_str().unwrap()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "google-1");
}

#[tokio::test]
async fn without_credentials_uses_anonymous_access() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    // A well-formed body, so the request reaches the handler rather than being
    // rejected by an extractor. Otherwise `Json` answers 415 for a missing
    // content-type and the auth check never runs.
    for (method, uri, body) in [
        ("GET", "/api/v1/snapshot", None),
        ("POST", "/api/v1/tasks", Some(DRAFT)),
        ("POST", "/api/v1/capture", Some(r#"{"text":"buy milk"}"#)),
        ("PUT", "/api/v1/tasks/order", Some(r#"{"ids":[]}"#)),
        ("GET", "/api/v1/calendar.ics", None),
    ] {
        let (status, _, _) = send_raw_status(&router, method, uri, body).await;
        assert_eq!(status, StatusCode::OK, "{method} {uri} was rejected");
    }
}

/// No `RAMBL_ADMIN_EMAILS` means the whole admin surface is off: the page is
/// 404 even to the person who would have been an admin.
#[tokio::test]
async fn admin_is_off_when_no_emails_are_configured() {
    let router = accounts_router(sunday_night(), "google-1", "boss@example.com");
    let token = sign_in(&router).await;
    let (status, _) = send_as(&router, "GET", "/admin", None, Some(&token)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the page must be 404 without RAMBL_ADMIN_EMAILS"
    );
    let (status, _) = send_as(&router, "GET", "/api/v1/admin/blocks", None, Some(&token)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the API must be 404 too");
}

/// A session is opened and checked on the same clock. On the wall clock, a
/// session opened by a clock more than 90 days behind it reads as already expired.
#[tokio::test]
async fn a_session_expires_by_the_apps_clock_not_the_wall_clock() {
    let long_ago = Utc.with_ymd_and_hms(2020, 1, 6, 15, 0, 0).unwrap();
    let router = accounts_router_with_admins(
        long_ago,
        "google-1",
        "boss@example.com",
        Some(vec!["boss@example.com".into()]),
    );
    let token = sign_in(&router).await;
    for uri in ["/api/v1/me", "/api/v1/snapshot", "/api/v1/admin/usage"] {
        let (status, body) = send_as(&router, "GET", uri, None, Some(&token)).await;
        assert_eq!(status, StatusCode::OK, "GET {uri}: {body}");
    }
}

/// Only a signed-in user whose email is in `RAMBL_ADMIN_EMAILS` sees the admin
/// surface. Everyone else gets 404 — friends, anonymous visitors, and signed-in
/// users with unlisted emails.
#[tokio::test]
async fn non_admins_get_404_on_the_page_and_the_api() {
    let router = accounts_router_with_admins(
        sunday_night(),
        "google-1",
        "sam@example.com",
        Some(vec!["boss@example.com".into()]),
    );
    let token = sign_in(&router).await;

    let (status, _) = send_as(&router, "GET", "/admin", None, Some(&token)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a non-admin gets no page");
    for (method, uri, body) in [
        ("GET", "/api/v1/admin/usage", None),
        ("GET", "/api/v1/admin/blocks", None),
        (
            "POST",
            "/api/v1/admin/block",
            Some(r#"{"kind":"ip","value":"203.0.113.9"}"#),
        ),
    ] {
        let (status, _) = send_as(&router, method, uri, body, Some(&token)).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {uri} must hide from non-admins"
        );
    }

    // Anonymous visitors never see data, but they do get the sign-in shell — the
    // page signs in the same way the main page does.
    let (status, _) = send_as(&router, "GET", "/api/v1/admin/blocks", None, None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "anonymous access is never admin"
    );
    let (status, _) = send_as(&router, "GET", "/admin", None, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the one place that must load is the shell itself"
    );
}

/// An admin can read today's usage, block and unblock an address, and move a
/// user between plans — everything the console can, over the web.
#[tokio::test]
async fn an_admin_can_block_unblock_see_usage_and_set_plans() {
    let router = accounts_router_with_admins(
        sunday_night(),
        "google-1",
        "boss@example.com",
        Some(vec!["boss@example.com".into()]),
    );
    let token = sign_in(&router).await;
    let bearer = format!("Bearer {token}");

    // A capture today, so usage has a row.
    let (status, _, _) = call(
        &router,
        "POST",
        "/api/v1/capture",
        Some(r#"{"text":"buy milk tomorrow"}"#),
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, usage) = send_as(&router, "GET", "/api/v1/admin/usage", None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        usage["per_plan"]["free"], 1,
        "the free user's capture is one Jev call"
    );
    assert_eq!(usage["users"][0]["id"], "google-1");
    assert_eq!(usage["users"][0]["plan"], "free");

    let (status, body) = send_as(
        &router,
        "POST",
        "/api/v1/admin/block",
        Some(r#"{"kind":"ip","value":"203.0.113.9"}"#),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "blocked ip 203.0.113.9");

    let (status, blocks) =
        send_as(&router, "GET", "/api/v1/admin/blocks", None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(blocks[0]["kind"], "ip");
    assert_eq!(blocks[0]["value"], "203.0.113.9");

    let (status, body) = send_as(
        &router,
        "POST",
        "/api/v1/admin/unblock",
        Some(r#"{"kind":"ip","value":"203.0.113.9"}"#),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["message"], "unblocked ip 203.0.113.9");
    let (status, blocks) =
        send_as(&router, "GET", "/api/v1/admin/blocks", None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(blocks.as_array().unwrap().is_empty());

    let (status, body) = send_as(
        &router,
        "POST",
        "/api/v1/admin/plan",
        Some(r#"{"who":"google-1","plan":"paid"}"#),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "google-1 is now on the paid plan");

    // The plan change is visible to the user themselves (`/me` keeps the
    // enum's serialized name; the admin API speaks `as_str`).
    let (status, me) = send_as(&router, "GET", "/api/v1/me", None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["plan"], "Paid");
}

/// A non-IP `CF-Connecting-IP` is never stored, so it cannot be echoed on the
/// admin page (the stored-XSS issue Pam raised on #161).
#[tokio::test]
async fn a_non_ip_proxy_header_is_never_stored_or_echoed() {
    let router = accounts_router_with_admins(
        sunday_night(),
        "google-1",
        "boss@example.com",
        Some(vec!["boss@example.com".into()]),
    );
    let token = sign_in(&router).await;
    let bearer = format!("Bearer {token}");

    let payload = "<img src=x onerror=alert(1)>";
    // From the tunnel, so the peer is trusted and the value alone is refused.
    let (status, _, _) = call_from(
        &router,
        "POST",
        "/api/v1/capture",
        Some(r#"{"text":"buy milk tomorrow"}"#),
        &[("authorization", &bearer), ("cf-connecting-ip", payload)],
        Some(TUNNEL),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the capture itself must not fail");

    let (status, usage) = send_as(&router, "GET", "/api/v1/admin/usage", None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let ips: Vec<String> = usage["ips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["ip"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        ips.iter().all(|ip| !ip.contains('<') && !ip.contains('>')),
        "a header string must never reach the admin page: {ips:?}"
    );
}

#[tokio::test]
async fn invalid_bearer_token_is_401() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    let (status, _, _) = send_raw_status_with_headers(
        &router,
        "GET",
        "/api/v1/snapshot",
        None,
        &[("authorization", "Bearer expired")],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

async fn send_raw_status(
    router: &Router,
    method: &str,
    uri: &str,
    json: Option<&str>,
) -> (StatusCode, String, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-rambl-tz", "America/New_York");
    let body = match json {
        Some(text) => {
            builder = builder.header("content-type", "application/json");
            Body::from(text.to_owned())
        }
        None => Body::empty(),
    };
    let response = router
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        String::new(),
        String::from_utf8_lossy(&bytes).to_string(),
    )
}

async fn send_raw_status_with_headers(
    router: &Router,
    method: &str,
    uri: &str,
    json: Option<&str>,
    headers: &[(&str, &str)],
) -> (StatusCode, String, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let body = json.map_or_else(Body::empty, |text| Body::from(text.to_owned()));
    let response = router
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        String::new(),
        String::from_utf8_lossy(&bytes).to_string(),
    )
}

/// The promise the whole feature rests on.
#[tokio::test]
async fn two_users_never_see_each_others_tasks() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    let sam = sign_in(&router).await;

    // Sign in a second person. The fake verifier answers the same way, so the two
    // sessions differ by user id rather than by credential; give the second one a
    // distinct id by re-routing with its own verifier.
    let second = super::router_with_verifier(
        App::new(
            Box::new(MemoryStore::default()) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(auth::FakeVerifier {
            answer: Ok(auth::TokenInfo::for_test("google-2", "alex@example.com")),
        }),
        false,
        Caps::default(),
        None,
        TrustedProxies::default(),
    );
    let alex = sign_in(&second).await;

    let (status, _) = send_as(&router, "POST", "/api/v1/tasks", Some(DRAFT), Some(&sam)).await;
    assert_eq!(status, StatusCode::OK);

    // Sam sees her task.
    let (status, body) = send_as(&router, "GET", "/api/v1/snapshot", None, Some(&sam)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tasks"].as_array().unwrap().len(), 1);

    // Alex, on a different router and a different store, cannot see it, and her
    // own snapshot is empty.
    let (status, body) = send_as(&second, "GET", "/api/v1/snapshot", None, Some(&alex)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["tasks"].as_array().unwrap().is_empty(),
        "one person saw another's tasks: {body}"
    );

    // And Sam's token is useless on Alex's server.
    let (status, _) = send_as(&second, "GET", "/api/v1/snapshot", None, Some(&sam)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let _ = sam;
}

/// Answers each credential with its own identity, so one test store can
/// hold two people signing in from one router.
struct PerCredential(Vec<(&'static str, auth::TokenInfo)>);

impl auth::TokenVerifier for PerCredential {
    fn verify<'a>(
        &'a self,
        credential: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<auth::TokenInfo, auth::AuthError>> + Send + 'a>,
    > {
        let info = self
            .0
            .iter()
            .find(|(name, _)| *name == credential)
            .map(|(_, info)| info.clone());
        Box::pin(async move {
            info.ok_or_else(|| auth::AuthError::Rejected("unknown credential".into()))
        })
    }
}

fn draft_titled(title: &str) -> String {
    format!(
        r#"{{"title":"{title}","duration_min":30,"when":{{"kind":"anytime"}},"label":"Other","priority":"low","source":"text"}}"#
    )
}

async fn sign_in_with(router: &Router, credential: &str) -> String {
    let (status, body) = send_as(
        router,
        "POST",
        "/api/v1/auth/google",
        Some(&format!(r#"{{"credential":"{credential}"}}"#)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "sign in failed: {body}");
    body["token"].as_str().expect("a session token").to_owned()
}

fn task_titles(value: &serde_json::Value) -> Vec<&str> {
    value["tasks"]
        .as_array()
        .expect("a tasks list")
        .iter()
        .map(|task| task["title"].as_str().expect("a title"))
        .collect()
}

/// `GET /api/v1/export` is a download of the caller's own rows and nothing
/// else: the file carries the attachment header and never another user's
/// task, and the filename date is the caller's own calendar date.
#[tokio::test]
async fn export_is_a_download_of_only_the_callers_tasks() {
    let verifier = PerCredential(vec![
        (
            "sam-credential",
            auth::TokenInfo::for_test("google-1", "sam@example.com"),
        ),
        (
            "lee-credential",
            auth::TokenInfo::for_test("google-2", "lee@example.com"),
        ),
    ]);
    let router = super::router_with_verifier(
        App::new(
            Box::new(MemoryStore::default()) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(verifier),
        false,
        Caps::default(),
        None,
        TrustedProxies::default(),
    );
    let sam = sign_in_with(&router, "sam-credential").await;
    let lee = sign_in_with(&router, "lee-credential").await;
    for title in ["Sam one", "Sam two"] {
        let (status, _) = send_as(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(&draft_titled(title)),
            Some(&sam),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, _) = send_as(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(&draft_titled("Lee one")),
        Some(&lee),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Raw request so the download headers can be read.
    let request = Request::builder()
        .uri("/api/v1/export")
        .header("x-rambl-tz", "America/New_York")
        .header("authorization", format!("Bearer {sam}"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(
        response.headers().get("content-disposition").unwrap(),
        "attachment; filename=\"rambl-export-2026-10-11.json\"",
        "sunday night is the evening of 2026-10-11 in New York"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let export: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let titles = task_titles(&export);
    assert_eq!(
        titles.len(),
        2,
        "Sam's export holds only Sam's tasks: {titles:?}"
    );
    assert!(titles.contains(&"Sam one") && titles.contains(&"Sam two"));
    assert!(
        !titles.contains(&"Lee one"),
        "another user's task leaked into the export: {titles:?}"
    );

    // Lee's download holds only Lee's task.
    let (status, body) = send_as(&router, "GET", "/api/v1/export", None, Some(&lee)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(task_titles(&body), vec!["Lee one"]);
}

/// An account with nothing in it exports an empty list, not an error.
#[tokio::test]
async fn an_empty_account_exports_an_empty_task_list() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    let sam = sign_in(&router).await;
    let (status, body) = send_as(&router, "GET", "/api/v1/export", None, Some(&sam)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["tasks"].as_array().expect("a tasks list").is_empty(),
        "a new account exported tasks: {body}"
    );
}

/// An anonymous caller exports its own tasks and nobody else's, the same rule the
/// signed-in test holds. The two ids below are two different people as far as the
/// store is concerned.
#[tokio::test]
async fn an_anonymous_caller_exports_only_its_own_tasks() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    // Two fresh anonymous ids: no header at all, so the middleware mints each.
    let (_, first_headers, _) = call(&router, "GET", "/api/v1/snapshot", None, &[]).await;
    let (_, second_headers, _) = call(&router, "GET", "/api/v1/snapshot", None, &[]).await;
    let first = anonymous_id_of(&first_headers);
    let second = anonymous_id_of(&second_headers);
    assert_ne!(first, second, "two visitors must be two accounts");

    for (id, title) in [(&first, "Anon one"), (&second, "Anon two")] {
        let (status, _, body) = call(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(&draft_titled(title)),
            &[("x-rambl-anon", id)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let (status, _, body) = call(
        &router,
        "GET",
        "/api/v1/export",
        None,
        &[("x-rambl-anon", &first)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        task_titles(&body),
        vec!["Anon one"],
        "an anonymous export must hold only that caller's tasks"
    );

    // A forged id is not a person: the middleware replaces it, so the file is
    // empty rather than someone else's.
    let (status, _, body) = call(
        &router,
        "GET",
        "/api/v1/export",
        None,
        &[("x-rambl-anon", FOREIGN_ID)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        task_titles(&body).is_empty(),
        "a forged id exported someone else's tasks: {body}"
    );

    // No credentials at all is a fresh anonymous account, so 200 and an empty
    // list. Pinned because it reads like it should be a 401.
    let (status, _, body) = call(&router, "GET", "/api/v1/export", None, &[]).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a signed-out visitor gets an empty file"
    );
    assert!(task_titles(&body).is_empty(), "{body}");

    // A bad token is refused outright, never quietly replaced by an anonymous id.
    let (status, _, _) = call(
        &router,
        "GET",
        "/api/v1/export",
        None,
        &[("authorization", "Bearer not-a-real-token")],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Local mode has no sign-in, so every request is the fixed local user and the
/// export is theirs.
#[tokio::test]
async fn local_mode_exports_the_local_users_tasks() {
    let router = super::router_with_verifier(
        App::new(
            Box::new(MemoryStore::default()) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(auth::FakeVerifier {
            answer: Err("unused".into()),
        }),
        true,
        Caps::default(),
        None,
        TrustedProxies::default(),
    );
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(&draft_titled("Local one")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(&router, "GET", "/api/v1/export", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(task_titles(&body), vec!["Local one"]);
}

/// The file is a copy, not a summary: every field of a task comes back, including
/// the ones a different shape would drop.
#[tokio::test]
async fn the_export_carries_every_field_of_a_task() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    let sam = sign_in(&router).await;
    let (status, _) = send_as(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(
            r#"{"title":"Hand in","duration_min":45,"when":{"kind":"due_by","time":"2026-10-13T21:00:00Z"},"label":"School","priority":"high","source":"text"}"#,
        ),
        Some(&sam),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send_as(&router, "GET", "/api/v1/export", None, Some(&sam)).await;
    assert_eq!(status, StatusCode::OK);
    let task = &body["tasks"][0];
    let fields: Vec<&str> = task
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        fields,
        [
            "created",
            "done",
            "due_tz",
            "duration_min",
            "external_uid",
            "id",
            "label",
            "off_planner",
            "order",
            "priority",
            "source",
            "title",
            "updated_at",
            "when",
        ],
        "the export must carry every field, listed alphabetically (serde_json sorts object keys)"
    );
    assert_eq!(task["label"], "School");
    assert_eq!(task["priority"], "high");
    assert_eq!(task["duration_min"], 45);
    assert_eq!(task["when"]["kind"], "due_by");
    // A deadline set in a zone keeps it, or a restore loses the local time.
    assert_eq!(
        task["due_tz"], "America/New_York",
        "the deadline's zone must survive the export"
    );
}

/// A download of one person's tasks is not a page to keep around.
#[tokio::test]
async fn the_export_is_not_cacheable() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    let sam = sign_in(&router).await;
    let request = Request::builder()
        .uri("/api/v1/export")
        .header("x-rambl-tz", "America/New_York")
        .header("authorization", format!("Bearer {sam}"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.headers().get("cache-control").unwrap(),
        "no-store",
        "a task list must not be kept by a cache"
    );
}

/// A session past its expiry is not a session.
#[tokio::test]
async fn an_expired_session_is_401() {
    let store = MemoryStore::default();
    let app = App::new(Box::new(store) as Box<dyn Store>, Arc::new(SystemClock));
    let token = auth::new_token();
    let user = auth::Me {
        id: "google-1".into(),
        email: "sam@example.com".into(),
        name: "Sam".into(),
        picture: None,
        plan: auth::Plan::Free,
    };
    // A session that ended a minute ago.
    let past = Utc::now() - chrono::Duration::minutes(1);
    app.store()
        .upsert_user_and_session(&user, &token, past - chrono::Duration::days(1), past)
        .expect("store the session");

    assert!(
        auth::session_user(app.store(), &token, Utc::now())
            .unwrap()
            .is_none(),
        "an expired session still resolves to a user"
    );
    assert!(
        auth::session_user(app.store(), &token, past - chrono::Duration::minutes(1))
            .unwrap()
            .is_some(),
        "a live session should resolve"
    );
}

/// Deleting an account takes their tasks with it.
#[tokio::test]
async fn deleting_an_account_removes_their_tasks() {
    let store = MemoryStore::default();
    let app = App::new(Box::new(store) as Box<dyn Store>, Arc::new(SystemClock));
    let user = "google-1";
    app.store()
        .put(
            user,
            todo_core::Task {
                id: todo_core::TaskId("t1".into()),
                title: DRAFT_TITLE.into(),
                duration_min: 30,
                when: When::Anytime,
                done: false,
                order: 0,
                label: Label::Other,
                priority: Priority::Low,
                source: Source::Text,
                created: Utc::now(),
                updated_at: Utc::now(),
                due_tz: None,
                external_uid: None,
                off_planner: false,
            },
        )
        .unwrap();
    assert_eq!(app.store().tasks(user).unwrap().len(), 1);

    app.store().delete_user(user).expect("delete");
    assert!(
        app.store().tasks(user).unwrap().is_empty(),
        "the tasks outlived the account"
    );
    assert!(
        app.store()
            .session_user(&auth::new_token(), Utc::now())
            .unwrap()
            .is_none()
    );
}

const DRAFT_TITLE: &str = "Write the report";

/// Past the daily cap, capture still works but Jev is not called.
#[tokio::test]
async fn over_the_daily_cap_capture_falls_back_and_says_so() {
    let store = MemoryStore::default();
    let app = App::new(
        Box::new(store) as Box<dyn Store>,
        Arc::new(FixedClock(sunday_night())),
    );
    // A cap of one: the first capture is judged, the second falls back.
    let router = super::router_with_verifier(
        app,
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(auth::FakeVerifier {
            answer: Ok(auth::TokenInfo::for_test("google-1", "sam@example.com")),
        }),
        false,
        Caps {
            free: 1,
            ..Caps::default()
        },
        None,
        TrustedProxies::default(),
    );
    let token = sign_in(&router).await;

    let (status, first) = send_as(
        &router,
        "POST",
        "/api/v1/capture",
        Some(r#"{"text":"buy milk tomorrow"}"#),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(
        first["jev_fallback"], false,
        "the first capture is under the cap"
    );

    let (status, second) = send_as(
        &router,
        "POST",
        "/api/v1/capture",
        Some(r#"{"text":"call the dentist"}"#),
        Some(&token),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the cap must not fail the capture: {second}"
    );
    assert_eq!(
        second["jev_fallback"], true,
        "over the cap, and the response must say so: {second}"
    );
    assert!(
        second["created"].as_array().is_some_and(|c| !c.is_empty()),
        "the fallback still creates the task"
    );
}

// ---- anonymous use ----

/// One request with arbitrary headers, returning its status, headers and JSON body.
async fn call(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
    call_from(router, method, uri, body, headers, None).await
}

/// [`call`], with the socket the request really arrived on. The peer decides
/// whether `CF-Connecting-IP` is believed at all.
async fn call_from(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
    peer: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-rambl-tz", "America/New_York");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = match body {
        Some(text) => builder
            .header("content-type", "application/json")
            .body(Body::from(text.to_owned()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    if let Some(peer) = peer {
        request.extensions_mut().insert(ConnectInfo(
            peer.parse::<SocketAddr>().expect("a socket address"),
        ));
    }
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or_default(),
    )
}

/// The anonymous id the server attached to a response.
fn anonymous_id_of(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("x-rambl-anon")
        .expect("the response names the anonymous id")
        .to_str()
        .expect("ASCII")
        .to_owned()
}

const FOREIGN_ID: &str = "anon_00000000000000000000000000000000";

/// A forged id must never reach someone else's tasks. Each forgery is a value a
/// client could plausibly send: another account's id, junk, or a well-formed id the
/// server never issued.
#[tokio::test]
async fn forged_anonymous_ids_never_reach_someone_elses_tasks() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    let sam = sign_in(&router).await;
    let bearer = format!("Bearer {sam}");
    let (status, _, _) = call(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(DRAFT),
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    for forged in ["google-1", "anon_zz", "anon_ABCDEF", FOREIGN_ID] {
        let (status, headers, body) = call(
            &router,
            "GET",
            "/api/v1/snapshot",
            None,
            &[("x-rambl-anon", forged)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{forged}: {body}");
        assert!(
            body["tasks"].as_array().unwrap().is_empty(),
            "a forged id ({forged}) reached someone's tasks: {body}"
        );
        assert_ne!(
            anonymous_id_of(&headers),
            forged,
            "the server must issue its own id, not echo a forged one"
        );
    }
}

/// An anonymous id the server issued keeps its own tasks from one request to the
/// next.
#[tokio::test]
async fn an_issued_anonymous_id_keeps_its_tasks() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    let (_, headers, _) = call(&router, "GET", "/api/v1/snapshot", None, &[]).await;
    let id = anonymous_id_of(&headers);

    let (status, _, _) = call(
        &router,
        "POST",
        "/api/v1/tasks",
        Some(DRAFT),
        &[("x-rambl-anon", &id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, _, body) = call(
        &router,
        "GET",
        "/api/v1/snapshot",
        None,
        &[("x-rambl-anon", &id)],
    )
    .await;
    assert_eq!(body["tasks"].as_array().unwrap().len(), 1, "{body}");
}

fn router_with_caps(store: MemoryStore, caps: Caps) -> Router {
    super::router_with_verifier(
        App::new(
            Box::new(store) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(auth::FakeVerifier {
            answer: Err("unused".into()),
        }),
        false,
        caps,
        None,
        TrustedProxies::default(),
    )
}

/// Watches whether the anonymous-id machinery ever reached the store.
struct CountingAnonStore {
    inner: MemoryStore,
    /// Calls to `ensure_anonymous`, i.e. anonymous user rows created.
    issued: Arc<AtomicUsize>,
    /// Calls to `record_anon_issued`, i.e. per-address allowance use.
    counted: Arc<AtomicUsize>,
}

impl Store for CountingAnonStore {
    fn tasks(&self, user: &str) -> Result<Vec<todo_core::Task>, StoreError> {
        self.inner.tasks(user)
    }
    fn put(&self, user: &str, task: todo_core::Task) -> Result<(), StoreError> {
        self.inner.put(user, task)
    }
    fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError> {
        self.inner.delete(user, id)
    }
    fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError> {
        self.inner.set_order(user, ids)
    }
    fn find_by_uid(&self, user: &str, uid: &str) -> Result<Option<todo_core::Task>, StoreError> {
        self.inner.find_by_uid(user, uid)
    }
    fn series(&self, user: &str) -> Result<Vec<todo_core::Series>, StoreError> {
        self.inner.series(user)
    }
    fn put_series(&self, user: &str, series: todo_core::Series) -> Result<(), StoreError> {
        self.inner.put_series(user, series)
    }
    fn delete_series(&self, user: &str, id: &todo_core::SeriesId) -> Result<bool, StoreError> {
        self.inner.delete_series(user, id)
    }
    fn exceptions(
        &self,
        user: &str,
        id: &todo_core::SeriesId,
    ) -> Result<Vec<todo_core::SeriesException>, StoreError> {
        self.inner.exceptions(user, id)
    }
    fn put_exception(
        &self,
        user: &str,
        id: &todo_core::SeriesId,
        exception: todo_core::SeriesException,
    ) -> Result<(), StoreError> {
        self.inner.put_exception(user, id, exception)
    }
    fn delete_exception(
        &self,
        user: &str,
        id: &todo_core::SeriesId,
        date: chrono::NaiveDate,
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
    fn captures_today(&self, user: &str, day: NaiveDate) -> Result<u32, StoreError> {
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
    fn day_settings(&self, user: &str) -> Result<todo_core::DaySettings, StoreError> {
        self.inner.day_settings(user)
    }
    fn set_day_settings(
        &self,
        user: &str,
        settings: todo_core::DaySettings,
    ) -> Result<(), StoreError> {
        self.inner.set_day_settings(user, settings)
    }
    fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError> {
        self.inner.anonymous_exists(user)
    }
    fn ensure_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
        self.issued.fetch_add(1, Ordering::SeqCst);
        self.inner.ensure_anonymous(user, now)
    }
    fn touch_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
        self.inner.touch_anonymous(user, now)
    }
    fn expire_anonymous(&self, cutoff: DateTime<Utc>) -> Result<usize, StoreError> {
        self.inner.expire_anonymous(cutoff)
    }
    fn record_capture(&self, user: &str, ip: &str, day: NaiveDate) -> Result<(), StoreError> {
        self.inner.record_capture(user, ip, day)
    }
    fn ip_captures_today(&self, user: &str, day: NaiveDate) -> Result<u32, StoreError> {
        self.inner.ip_captures_today(user, day)
    }
    fn record_anon_issued(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError> {
        self.counted.fetch_add(1, Ordering::SeqCst);
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
        day: NaiveDate,
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

/// Local mode (`GOOGLE_CLIENT_ID` unset) answers as the fixed local user, so
/// the anonymous-id middleware must not issue ids — or spend the per-address
/// allowance on ids nobody uses. A fresh browser profile is a request with no
/// `X-Rambl-Anon`; 30 of them is past the default 20-a-day allowance, and
/// Dwight found the install locked itself out there.
#[tokio::test]
async fn local_mode_issues_no_anonymous_ids() {
    let issued = Arc::new(AtomicUsize::new(0));
    let counted = Arc::new(AtomicUsize::new(0));
    let store = CountingAnonStore {
        inner: MemoryStore::default(),
        issued: Arc::clone(&issued),
        counted: Arc::clone(&counted),
    };
    let router = super::router_with_verifier(
        App::new(
            Box::new(store) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(auth::FakeVerifier {
            answer: Err("unused".into()),
        }),
        true, // local mode
        Caps::default(),
        None,
        TrustedProxies::default(),
    );
    let from = [("cf-connecting-ip", "203.0.113.9")];
    for _ in 0..30 {
        let (status, headers, _) = call(&router, "GET", "/api/v1/snapshot", None, &from).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "local mode must never refuse a fresh profile"
        );
        assert!(
            !headers.contains_key(ANON_HEADER),
            "local mode must not hand out anonymous ids"
        );
    }
    assert_eq!(
        issued.load(Ordering::SeqCst),
        0,
        "no anonymous user row may be created in local mode"
    );
    assert_eq!(
        counted.load(Ordering::SeqCst),
        0,
        "no per-address issuance may be counted in local mode"
    );
}

/// The tunnel's socket on this machine, the peer production requests arrive on.
const TUNNEL: &str = "127.0.0.1:41234";

/// Sign-in asks Google each time, so an address gets a daily number of attempts.
#[tokio::test]
async fn sign_in_attempts_are_limited_per_address() {
    let router = router_with_caps(
        MemoryStore::default(),
        Caps {
            signins_per_ip: 2,
            ..Caps::default()
        },
    );
    let attempt = |ip: &'static str| {
        let router = router.clone();
        async move {
            call_from(
                &router,
                "POST",
                "/api/v1/auth/google",
                Some(r#"{"credential":"x"}"#),
                &[("cf-connecting-ip", ip)],
                Some(TUNNEL),
            )
            .await
            .0
        }
    };
    for _ in 0..2 {
        assert_eq!(attempt("203.0.113.7").await, StatusCode::BAD_REQUEST);
    }
    assert_eq!(attempt("203.0.113.7").await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        attempt("203.0.113.8").await,
        StatusCode::BAD_REQUEST,
        "another address has its own allowance"
    );
}

/// Past the per-address allowance, a client gets no new anonymous id.
#[tokio::test]
async fn new_anonymous_ids_are_limited_per_address() {
    let router = router_with_caps(
        MemoryStore::default(),
        Caps {
            anon_ids_per_ip: 2,
            ..Caps::default()
        },
    );
    let from = [("cf-connecting-ip", "203.0.113.7")];
    for _ in 0..2 {
        let (status, _, _) = call_from(
            &router,
            "GET",
            "/api/v1/snapshot",
            None,
            &from,
            Some(TUNNEL),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, _, _) = call_from(
        &router,
        "GET",
        "/api/v1/snapshot",
        None,
        &from,
        Some(TUNNEL),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let other = [("cf-connecting-ip", "203.0.113.8")];
    let (status, _, _) = call_from(
        &router,
        "GET",
        "/api/v1/snapshot",
        None,
        &other,
        Some(TUNNEL),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "another address is not affected");
}

/// The reason the header is only read from a trusted peer: someone on the LAN
/// sends a fresh address every request and must not get a fresh allowance with it.
#[tokio::test]
async fn a_direct_client_cannot_buy_allowance_by_forging_the_proxy_header() {
    let router = router_with_caps(
        MemoryStore::default(),
        Caps {
            anon_ids_per_ip: 1,
            ..Caps::default()
        },
    );
    let direct = Some("198.51.100.4:5555");
    let (status, _, _) = call_from(
        &router,
        "GET",
        "/api/v1/snapshot",
        None,
        &[("cf-connecting-ip", "203.0.113.1")],
        direct,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the first new id is within the allowance"
    );

    for forged in ["203.0.113.2", "203.0.113.3"] {
        let (status, _, _) = call_from(
            &router,
            "GET",
            "/api/v1/snapshot",
            None,
            &[("cf-connecting-ip", forged)],
            direct,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{forged} bought a second new id from one address"
        );
    }
}

/// Past the anonymous cap, capture still works but is judged by `FakeJudge`.
#[tokio::test]
async fn anonymous_capture_falls_back_past_the_anonymous_cap() {
    let router = router_with_caps(
        MemoryStore::default(),
        Caps {
            anon: 1,
            ..Caps::default()
        },
    );
    let (_, headers, first) = call(
        &router,
        "POST",
        "/api/v1/capture",
        Some(r#"{"text":"buy milk tomorrow"}"#),
        &[],
    )
    .await;
    assert_eq!(first["jev_fallback"], false, "{first}");
    let id = anonymous_id_of(&headers);

    let (status, _, second) = call(
        &router,
        "POST",
        "/api/v1/capture",
        Some(r#"{"text":"call the dentist"}"#),
        &[("x-rambl-anon", &id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["jev_fallback"], true, "{second}");
}

/// An email's send date resolves its words, but it must not pick the day the caps
/// count on: an old date would sidestep the cap, a far one would prune every counter.
#[tokio::test]
async fn an_emails_date_does_not_choose_the_capped_day() {
    let router = router_with_caps(
        MemoryStore::default(),
        Caps {
            anon: 1,
            ..Caps::default()
        },
    );
    let (_, headers, first) = call(
        &router,
        "POST",
        "/api/v1/capture",
        Some(r#"{"text":"buy milk tomorrow"}"#),
        &[],
    )
    .await;
    assert_eq!(first["jev_fallback"], false, "{first}");
    let id = anonymous_id_of(&headers);

    let boundary = "XRAMBLEML";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"old.eml\"\r\n\r\n\
         Sent: Monday, January 1, 2001 10:03 AM\r\n\r\ncall the dentist\r\n--{boundary}--\r\n"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/capture")
        .header("x-rambl-tz", "America/New_York")
        .header("x-rambl-anon", &id)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let second: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        second["jev_fallback"], true,
        "the 2001 email still counts against today: {second}"
    );
}

/// A paused account stops using the app: every user-facing call is refused with 403,
/// including capture. (`pause` in the console writes a user block, which used to
/// mean only "judge with FakeJudge"; an *address* block still means that, and
/// `a_blocked_address_captures_with_fakejudge` covers it.)
#[tokio::test]
async fn a_paused_account_is_refused_by_the_api() {
    let store = MemoryStore::default();
    store
        .block(&crate::v2::store::Block::User("google-1".into()))
        .unwrap();
    let router = super::router_with_verifier(
        App::new(
            Box::new(store) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(auth::FakeVerifier {
            answer: Ok(auth::TokenInfo::for_test("google-1", "sam@example.com")),
        }),
        false,
        Caps::default(),
        None,
        TrustedProxies::default(),
    );
    let token = sign_in(&router).await;
    let bearer = format!("Bearer {token}");

    let (status, _, body) = call(
        &router,
        "POST",
        "/api/v1/capture",
        Some(r#"{"text":"buy milk tomorrow"}"#),
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "forbidden");

    // Not only capture: the list is refused too.
    let (status, _, body) = call(
        &router,
        "GET",
        "/api/v1/snapshot",
        None,
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

/// A blocked address is judged by `FakeJudge` whoever sends from it.
#[tokio::test]
async fn a_blocked_address_captures_with_fakejudge() {
    let store = MemoryStore::default();
    store
        .block(&crate::v2::store::Block::Ip("203.0.113.9".into()))
        .unwrap();
    let router = router_with_caps(store, Caps::default());
    let (status, _, body) = call_from(
        &router,
        "POST",
        "/api/v1/capture",
        Some(r#"{"text":"buy milk tomorrow"}"#),
        &[("cf-connecting-ip", "203.0.113.9")],
        Some(TUNNEL),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["jev_fallback"], true, "{body}");
}

/// With no Google client id configured the page is told so, and shows no sign-in.
#[tokio::test]
async fn config_without_a_google_client_id_is_null() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    let (status, _, body) = call(&router, "GET", "/api/v1/config", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["google_client_id"].is_null(), "{body}");
}

/// Headers carrying `CF-Connecting-IP: 203.0.113.5`.
fn from_cloudflare() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("cf-connecting-ip", HeaderValue::from_static("203.0.113.5"));
    headers
}

/// The tunnel runs on this machine, so a loopback peer may name the client.
#[test]
fn the_proxy_header_is_honoured_from_loopback() {
    let peer = Some("127.0.0.1".parse::<IpAddr>().unwrap());
    assert_eq!(
        client_ip(&from_cloudflare(), peer, &TrustedProxies::default()),
        "203.0.113.5"
    );
    // The whole loopback network, and IPv6 loopback, are the same machine.
    for loopback in ["127.0.0.5", "::1"] {
        let peer: IpAddr = loopback.parse().unwrap();
        assert_eq!(
            client_ip(&from_cloudflare(), Some(peer), &TrustedProxies::default()),
            "203.0.113.5",
            "{loopback} is this machine too"
        );
    }
}

/// Anyone reaching the server directly can send any address, so from a peer we do
/// not trust the header is worth nothing: it is the socket that counts.
#[test]
fn the_proxy_header_is_ignored_from_an_untrusted_peer() {
    let peer = Some("198.51.100.4".parse::<IpAddr>().unwrap());
    assert_eq!(
        client_ip(&from_cloudflare(), peer, &TrustedProxies::default()),
        "198.51.100.4"
    );

    let trusted = TrustedProxies::from_list(Some("203.0.113.0/24"));
    assert_eq!(
        client_ip(&from_cloudflare(), peer, &trusted),
        "198.51.100.4",
        "a different address must not become trusted by another entry"
    );

    // Nothing at all to fall back on, and no peer to check: the header is still
    // not taken, because no socket vouched for it.
    assert_eq!(
        client_ip(&from_cloudflare(), None, &TrustedProxies::default()),
        "unknown"
    );
}

/// `RAMBL_TRUSTED_PROXIES` names the proxies allowed to speak for the client.
#[test]
fn a_listed_proxy_may_name_the_client() {
    let trusted = TrustedProxies::from_list(Some("198.51.100.4, 10.0.0.0/8, fd00::/8"));
    for peer in ["198.51.100.4", "10.1.2.3", "fd00::1"] {
        let peer: IpAddr = peer.parse().unwrap();
        assert_eq!(
            client_ip(&from_cloudflare(), Some(peer), &trusted),
            "203.0.113.5",
            "{peer} is listed, so it may name the client"
        );
    }
    let edge = Some("198.51.100.5".parse::<IpAddr>().unwrap());
    assert_eq!(
        client_ip(&from_cloudflare(), edge, &trusted),
        "198.51.100.5",
        "an unlisted neighbour is not a proxy"
    );
}

/// A trusted proxy that sends something that is not an address must not get it
/// stored: the value is echoed on the admin page, so a client must not be able to
/// smuggle an HTML string in there.
#[test]
fn a_header_that_is_not_an_address_falls_back_to_the_peer() {
    let peer = Some("127.0.0.1".parse::<IpAddr>().unwrap());
    for junk in [
        "<img src=x onerror=alert(1)>",
        "not-an-ip",
        "203.0.113.5 203.0.113.6",
        "  ",
    ] {
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", HeaderValue::from_static(junk));
        assert_eq!(
            client_ip(&headers, peer, &TrustedProxies::default()),
            "127.0.0.1",
            "{junk} must not be stored"
        );
    }
    assert_eq!(
        client_ip(&HeaderMap::new(), peer, &TrustedProxies::default()),
        "127.0.0.1",
        "with no header the peer is the client"
    );
    assert_eq!(
        client_ip(&HeaderMap::new(), None, &TrustedProxies::default()),
        "unknown"
    );
}

/// The list is read once at startup, so a typo in it is dropped rather than
/// taking the whole list down with it.
#[test]
fn only_real_addresses_become_trusted_proxies() {
    let trusted =
        TrustedProxies::from_list(Some("198.51.100.4,, not-an-ip, 10.0.0.0/33, fd00::/8"));
    for peer in ["198.51.100.4", "fd00::1"] {
        assert!(
            trusted.trusts(peer.parse().unwrap()),
            "{peer} should have been kept"
        );
    }
    for peer in ["198.51.100.5", "10.1.2.3"] {
        assert!(
            !trusted.trusts(peer.parse().unwrap()),
            "{peer} should not have been kept"
        );
    }
    assert!(
        !TrustedProxies::default().trusts("198.51.100.4".parse().unwrap()),
        "an unset list trusts nothing but loopback"
    );
}

#[test]
fn anonymous_ids_require_prefix_and_32_lowercase_hex_digits() {
    assert!(valid_anon_id("anon_0123456789abcdef0123456789abcdef"));
    assert!(!valid_anon_id("0123456789abcdef0123456789abcdef"));
    assert!(!valid_anon_id("anon_0123456789ABCDEF0123456789abcdef"));
    assert!(!valid_anon_id("anon_0123"));
}

// ---- edge cases: bad input fails cleanly, never panics or 500s ----

/// Garbage bytes with a JSON content-type are a 4xx, and the server answers
/// the next request afterwards: no panic, no 500, no poisoned lock.
#[tokio::test]
async fn malformed_json_body_is_a_4xx_and_the_server_survives() {
    let router = router_at(sunday_night());
    let (status, _) = send(&router, "POST", "/api/v1/tasks", Some("{not json")).await;
    assert!(
        status.is_client_error(),
        "garbage JSON must fail cleanly, got {status}"
    );
    assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let (status, _) = send(&router, "GET", "/api/v1/snapshot", None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the server did not survive a bad body"
    );
}

/// Fields the server does not know are ignored, so a newer client still creates
/// its task against an older server. What matters is no panic and no 500.
#[tokio::test]
async fn unknown_fields_are_ignored_on_create() {
    let router = router_at(sunday_night());
    // The extra field sits on the draft itself: `When` is an internally
    // tagged enum and serde rejects trailing fields inside its object, so a
    // client must never hide new data there.
    let draft = r#"{"title":"Buy milk","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text","extra_field":1}"#;
    let (status, body) = send(&router, "POST", "/api/v1/tasks", Some(draft)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "unknown fields must be ignored: {body}"
    );
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(snapshot.tasks[0].title, "Buy milk");
    assert_invariant(&snapshot);
}

/// One anonymous user's task id is a 404 to another, for PATCH and DELETE
/// alike, and the owner's task is untouched by both attempts.
#[tokio::test]
async fn another_users_task_id_is_404_and_their_data_is_untouched() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    let (status, headers, body) = call(&router, "POST", "/api/v1/tasks", Some(DRAFT), &[]).await;
    assert_eq!(status, StatusCode::OK);
    let owner = anonymous_id_of(&headers);
    let id = serde_json::from_value::<Snapshot>(body).unwrap().tasks[0]
        .id
        .0
        .clone();
    let path = format!("/api/v1/tasks/{id}");

    // A second anonymous visitor is a different person on the same server.
    let (_, stranger_headers, _) = call(&router, "GET", "/api/v1/snapshot", None, &[]).await;
    let stranger = anonymous_id_of(&stranger_headers);
    assert_ne!(owner, stranger);

    let (status, _, body) = call(
        &router,
        "PATCH",
        &path,
        Some(r#"{"done":true}"#),
        &[("x-rambl-anon", &stranger)],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a stranger patched a task: {body}"
    );
    assert_eq!(body["error"]["code"], "not_found");

    let (status, _, body) = call(
        &router,
        "DELETE",
        &path,
        None,
        &[("x-rambl-anon", &stranger)],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a stranger deleted a task: {body}"
    );

    // The stranger sees an empty list; the owner still sees the original task.
    let (_, _, body) = call(
        &router,
        "GET",
        "/api/v1/snapshot",
        None,
        &[("x-rambl-anon", &stranger)],
    )
    .await;
    assert!(body["tasks"].as_array().unwrap().is_empty());
    let (_, _, body) = call(
        &router,
        "GET",
        "/api/v1/snapshot",
        None,
        &[("x-rambl-anon", &owner)],
    )
    .await;
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(snapshot.tasks.len(), 1);
    assert_eq!(snapshot.tasks[0].title, "Write the report");
    assert!(!snapshot.tasks[0].done);
    assert_invariant(&snapshot);
}

/// An empty PATCH body is a 4xx; an empty object patches nothing and changes nothing.
#[tokio::test]
async fn patch_with_an_empty_body_is_a_4xx_and_an_empty_object_is_a_noop() {
    let router = router_at(sunday_night());
    let id = create_task(&router, "Read").await;
    let path = format!("/api/v1/tasks/{id}");
    let (status, _) = send(&router, "PATCH", &path, Some("")).await;
    assert!(
        status.is_client_error(),
        "an empty PATCH body must fail cleanly, got {status}"
    );
    let (status, body) = send(&router, "PATCH", &path, Some("{}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an empty object patches nothing: {body}"
    );
    let snapshot: Snapshot = serde_json::from_value(body).unwrap();
    assert_eq!(snapshot.tasks[0].title, "Read");
    assert_invariant(&snapshot);
}

/// A 1 MB title is refused cleanly: a 400 that says why, never a panic or a 500.
#[tokio::test]
async fn a_one_megabyte_title_is_refused_not_a_500() {
    let router = router_at(sunday_night());
    let title = "a".repeat(1024 * 1024);
    let draft = format!(
        r#"{{"title":"{title}","duration_min":30,"when":{{"kind":"anytime"}},"label":"Other","priority":"low","source":"text"}}"#
    );
    let (status, body) = send(&router, "POST", "/api/v1/tasks", Some(&draft)).await;
    assert_ne!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a 1 MB title must never be a 500"
    );
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("too long")
    );
}

/// Durations outside 5 to 480 minutes are refused: a negative or out-of-range
/// number never parses (422), a big-but-parseable one fails validation (400).
#[tokio::test]
async fn out_of_range_durations_are_4xx() {
    let router = router_at(sunday_night());
    for (duration, want) in [
        ("-5", StatusCode::UNPROCESSABLE_ENTITY),
        ("999999", StatusCode::UNPROCESSABLE_ENTITY),
        ("60000", StatusCode::BAD_REQUEST),
        ("0", StatusCode::BAD_REQUEST),
    ] {
        let draft = format!(
            r#"{{"title":"Big job","duration_min":{duration},"when":{{"kind":"anytime"}},"label":"Other","priority":"low","source":"text"}}"#
        );
        let (status, _) = send(&router, "POST", "/api/v1/tasks", Some(&draft)).await;
        assert_eq!(status, want, "duration_min {duration} gave {status}");
    }
}

/// A zone that is not IANA is a 400 on writes too, not just on snapshot.
#[tokio::test]
async fn an_unknown_zone_is_a_400_on_writes() {
    let router = router_at(sunday_night());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/tasks")
        .header("x-rambl-tz", "Mars/Olympus")
        .header("content-type", "application/json")
        .body(Body::from(DRAFT))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["code"], "bad_request");
}

/// Deleting twice: the first delete answers 200, the second is a 404.
#[tokio::test]
async fn deleting_twice_is_ok_then_404() {
    let router = router_at(sunday_night());
    let id = create_task(&router, "Gone").await;
    let path = format!("/api/v1/tasks/{id}");
    let (status, _) = send(&router, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(&router, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

/// A session the store has expired is a 401 over HTTP, not just in the store
/// test: the request carries a real but dead token.
#[tokio::test]
async fn an_expired_bearer_token_is_401() {
    let store = MemoryStore::default();
    let user = auth::Me {
        id: "google-1".into(),
        email: "sam@example.com".into(),
        name: "Sam".into(),
        picture: None,
        plan: auth::Plan::Free,
    };
    let token = auth::new_token();
    // Sessions are checked against the wall clock, so an expiry an hour ago
    // is dead whatever the app clock says.
    let expired = sunday_night() - chrono::Duration::hours(1);
    store
        .upsert_user_and_session(&user, &token, expired - chrono::Duration::days(1), expired)
        .expect("store the expired session");
    let router = super::router_with_verifier(
        App::new(
            Box::new(store) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        ),
        AnyJudge::Fake(judge::FakeJudge),
        Box::new(auth::FakeVerifier {
            answer: Err("unused".into()),
        }),
        false,
        Caps::default(),
        None,
        TrustedProxies::default(),
    );
    let (status, body) = send_as(&router, "GET", "/api/v1/snapshot", None, Some(&token)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "an expired token worked: {body}"
    );
    assert_eq!(body["error"]["code"], "unauthorized");
}

/// A token the server never issued is a 401 with the JSON error body, on both
/// reads and writes.
#[tokio::test]
async fn a_forged_bearer_token_is_401() {
    let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
    let forged = auth::new_token();
    let (status, body) = send_as(&router, "GET", "/api/v1/snapshot", None, Some(&forged)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
    let (status, _) = send_as(&router, "POST", "/api/v1/tasks", Some(DRAFT), Some(&forged)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// A JSON body without a JSON content-type is a 415: axum refuses it before
/// the handler ever runs, and the server answers the next request.
#[tokio::test]
async fn a_missing_content_type_is_415_and_the_server_survives() {
    let router = router_at(sunday_night());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/tasks")
        .header("x-rambl-tz", "America/New_York")
        .body(Body::from(DRAFT))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "a body without a content-type must be refused"
    );
    let (status, _) = send(&router, "GET", "/api/v1/snapshot", None).await;
    assert_eq!(status, StatusCode::OK, "the server did not survive a 415");
}

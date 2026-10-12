//! Thin HTTP handlers for `/api/v1`. They read the time zone header, lock the `App`
//! for the whole call, and map errors. No business rules live here.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{FromRequest, Multipart, State},
    http::{HeaderMap, StatusCode, header::HeaderName},
    response::{IntoResponse, Response},
    routing::{get, patch, post, put},
};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use todo_core::{ApiError, ErrorCode, Snapshot, TaskDraft};

use super::app::{App, ImportResult, ImportedEvent, Patch};
use super::ics_export::build_ics;
use super::store::Store;
use capture::{Revision, capture_text};
use chrono::{DateTime, Utc};
use judge::AnyJudge;
use todo_core::{LocalContext, Source, TaskId};

const TZ_HEADER: HeaderName = HeaderName::from_static("x-rambl-tz");

/// The state every handler shares: the app (locked for each call) and the judge.
pub struct Shared {
    app: Mutex<App<Box<dyn Store>>>,
    judge: AnyJudge,
}

/// An error response: `{ "error": { "code", "message" } }` with a status for the code.
pub struct HttpError(ApiError);

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            ErrorCode::BadRequest => StatusCode::BAD_REQUEST,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::Conflict => StatusCode::CONFLICT,
            // A store failure is this server's fault, not the caller's, so it is a 500
            // rather than the 503 the variant name suggests. `Unavailable` is part
            // of the frozen wire enum in `todo_core`, so the name stays and only
            // the status is what we mean here.
            ErrorCode::Unavailable => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Upstream => StatusCode::BAD_GATEWAY,
        };
        #[derive(Serialize)]
        struct Body {
            error: ApiError,
        }
        (status, Json(Body { error: self.0 })).into_response()
    }
}

impl From<ApiError> for HttpError {
    fn from(error: ApiError) -> Self {
        Self(error)
    }
}

/// The zone from `X-Rambl-TZ`, else `RAMBL_DEFAULT_TZ`, else New York.
pub fn zone(headers: &HeaderMap) -> Result<Tz, HttpError> {
    match headers.get(&TZ_HEADER) {
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|name| name.parse::<Tz>().ok())
            .ok_or_else(|| {
                HttpError(ApiError {
                    code: ErrorCode::BadRequest,
                    message: "X-Rambl-TZ is not a known IANA time zone".into(),
                })
            }),
        None => Ok(std::env::var("RAMBL_DEFAULT_TZ")
            .ok()
            .and_then(|name| name.parse().ok())
            .unwrap_or(chrono_tz::America::New_York)),
    }
}

pub fn router(app: App<Box<dyn Store>>, judge: AnyJudge) -> Router {
    Router::new()
        .route("/api/v1/snapshot", get(snapshot))
        .route("/api/v1/tasks", post(create))
        .route("/api/v1/tasks/order", put(reorder))
        .route("/api/v1/import", post(import))
        .route("/api/v1/tasks/{id}", patch(update).delete(remove))
        .route("/api/v1/capture", post(capture))
        .route("/api/v1/tasks/{id}/revise", post(revise_task))
        .route("/api/v1/calendar.ics", get(calendar_ics))
        .with_state(Arc::new(Shared {
            app: Mutex::new(app),
            judge,
        }))
}

async fn snapshot(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<Json<Snapshot>, HttpError> {
    let tz = zone(&headers)?;
    let app = shared.app.lock().unwrap();
    Ok(Json(app.snapshot(tz)?))
}

async fn create(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(draft): Json<TaskDraft>,
) -> Result<Json<Snapshot>, HttpError> {
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.create(draft, tz)?))
}

async fn update(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(patch): Json<Patch>,
) -> Result<Json<Snapshot>, HttpError> {
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.patch(&id, patch, tz)?))
}

async fn remove(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Snapshot>, HttpError> {
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.delete(&id, tz)?))
}

/// `POST /api/v1/import`: a multipart `file` holding an .ics calendar.
async fn import(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<ImportResult>, HttpError> {
    let tz = zone(&headers)?;
    let mut ics_text = None;
    while let Some(field) = multipart.next_field().await.map_err(bad_request)? {
        if field.name() == Some("file") {
            ics_text = Some(field.text().await.map_err(bad_request)?);
        }
    }
    let ics_text =
        ics_text.ok_or_else(|| bad_request("send the calendar as multipart field `file`"))?;
    let events = crate::ics::parse_events(&ics_text)
        .map_err(|error| bad_request(error.0))?
        .into_iter()
        .map(|(uid, row)| ImportedEvent {
            uid,
            title: row.title,
            start: row.start,
            end: row.end,
        })
        .collect();
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.import(events, tz)?))
}

fn bad_request(message: impl std::fmt::Display) -> HttpError {
    HttpError(ApiError {
        code: ErrorCode::BadRequest,
        message: message.to_string(),
    })
}

#[derive(Deserialize)]
struct OrderIn {
    ids: Vec<String>,
}

async fn reorder(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<OrderIn>,
) -> Result<Json<Snapshot>, HttpError> {
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.reorder(&body.ids, tz)?))
}

#[derive(Deserialize)]
struct CaptureIn {
    text: String,
}

#[derive(Serialize)]
struct CaptureOut {
    created: Vec<TaskId>,
    snapshot: todo_core::Snapshot,
}

/// `POST /api/v1/capture`: JSON `{text}`, or multipart `file` (.pdf, .eml or text).
async fn capture(
    State(shared): State<Arc<Shared>>,
    request: axum::extract::Request,
) -> Result<Json<CaptureOut>, HttpError> {
    let tz = zone(request.headers())?;
    let (text, source, anchor) = read_capture(request).await?;
    let now = anchor.unwrap_or_else(|| shared.app.lock().unwrap().now());
    let ctx = LocalContext { now, tz };
    let drafts = capture_text(&shared.judge, &text, source, &ctx)
        .await
        .map_err(upstream)?;
    let mut app = shared.app.lock().unwrap();
    let (created, snapshot) = app.add_drafts(drafts, tz)?;
    Ok(Json(CaptureOut { created, snapshot }))
}

/// The capture text, its source, and an anchor time when the source has a send date.
async fn read_capture(
    request: axum::extract::Request,
) -> Result<(String, Source, Option<DateTime<Utc>>), HttpError> {
    let multipart = request
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("multipart/form-data"));
    if multipart {
        let mut multipart = Multipart::from_request(request, &())
            .await
            .map_err(bad_request)?;
        while let Some(field) = multipart.next_field().await.map_err(bad_request)? {
            if field.name() == Some("file") {
                let name = field.file_name().unwrap_or_default().to_ascii_lowercase();
                let bytes = field.bytes().await.map_err(bad_request)?;
                return file_text(&name, &bytes);
            }
        }
        return Err(bad_request("send the document as multipart field `file`"));
    }
    let Json(body) = Json::<CaptureIn>::from_request(request, &())
        .await
        .map_err(bad_request)?;
    if body.text.trim().is_empty() {
        return Err(bad_request("text is required"));
    }
    Ok((body.text, Source::Text, None))
}

fn file_text(
    name: &str,
    bytes: &[u8],
) -> Result<(String, Source, Option<DateTime<Utc>>), HttpError> {
    if name.ends_with(".pdf") {
        let text = capture::pdf_to_text(bytes).map_err(|capture::PdfError::Unreadable(why)| {
            bad_request(format!("could not read that PDF: {why}"))
        })?;
        return Ok((text, Source::Pdf, None));
    }
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_| bad_request("that file is not UTF-8 text"))?;
    if name.ends_with(".eml") {
        let email = capture::parse_email(&text);
        let body = format!("{}\n{}", email.subject, email.body);
        return Ok((body, Source::Email, email.sent));
    }
    Ok((text, Source::Text, None))
}

#[derive(Deserialize)]
struct ReviseIn {
    instruction: String,
}

/// `POST /api/v1/tasks/{id}/revise`: one sentence re-classifies the task.
async fn revise_task(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<ReviseIn>,
) -> Result<Json<todo_core::Snapshot>, HttpError> {
    let tz = zone(&headers)?;
    let instruction = body.instruction.trim().to_owned();
    if instruction.is_empty() {
        return Err(bad_request("instruction is required"));
    }
    let (title, now) = {
        let app = shared.app.lock().unwrap();
        (app.title_of(&id)?, app.now())
    };
    let ctx = LocalContext { now, tz };
    let revision: Revision = capture::revise(&shared.judge, &title, &instruction, &ctx)
        .await
        .map_err(upstream)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.apply_revision(&id, &revision, tz)?))
}

/// `GET /api/v1/calendar.ics`: the plan as an iCalendar file.
async fn calendar_ics(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<([(HeaderName, &'static str); 1], String), HttpError> {
    let tz = zone(&headers)?;
    let snapshot = shared.app.lock().unwrap().snapshot(tz)?;
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/calendar; charset=utf-8",
        )],
        build_ics(&snapshot),
    ))
}

fn upstream(error: anyhow::Error) -> HttpError {
    HttpError(ApiError {
        code: ErrorCode::Upstream,
        message: format!("judgment failed: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::store::MemoryStore;
    use axum::body::Body;
    use axum::http::Request;
    use chrono::{TimeZone, Utc};
    use http_body_util::BodyExt;
    use todo_core::{FixedClock, Source, When};
    use tower::ServiceExt;

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
        }
        for task in snapshot.tasks.iter().filter(|task| !task.done) {
            if let When::At(start) = task.when {
                let shown: Vec<_> = snapshot
                    .plan
                    .placements
                    .iter()
                    .filter(|placement| placement.task == task.id)
                    .collect();
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
        assert_eq!(
            serde_json::from_value::<Snapshot>(body).unwrap().tasks[0].title,
            "Second"
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

    const SYNTHETIC_ICS: &str = include_str!("../../tests/fixtures/synthetic.ics");

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
    }
}

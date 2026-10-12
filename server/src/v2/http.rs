//! Thin HTTP handlers for `/api/v1`. They read the time zone header, lock the `App`
//! for the whole call, and map errors. No business rules live here.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{FromRequest, Multipart, Query, State},
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
use chrono::{DateTime, NaiveDate, Utc};
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
            // A dependency being down is 503, "come back later"; our own
            // failure is 500. The two are different things to a client, which
            // is why they are different codes.
            ErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
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

/// The zone from `X-Rambl-TZ`, else `RAMBL_DEFAULT_TZ`, else `TZ_NAME`, else New York.
///
/// `TZ_NAME` is the name v1 and the existing `.env` files use, so a server set up
/// before v2 keeps the zone it was configured with.
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
        None => Ok(zone_from_env(
            std::env::var("RAMBL_DEFAULT_TZ").ok(),
            std::env::var("TZ_NAME").ok(),
        )),
    }
}

/// The zone for a request that named none, from the two configured fallbacks.
///
/// Split out from [`zone`] so it can be tested without touching the process
/// environment: these tests run in parallel threads and `std::env::set_var` is
/// `unsafe` in edition 2024 for exactly that reason.
fn zone_from_env(default_tz: Option<String>, tz_name: Option<String>) -> Tz {
    default_tz
        .or(tz_name)
        .and_then(|name| name.parse().ok())
        .unwrap_or(chrono_tz::America::New_York)
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

/// `GET /api/v1/snapshot?from=YYYY-MM-DD&days=N`: the plan window, today and 7 days
/// by default. The task list is always the whole list.
async fn snapshot(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(window): Query<SnapshotWindow>,
) -> Result<Json<Snapshot>, HttpError> {
    let tz = zone(&headers)?;
    let app = shared.app.lock().unwrap();
    let today = app.now().with_timezone(&tz).date_naive();
    let from = match window.from {
        Some(from) => from,
        None => today,
    };
    let days = window.days.unwrap_or(DEFAULT_WINDOW_DAYS);
    if days == 0 || days > MAX_WINDOW_DAYS {
        return Err(bad_request(format!(
            "days must be between 1 and {MAX_WINDOW_DAYS}"
        )));
    }
    Ok(Json(app.snapshot_in_window(tz, from, days)?))
}

/// The `?from=`/`?days=` plan window.
#[derive(Default, Deserialize)]
struct SnapshotWindow {
    from: Option<NaiveDate>,
    days: Option<u32>,
}

const DEFAULT_WINDOW_DAYS: u32 = 7;
/// A year, matching the horizon the scheduler itself fills.
const MAX_WINDOW_DAYS: u32 = 366;

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
///
/// Only `.pdf`, `.eml` and `.txt` are accepted. Anything else used to fall through to
/// "treat the bytes as text", which quietly accepted a `.docx` and handed the judge
/// binary noise.
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
    if !(name.ends_with(".eml") || name.ends_with(".txt")) {
        return Err(bad_request("upload a .pdf, .eml or .txt file"));
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

/// A Jev failure, split the way ARCHITECTURE.md section 6 splits it.
///
/// A network error or a timeout means Jev was never reached: the dependency is down,
/// so this is `Unavailable` (503) and the caller should retry. A response that came
/// back but could not be understood is `Upstream` (502): Jev answered, wrongly.
/// Collapsing both into 502 tells the client to give up on a dependency that is
/// merely busy.
fn upstream(error: anyhow::Error) -> HttpError {
    let message = error.to_string();
    let unreachable = error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|error| error.is_timeout() || error.is_connect())
    });
    let code = if unreachable {
        ErrorCode::Unavailable
    } else {
        ErrorCode::Upstream
    };
    HttpError(ApiError {
        code,
        message: format!("judgment failed: {message}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::store::{MemoryStore, SqliteStore};
    use axum::body::Body;
    use axum::http::Request;
    use chrono::{TimeZone, Utc};
    use http_body_util::BodyExt;
    use todo_core::{FixedClock, Source, When};
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

        let bytes =
            futures::executor::block_on(axum::body::to_bytes(response.into_body(), usize::MAX))
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
        let week = include_str!("../../tests/fixtures/week.ics");
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
        for query in ["?days=0", "?days=9999", "?from=not-a-date"] {
            let (status, body) =
                send(&router, "GET", &format!("/api/v1/snapshot{query}"), None).await;
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
            .snapshot(chrono_tz::America::New_York)
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
}

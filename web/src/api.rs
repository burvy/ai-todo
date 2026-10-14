use chrono::{DateTime, Utc, Weekday};
use gloo_net::http::{Request, RequestBuilder, Response};
#[cfg(target_arch = "wasm32")]
use js_sys;
use serde::{Deserialize, Serialize};
use std::fmt;
use todo_core::tuning::DEFAULT_WINDOW_DAYS;
use todo_core::{ApiError as CoreApiError, ErrorCode, Snapshot, TaskId, When};
use uuid::Uuid;

/// Keeps the anonymous id the server issued, and ends the signed-in session when the
/// server refuses its token.
fn check_response(response: &Response) {
    if response.status() == 401 {
        crate::account::session_ended();
    }
    #[cfg(target_arch = "wasm32")]
    if let Some(id) = response.headers().get("x-rambl-anon")
        && let Some(window) = web_sys::window()
        && let Ok(Some(storage)) = window.local_storage()
    {
        let _ = storage.set_item("rambl_anon_id", &id);
    }
}

/// Every request says who it is: the time zone, the anonymous id, and the session token
/// when someone is signed in.
trait Identity: Sized {
    fn identity(self) -> Self;
}

impl Identity for RequestBuilder {
    fn identity(self) -> Self {
        let request = self
            .header("X-Rambl-TZ", &timezone())
            .header("X-Rambl-Anon", &anon_id());
        match crate::account::token() {
            Some(token) => request.header("Authorization", &format!("Bearer {token}")),
            None => request,
        }
    }
}

/// Sends a request and checks the answer; a request with a body is sent the same way.
trait SendChecked: Sized {
    async fn send_checked(self) -> Result<Response, ApiError>;
}

impl SendChecked for RequestBuilder {
    async fn send_checked(self) -> Result<Response, ApiError> {
        let response = self
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        accepted(response).await
    }
}

impl SendChecked for Request {
    async fn send_checked(self) -> Result<Response, ApiError> {
        let response = self
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        accepted(response).await
    }
}

/// The response when the server accepted the request, else its refusal. A refusal must
/// not reach the caller's `.json::<Snapshot>()`: that would turn "this task changed
/// elsewhere" into a decode error the user cannot act on.
async fn accepted(response: Response) -> Result<Response, ApiError> {
    check_response(&response);
    if response.ok() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(refusal(status, &body))
}

/// A refused request as the server explained it: `{ "error": { "code", "message" } }`.
/// A body without that shape (a proxy's error page, an empty 502) is named from its
/// status, so the caller can still tell a stale edit from a server that is down.
fn refusal(status: u16, body: &str) -> ApiError {
    #[derive(Deserialize)]
    struct Refusal {
        error: CoreApiError,
    }
    if let Ok(Refusal { error }) = serde_json::from_str(body) {
        return ApiError::Server(error);
    }
    let code = match status {
        400 => ErrorCode::BadRequest,
        401 => ErrorCode::Unauthorized,
        403 => ErrorCode::Forbidden,
        404 => ErrorCode::NotFound,
        409 => ErrorCode::Conflict,
        502 | 504 => ErrorCode::Upstream,
        503 => ErrorCode::Unavailable,
        _ => ErrorCode::Internal,
    };
    ApiError::Server(CoreApiError {
        code,
        message: format!("The server answered {status}."),
    })
}

/// Get the anonymous user ID from localStorage, or generate a new one if not present.
pub fn anon_id() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        // Storage can be blocked (private windows, some embedded browsers). The page
        // still works then, but each request gets a new id, so tasks do not persist.
        const KEY: &str = "rambl_anon_id";
        let storage = web_sys::window().and_then(|window| window.local_storage().ok().flatten());
        if let Some(id) = storage
            .as_ref()
            .and_then(|storage| storage.get_item(KEY).ok().flatten())
        {
            return id;
        }
        let id = format!("anon_{}", Uuid::new_v4().simple());
        if let Some(storage) = storage {
            let _ = storage.set_item(KEY, &id);
        }
        id
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        format!("anon_{}", Uuid::new_v4().simple())
    }
}

pub fn timezone() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let options =
            js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &js_sys::Object::new())
                .resolved_options();
        js_sys::Reflect::get(&options, &"timeZone".into())
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_else(|| "UTC".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        "UTC".to_string()
    }
}

/// Load the fixture snapshot for initial state
pub fn fixture_snapshot() -> Snapshot {
    serde_json::from_str(include_str!("../fixtures/snapshot.json"))
        .expect("fixture snapshot must be valid")
}

/// Today's date on this device, as YYYY-MM-DD.
///
/// The browser's clock, not chrono's: chrono's `Local::now` panics on wasm32, because it
/// asks the platform clock, which the browser build does not have.
pub fn today_date() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let now = js_sys::Date::new_0();
        format!(
            "{:04}-{:02}-{:02}",
            now.get_full_year(),
            now.get_month() + 1,
            now.get_date()
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    }
}

/// API error that wraps both server errors and network/decoding failures.
#[derive(Debug)]
pub enum ApiError {
    /// Server returned an error response: { "error": { "code", "message" } }.
    Server(CoreApiError),
    /// Network or request failed.
    Network(String),
    /// JSON decoding failed.
    Decode(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Server(e) => write!(f, "{:?}: {}", e.code, e.message),
            ApiError::Network(e) => write!(f, "network error: {}", e),
            ApiError::Decode(e) => write!(f, "decode error: {}", e),
        }
    }
}

impl std::error::Error for ApiError {}

impl From<gloo_net::Error> for ApiError {
    fn from(e: gloo_net::Error) -> Self {
        ApiError::Network(e.to_string())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError::Decode(e.to_string())
    }
}

/// Response for capture: { created: [id], snapshot }.
#[derive(Clone, Debug, Deserialize)]
pub struct CaptureResponse {
    pub created: Vec<TaskId>,
    pub snapshot: Snapshot,
}

/// Response for import: { created: [id], updated: [id], snapshot }.
#[derive(Clone, Debug, Deserialize)]
pub struct ImportResponse {
    pub created: Vec<TaskId>,
    pub updated: Vec<TaskId>,
    pub snapshot: Snapshot,
}

/// Body for capturing text.
#[derive(Clone, Debug, Serialize)]
pub struct CaptureTextBody {
    pub text: String,
}

/// Body for reordering tasks.
#[derive(Clone, Debug, Serialize)]
pub struct ReorderBody {
    pub ids: Vec<TaskId>,
}

/// Patch body for updating a task - allows partial updates.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PatchTaskBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_min: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<When>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<todo_core::Label>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<todo_core::Priority>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<todo_core::Source>,
    /// The task's `updated_at` as this page last saw it; the server refuses the edit if
    /// the task has changed since.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

/// Body for revising a task with one spoken or typed sentence.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReviseBody {
    pub instruction: String,
    /// The task's `updated_at` as this page last saw it, as for [`PatchTaskBody`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

impl PatchTaskBody {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn updated_at(mut self, updated_at: DateTime<Utc>) -> Self {
        self.updated_at = Some(updated_at);
        self
    }

    pub fn title(mut self, title: String) -> Self {
        self.title = Some(title);
        self
    }

    pub fn duration_min(mut self, duration_min: u16) -> Self {
        self.duration_min = Some(duration_min);
        self
    }

    pub fn when(mut self, when: When) -> Self {
        self.when = Some(when);
        self
    }

    pub fn done(mut self, done: bool) -> Self {
        self.done = Some(done);
        self
    }

    pub fn priority(mut self, priority: todo_core::Priority) -> Self {
        self.priority = Some(priority);
        self
    }
}

/// The snapshot URL for the default window starting at `from`. Every fetch uses it, so a
/// snapshot never depends on which call ran last.
fn snapshot_url(from: &str) -> String {
    format!("/api/v1/snapshot?from={from}&days={DEFAULT_WINDOW_DAYS}")
}

/// Body for `PATCH /api/v1/settings`: only the fields sent change, each `HH:MM`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DaySettingsBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placement: Option<String>,
}

/// GET /api/v1/settings: when "by Friday" is due, and where a task with no time goes.
pub async fn fetch_day_settings() -> Result<todo_core::DaySettings, ApiError> {
    Request::get("/api/v1/settings")
        .identity()
        .send_checked()
        .await?
        .json::<todo_core::DaySettings>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// PATCH /api/v1/settings: answers with both times as they now stand.
pub async fn patch_day_settings(
    body: &DaySettingsBody,
) -> Result<todo_core::DaySettings, ApiError> {
    Request::patch("/api/v1/settings")
        .identity()
        .header("Content-Type", "application/json")
        .json(body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send_checked()
        .await?
        .json::<todo_core::DaySettings>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// GET /api/v1/snapshot for the default window from `from`.
pub async fn fetch_snapshot(from: &str) -> Result<Snapshot, ApiError> {
    let response = Request::get(&snapshot_url(from))
        .identity()
        .send_checked()
        .await?;
    response
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// POST /api/v1/capture with text body
pub async fn capture_text(text: &str) -> Result<CaptureResponse, ApiError> {
    let body = CaptureTextBody {
        text: text.to_string(),
    };
    let response = Request::post("/api/v1/capture")
        .identity()
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send_checked()
        .await?;
    response
        .json::<CaptureResponse>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// PATCH /api/v1/tasks/{id} - update a task
pub async fn patch_task(id: &TaskId, body: PatchTaskBody) -> Result<Snapshot, ApiError> {
    let url = format!("/api/v1/tasks/{}", id.0);
    Request::patch(&url)
        .identity()
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send_checked()
        .await?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// POST /api/v1/tasks/{id}/revise - Jev applies one sentence ("make it Friday at 3")
pub async fn revise_task(id: &TaskId, body: ReviseBody) -> Result<Snapshot, ApiError> {
    let url = format!("/api/v1/tasks/{}/revise", id.0);
    Request::post(&url)
        .identity()
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send_checked()
        .await?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// PATCH /api/v1/series/{id}/dates/{date} - one occurrence's exception. Moving
/// one day of a series is an exception on the series, never a task PATCH.
pub async fn move_series_occurrence(
    series_id: &str,
    date: chrono::NaiveDate,
    to: When,
) -> Result<Snapshot, ApiError> {
    let url = format!(
        "/api/v1/series/{}/dates/{}",
        series_id,
        date.format("%Y-%m-%d")
    );
    let body = OccurrencePatchBody {
        kind: "moved".to_owned(),
        when: Some(to),
    };
    Request::patch(&url)
        .identity()
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send_checked()
        .await?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// The body `PATCH /api/v1/series/{id}/dates/{date}` takes.
#[derive(Serialize)]
struct OccurrencePatchBody {
    kind: String,
    when: Option<When>,
}

/// POST /api/v1/tasks/restore - tape a just-deleted task back as it was
pub async fn restore_task(task: &todo_core::Task) -> Result<Snapshot, ApiError> {
    Request::post("/api/v1/tasks/restore")
        .identity()
        .header("Content-Type", "application/json")
        .json(task)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send_checked()
        .await?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// PUT /api/v1/tasks/order - reorder all tasks
pub async fn reorder_tasks(ids: Vec<TaskId>) -> Result<Snapshot, ApiError> {
    let body = ReorderBody { ids };
    Request::put("/api/v1/tasks/order")
        .identity()
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send_checked()
        .await?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// DELETE /api/v1/tasks/{id} - delete a task
pub async fn delete_task(id: &TaskId) -> Result<Snapshot, ApiError> {
    let url = format!("/api/v1/tasks/{}", id.0);
    Request::delete(&url)
        .identity()
        .send_checked()
        .await?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// POST /api/v1/import - import .ics file
pub async fn import_ics(file: web_sys::File) -> Result<ImportResponse, ApiError> {
    let form_data = web_sys::FormData::new().unwrap();
    let file_name = file.name();
    let name = if file_name.is_empty() {
        "filename.ics"
    } else {
        &file_name
    };
    form_data
        .append_with_blob_and_filename("file", &file.into(), name)
        .map_err(|e| {
            ApiError::Network(
                e.as_string()
                    .unwrap_or_else(|| "failed to append file".into()),
            )
        })?;

    Request::post("/api/v1/import")
        .identity()
        .body(form_data)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send_checked()
        .await?
        .json::<ImportResponse>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// GET /api/v1/calendar.ics - export calendar as iCalendar
pub async fn export_calendar() -> Result<String, ApiError> {
    Request::get("/api/v1/calendar.ics")
        .identity()
        .send_checked()
        .await?
        .text()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// Body for `POST /api/v1/series` — make a thing repeat weekly
/// (docs/ARCHITECTURE.md 7.1). Dates and times travel as `YYYY-MM-DD` / `HH:MM`
/// strings, the spelling chrono's serde uses on the server.
#[derive(Clone, Debug, Serialize)]
pub struct SeriesBody {
    pub title: String,
    pub duration_min: u16,
    /// chrono weekday names, Monday-first: `["Mon", "Thu"]`.
    pub weekdays: Vec<Weekday>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_of_day: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

/// Body for `PATCH /api/v1/series/{id}`: absent fields stay as they are. The
/// time and end date are always sent, because the dialog holds the whole rule:
/// `null` clears them (docs/ARCHITECTURE.md 7.1), so an emptied field ends the
/// series' fixed time or end date.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SeriesPatchBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_min: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekdays: Option<Vec<Weekday>>,
    pub time_of_day: Option<String>,
    pub until: Option<String>,
}

/// `POST /api/v1/series`: create a weekly series. Answers with the snapshot,
/// occurrences already expanded.
pub async fn create_series(body: SeriesBody) -> Result<Snapshot, ApiError> {
    Request::post("/api/v1/series")
        .identity()
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send_checked()
        .await?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// `PATCH /api/v1/series/{id}`: edit the rule every occurrence follows.
pub async fn patch_series(id: &str, body: SeriesPatchBody) -> Result<Snapshot, ApiError> {
    let url = format!("/api/v1/series/{id}");
    Request::patch(&url)
        .identity()
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send_checked()
        .await?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// `DELETE /api/v1/series/{id}`: the whole series, its exceptions with it.
pub async fn delete_series(id: &str) -> Result<Snapshot, ApiError> {
    let url = format!("/api/v1/series/{id}");
    Request::delete(&url)
        .identity()
        .send_checked()
        .await?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// `PATCH /api/v1/series/{id}/dates/{date}` with `{"kind": ...}`: mark ONE
/// occurrence of the series `"done"` or `"skipped"` (a move needs a `when` and
/// is not offered by this page yet).
pub async fn mark_occurrence(
    series_id: &str,
    date: &str,
    kind: &str,
) -> Result<Snapshot, ApiError> {
    let url = format!("/api/v1/series/{series_id}/dates/{date}");
    Request::patch(&url)
        .identity()
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "kind": kind }))
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send_checked()
        .await?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// `DELETE /api/v1/series/{id}/dates/{date}`: clear one occurrence's
/// exception, putting that day back to the rule — how a marked day is undone.
pub async fn clear_occurrence(series_id: &str, date: &str) -> Result<Snapshot, ApiError> {
    let url = format!("/api/v1/series/{series_id}/dates/{date}");
    Request::delete(&url)
        .identity()
        .send_checked()
        .await?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_edit_is_refused_as_stale_not_as_an_undecodable_snapshot() {
        let body = r#"{"error":{"code":"stale_update","message":"that task changed"}}"#;
        match refusal(409, body) {
            ApiError::Server(error) => {
                assert_eq!(error.code, ErrorCode::StaleUpdate);
                assert_eq!(error.message, "that task changed");
            }
            other => panic!("expected the server's refusal, got {other:?}"),
        }
    }

    #[test]
    fn every_snapshot_fetch_asks_for_the_servers_default_window() {
        assert_eq!(
            snapshot_url("2026-10-14"),
            "/api/v1/snapshot?from=2026-10-14&days=7"
        );
    }

    #[test]
    fn a_refusal_without_the_error_body_is_named_from_its_status() {
        let code = |status, body| match refusal(status, body) {
            ApiError::Server(error) => error.code,
            other => panic!("expected a server refusal, got {other:?}"),
        };
        assert_eq!(code(502, "<html>Bad gateway</html>"), ErrorCode::Upstream);
        assert_eq!(code(503, ""), ErrorCode::Unavailable);
        assert_eq!(code(401, ""), ErrorCode::Unauthorized);
        assert_eq!(code(404, "not json"), ErrorCode::NotFound);
        assert_eq!(code(500, "{}"), ErrorCode::Internal);
    }
}

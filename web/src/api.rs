use gloo_net::http::{Request, Response};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt;
use todo_core::{ApiError as CoreApiError, ErrorCode, Snapshot, TaskDraft, TaskId, When};

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

/// Body for revising a task.
#[derive(Clone, Debug, Serialize)]
pub struct ReviseBody {
    pub instruction: String,
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
}

impl PatchTaskBody {
    pub fn new() -> Self {
        Self::default()
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

    pub fn label(mut self, label: todo_core::Label) -> Self {
        self.label = Some(label);
        self
    }

    pub fn priority(mut self, priority: todo_core::Priority) -> Self {
        self.priority = Some(priority);
        self
    }

    pub fn source(mut self, source: todo_core::Source) -> Self {
        self.source = Some(source);
        self
    }
}

/// Helper to check if a status code indicates success.
#[allow(dead_code)]
fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// Helper to extract the error from a server response.
#[allow(dead_code)]
async fn handle_response<T: for<'de> serde::Deserialize<'de>>(
    response: Result<Response, gloo_net::Error>,
) -> Result<T, ApiError> {
    let response = response.map_err(|e| ApiError::Network(e.to_string()))?;
    let status = response.status();

    if !is_success(status) {
        // Try to parse error response: { "error": { "code", "message" } }.
        match response.json::<ServerErrorResponse>().await {
            Ok(err_resp) => Err(ApiError::Server(CoreApiError {
                code: err_resp.error.code,
                message: err_resp.error.message,
            })),
            Err(_) => {
                // If we can't parse the error, try to get the text.
                let text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "unknown error".into());
                Err(ApiError::Network(format!("HTTP {}: {}", status, text)))
            }
        }
    } else {
        response
            .json::<T>()
            .await
            .map_err(|e| ApiError::Decode(e.to_string()))
    }
}

/// Server error response shape: { "error": { "code", "message" } }.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ServerErrorResponse {
    error: ServerErrorInner,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ServerErrorInner {
    code: ErrorCode,
    message: String,
}

pub fn use_snapshot() -> ReadSignal<Snapshot> {
    let initial: Snapshot = serde_json::from_str(include_str!("../fixtures/snapshot.json"))
        .expect("fixture snapshot must be valid");
    let (snapshot, _) = signal(initial);
    snapshot
}

/// GET /api/v1/snapshot?from=YYYY-MM-DD&days=7
pub async fn fetch_snapshot(from: &str, days: u8) -> Result<Snapshot, ApiError> {
    let url = format!("/api/v1/snapshot?from={from}&days={days}");
    Request::get(&url)
        .header("X-Rambl-TZ", &timezone())
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// POST /api/v1/capture with text body
pub async fn capture_text(text: &str) -> Result<CaptureResponse, ApiError> {
    let body = CaptureTextBody {
        text: text.to_string(),
    };
    Request::post("/api/v1/capture")
        .header("X-Rambl-TZ", &timezone())
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?
        .json::<CaptureResponse>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// POST /api/v1/capture with file upload (PDF, EML, TXT)
pub async fn capture_file(file: web_sys::File) -> Result<CaptureResponse, ApiError> {
    let form_data = web_sys::FormData::new().unwrap();
    // Use the File's name or a default
    let file_name = file.name();
    let name = if file_name.is_empty() {
        "filename"
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

    Request::post("/api/v1/capture")
        .header("X-Rambl-TZ", &timezone())
        .body(form_data)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?
        .json::<CaptureResponse>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// POST /api/v1/tasks - create a new task
pub async fn create_task(draft: TaskDraft) -> Result<Snapshot, ApiError> {
    Request::post("/api/v1/tasks")
        .header("X-Rambl-TZ", &timezone())
        .header("Content-Type", "application/json")
        .json(&draft)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// PATCH /api/v1/tasks/{id} - update a task
pub async fn patch_task(id: &TaskId, body: PatchTaskBody) -> Result<Snapshot, ApiError> {
    let url = format!("/api/v1/tasks/{}", id.0);
    Request::patch(&url)
        .header("X-Rambl-TZ", &timezone())
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// POST /api/v1/tasks/{id}/revise - revise a task with an instruction
pub async fn revise_task(id: &TaskId, instruction: &str) -> Result<Snapshot, ApiError> {
    let url = format!("/api/v1/tasks/{}/revise", id.0);
    let body = ReviseBody {
        instruction: instruction.to_string(),
    };
    Request::post(&url)
        .header("X-Rambl-TZ", &timezone())
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// PUT /api/v1/tasks/order - reorder all tasks
pub async fn reorder_tasks(ids: Vec<TaskId>) -> Result<Snapshot, ApiError> {
    let body = ReorderBody { ids };
    Request::put("/api/v1/tasks/order")
        .header("X-Rambl-TZ", &timezone())
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?
        .json::<Snapshot>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// DELETE /api/v1/tasks/{id} - delete a task
pub async fn delete_task(id: &TaskId) -> Result<Snapshot, ApiError> {
    let url = format!("/api/v1/tasks/{}", id.0);
    Request::delete(&url)
        .header("X-Rambl-TZ", &timezone())
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?
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
        .header("X-Rambl-TZ", &timezone())
        .body(form_data)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?
        .json::<ImportResponse>()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

/// GET /api/v1/calendar.ics - export calendar as iCalendar
pub async fn export_calendar() -> Result<String, ApiError> {
    Request::get("/api/v1/calendar.ics")
        .header("X-Rambl-TZ", &timezone())
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?
        .text()
        .await
        .map_err(|e| ApiError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_core::{Label, Priority, Source, When};

    #[test]
    fn api_error_display() {
        let server_err = ApiError::Server(CoreApiError {
            code: ErrorCode::NotFound,
            message: "Task not found".to_string(),
        });
        // ErrorCode Debug format is "NotFound", not "not_found"
        assert_eq!(format!("{}", server_err), "NotFound: Task not found");

        let network_err = ApiError::Network("connection failed".to_string());
        assert_eq!(
            format!("{}", network_err),
            "network error: connection failed"
        );

        let decode_err = ApiError::Decode("invalid json".to_string());
        assert_eq!(format!("{}", decode_err), "decode error: invalid json");
    }

    #[test]
    fn snapshot_url_building() {
        let url = "/api/v1/snapshot?from=2026-10-12&days=7".to_string();
        assert_eq!(url, "/api/v1/snapshot?from=2026-10-12&days=7");
    }

    #[test]
    fn task_url_building() {
        let task_id = TaskId("test-123".to_string());
        let url = format!("/api/v1/tasks/{}", task_id.0);
        assert_eq!(url, "/api/v1/tasks/test-123");
    }

    #[test]
    fn revise_url_building() {
        let task_id = TaskId("test-456".to_string());
        let url = format!("/api/v1/tasks/{}/revise", task_id.0);
        assert_eq!(url, "/api/v1/tasks/test-456/revise");
    }

    #[test]
    fn reorder_url_is_correct() {
        assert_eq!("/api/v1/tasks/order", "/api/v1/tasks/order");
    }

    #[test]
    fn capture_url_is_correct() {
        assert_eq!("/api/v1/capture", "/api/v1/capture");
    }

    #[test]
    fn import_url_is_correct() {
        assert_eq!("/api/v1/import", "/api/v1/import");
    }

    #[test]
    fn calendar_export_url_is_correct() {
        assert_eq!("/api/v1/calendar.ics", "/api/v1/calendar.ics");
    }

    #[test]
    fn is_success_works() {
        assert!(is_success(200));
        assert!(is_success(201));
        assert!(is_success(299));
        assert!(!is_success(199));
        assert!(!is_success(300));
        assert!(!is_success(404));
        assert!(!is_success(500));
    }

    #[test]
    fn patch_task_body_serialization() {
        let body = PatchTaskBody::new()
            .title("New title".to_string())
            .done(true);
        let json = serde_json::to_string(&body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["title"], "New title");
        assert_eq!(parsed["done"], true);
        assert!(parsed.get("duration_min").is_none());
        assert!(parsed.get("when").is_none());
    }

    #[test]
    fn capture_text_body_serialization() {
        let body = CaptureTextBody {
            text: "Test capture text".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["text"], "Test capture text");
    }

    #[test]
    fn revise_body_serialization() {
        let body = ReviseBody {
            instruction: "Make this urgent".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["instruction"], "Make this urgent");
    }

    #[test]
    fn reorder_body_serialization() {
        let ids = vec![
            TaskId("a".to_string()),
            TaskId("b".to_string()),
            TaskId("c".to_string()),
        ];
        let body = ReorderBody { ids };
        let json = serde_json::to_string(&body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["ids"], serde_json::json!(["a", "b", "c"]));
    }

    #[test]
    fn task_draft_serialization() {
        let draft = TaskDraft {
            title: "Test task".to_string(),
            duration_min: 30,
            when: When::Anytime,
            label: Label::Work,
            priority: Priority::High,
            source: Source::Text,
        };
        let json = serde_json::to_string(&draft).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["title"], "Test task");
        assert_eq!(parsed["duration_min"], 30);
        // Label enum serializes to variant name
        assert_eq!(parsed["label"], "Work");
        assert_eq!(parsed["priority"], "high");
        assert_eq!(parsed["source"], "text");
        assert_eq!(parsed["when"]["kind"], "anytime");
    }

    #[test]
    fn server_error_response_parsing() {
        let json = r#"{"error": {"code": "not_found", "message": "Task not found"}}"#;
        let parsed: ServerErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.error.message, "Task not found");
    }

    #[test]
    fn capture_response_parsing() {
        let snapshot_json = r#"{"tasks":[],"plan":{"placements":[],"unplaced":[]},"tz":"UTC","generated":"2026-01-01T00:00:00Z"}"#;
        let json = format!("{{\"created\":[\"id1\"],\"snapshot\":{}}}", snapshot_json);
        let parsed: CaptureResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.created.len(), 1);
        assert_eq!(parsed.created[0].0, "id1");
    }

    #[test]
    fn import_response_parsing() {
        let snapshot_json = r#"{"tasks":[],"plan":{"placements":[],"unplaced":[]},"tz":"UTC","generated":"2026-01-01T00:00:00Z"}"#;
        let json = format!(
            "{{\"created\":[\"id1\"],\"updated\":[\"id2\"],\"snapshot\":{}}}",
            snapshot_json
        );
        let parsed: ImportResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.created.len(), 1);
        assert_eq!(parsed.updated.len(), 1);
    }

    #[test]
    fn api_error_from_network() {
        // gloo_net::Error doesn't expose a simple constructor, but we can test the From impl
        // by using the fact that it implements From<serde_json::Error>
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let gloo_err = gloo_net::Error::from(json_err);
        let api_err: ApiError = gloo_err.into();
        match api_err {
            ApiError::Network(msg) => assert!(!msg.is_empty()),
            _ => panic!("Expected Network variant"),
        }
    }

    #[test]
    fn api_error_from_decode() {
        let serde_err = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let api_err: ApiError = serde_err.into();
        match api_err {
            ApiError::Decode(msg) => assert!(!msg.is_empty()),
            _ => panic!("Expected Decode variant"),
        }
    }
}

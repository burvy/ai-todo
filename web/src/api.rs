use gloo_net::http::Request;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt;
use todo_core::{ApiError as CoreApiError, Snapshot, TaskDraft, TaskId, When};

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

/// Fetch today's date in YYYY-MM-DD format
pub fn today_date() -> String {
    use chrono::Local;
    let now = Local::now();
    now.format("%Y-%m-%d").to_string()
}

pub fn use_snapshot() -> ReadSignal<Snapshot> {
    let initial = fixture_snapshot();
    let (snapshot, _) = signal(initial);
    snapshot
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

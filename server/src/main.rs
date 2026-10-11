//! HTTP server for the ai-todo API. Storage is in memory for now (no SQLite).
//! Routes and payloads follow the frozen API contract in `hive/board.md`.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Path as UrlPath, Query, State},
    http::StatusCode,
    routing::{get, patch, post},
};
use chrono::{DateTime, NaiveTime, Utc};
use scheduler::{BusyBlock, Placement, UnschedulableReason, WorkingHours};
use serde::{Deserialize, Serialize};
use todo_core::{Source, Task};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

#[derive(Default)]
struct Store {
    tasks: Vec<Task>,
    next_id: u64,
    placements: Vec<PlacementOut>,
}

#[derive(Clone)]
struct AppState {
    store: Arc<Mutex<Store>>,
    busy: Arc<Vec<BusyOut>>,
}

#[derive(Clone, Serialize)]
struct BusyOut {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    title: String,
}

#[derive(Clone, Serialize)]
struct PlacementOut {
    task_id: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

#[derive(Serialize)]
struct UnscheduledOut {
    task_id: String,
    reason: String,
}

#[derive(Serialize)]
struct TasksOut {
    tasks: Vec<Task>,
}

#[derive(Deserialize)]
struct CaptureIn {
    text: String,
}

#[derive(Serialize)]
struct CaptureOut {
    tasks: Vec<Task>,
}

#[derive(Deserialize, Default)]
struct PatchIn {
    title: Option<String>,
    notes: Option<String>,
    duration_min: Option<i64>,
    deadline: Option<Option<DateTime<Utc>>>,
    priority: Option<u8>,
    status: Option<String>,
    confidence: Option<f64>,
}

#[derive(Serialize)]
struct ScheduleOut {
    placements: Vec<PlacementOut>,
    unscheduled: Vec<UnscheduledOut>,
}

#[derive(Deserialize)]
struct RangeQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct CalendarOut {
    busy: Vec<BusyOut>,
    placements: Vec<PlacementOut>,
}

/// Turn each non-empty line into one task. Stub: the judge crate replaces this later.
fn tasks_from_text(text: &str) -> Vec<Task> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| Task {
            id: String::new(),
            title: line.to_string(),
            notes: String::new(),
            duration_min: 30,
            deadline: None,
            priority: 0,
            status: "todo".into(),
            confidence: 0.5,
            source: Source::Text,
        })
        .collect()
}

/// Busy blocks for today (UTC), seeded so the demo has something to schedule around.
fn seed_busy(today: DateTime<Utc>) -> Vec<BusyOut> {
    let at = |h: u32, m: u32| {
        today
            .date_naive()
            .and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap())
            .and_utc()
    };
    vec![
        BusyOut {
            start: at(9, 0),
            end: at(10, 0),
            title: "Standup".into(),
        },
        BusyOut {
            start: at(12, 0),
            end: at(13, 0),
            title: "Lunch".into(),
        },
        BusyOut {
            start: at(15, 0),
            end: at(15, 30),
            title: "1:1".into(),
        },
    ]
}

fn reason_text(reason: &UnschedulableReason) -> &'static str {
    match reason {
        UnschedulableReason::InvalidDuration => "invalid_duration",
        UnschedulableReason::DeadlinePassed => "deadline_passed",
        UnschedulableReason::NoAvailableTime => "no_available_time",
    }
}

async fn capture(State(state): State<AppState>, Json(body): Json<CaptureIn>) -> Json<CaptureOut> {
    // TODO: multipart `file` (PDF) capture. Needs a PDF-to-text step in crates/capture.
    // TODO: swap tasks_from_text for the judge crate (Jev) once it lands.
    let mut store = state.store.lock().unwrap();
    let mut created = tasks_from_text(&body.text);
    for task in &mut created {
        store.next_id += 1;
        task.id = format!("t{}", store.next_id);
    }
    store.tasks.extend(created.iter().cloned());
    Json(CaptureOut { tasks: created })
}

async fn list_tasks(State(state): State<AppState>) -> Json<TasksOut> {
    let store = state.store.lock().unwrap();
    Json(TasksOut {
        tasks: store.tasks.clone(),
    })
}

async fn patch_task(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<PatchIn>,
) -> Result<Json<Task>, StatusCode> {
    let mut store = state.store.lock().unwrap();
    let task = store
        .tasks
        .iter_mut()
        .find(|task| task.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    if let Some(v) = body.title {
        task.title = v;
    }
    if let Some(v) = body.notes {
        task.notes = v;
    }
    if let Some(v) = body.duration_min {
        task.duration_min = v;
    }
    if let Some(v) = body.deadline {
        task.deadline = v;
    }
    if let Some(v) = body.priority {
        task.priority = v;
    }
    if let Some(v) = body.status {
        task.status = v;
    }
    if let Some(v) = body.confidence {
        task.confidence = v;
    }
    Ok(Json(task.clone()))
}

async fn delete_task(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> StatusCode {
    let mut store = state.store.lock().unwrap();
    let before = store.tasks.len();
    store.tasks.retain(|task| task.id != id);
    if store.tasks.len() == before {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::NO_CONTENT
    }
}

async fn schedule_tasks(State(state): State<AppState>) -> Json<ScheduleOut> {
    let mut store = state.store.lock().unwrap();
    let inputs: Vec<scheduler::Task> = store
        .tasks
        .iter()
        .map(|task| scheduler::Task {
            id: task.id.clone(),
            title: task.title.clone(),
            duration_minutes: task.duration_min,
            deadline: task.deadline,
            priority: i32::from(task.priority),
        })
        .collect();
    let busy: Vec<BusyBlock> = state
        .busy
        .iter()
        .map(|block| BusyBlock {
            start: block.start,
            end: block.end,
        })
        .collect();
    let hours = WorkingHours::every_day(
        NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
    );

    let mut placements = Vec::new();
    let mut unscheduled = Vec::new();
    for placement in scheduler::schedule(&inputs, &busy, &hours, Utc::now()) {
        match placement {
            Placement::Scheduled(p) => placements.push(PlacementOut {
                task_id: p.task_id,
                start: p.start,
                end: p.end,
            }),
            Placement::Unschedulable { task_id, reason } => unscheduled.push(UnscheduledOut {
                task_id,
                reason: reason_text(&reason).into(),
            }),
        }
    }
    store.placements = placements.clone();
    Json(ScheduleOut {
        placements,
        unscheduled,
    })
}

async fn calendar(
    State(state): State<AppState>,
    Query(range): Query<RangeQuery>,
) -> Json<CalendarOut> {
    let store = state.store.lock().unwrap();
    let in_range = |start: DateTime<Utc>, end: DateTime<Utc>| {
        range.from.is_none_or(|from| end > from) && range.to.is_none_or(|to| start < to)
    };
    let busy = state
        .busy
        .iter()
        .filter(|block| in_range(block.start, block.end))
        .cloned()
        .collect();
    let placements = store
        .placements
        .iter()
        .filter(|p| in_range(p.start, p.end))
        .cloned()
        .collect();
    Json(CalendarOut { busy, placements })
}

fn app(state: AppState) -> Router {
    let mut router = Router::new()
        .route("/api/capture", post(capture))
        .route("/api/tasks", get(list_tasks))
        .route("/api/tasks/{id}", patch(patch_task).delete(delete_task))
        .route("/api/schedule", post(schedule_tasks))
        .route("/api/calendar", get(calendar))
        .with_state(state)
        .layer(CorsLayer::permissive());
    if Path::new("static").is_dir() {
        router = router.fallback_service(ServeDir::new("static"));
    }
    router
}

fn new_state() -> AppState {
    AppState {
        store: Arc::new(Mutex::new(Store::default())),
        busy: Arc::new(seed_busy(Utc::now())),
    }
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind port");
    println!("listening on http://{addr}");
    axum::serve(listener, app(new_state()))
        .await
        .expect("serve");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[test]
    fn capture_makes_one_task_per_non_empty_line() {
        let tasks = tasks_from_text("buy milk\n\n  call mom  \n");
        let titles: Vec<&str> = tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["buy milk", "call mom"]);
    }

    #[tokio::test]
    async fn capture_then_schedule_then_calendar() {
        let router = app(new_state());

        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/capture")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"write report"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/schedule")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["placements"].as_array().unwrap().len(), 1);

        let res = router
            .oneshot(
                Request::builder()
                    .uri("/api/calendar")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["busy"].as_array().unwrap().len(), 3);
        assert_eq!(json["placements"].as_array().unwrap().len(), 1);
    }
}

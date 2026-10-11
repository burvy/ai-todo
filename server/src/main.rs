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
use chrono::{DateTime, Datelike, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use futures::future::join_all;
use judge::{find_date_candidates, split_sentences, understand};
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
    #[serde(default)]
    source: Option<Source>,
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

pub fn clean_title(sentence: &str) -> String {
    let mut title = sentence
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .trim()
        .to_owned();
    loop {
        // Normalize punctuation on a leading hedge token ("um,", "so...", etc.).
        if let Some(end) = title.find(char::is_whitespace) {
            let token = &title[..end];
            let cleaned = token.trim_matches(|character: char| {
                matches!(character, ',' | '.' | '…' | '!' | ';' | ':')
            });
            if cleaned != token {
                title = format!("{}{}", cleaned, &title[end..]);
            }
        }
        let lower = title.to_ascii_lowercase();
        let filler = [
            "ugh ",
            "okay ",
            "ok ",
            "so ",
            "um ",
            "like ",
            "i need to ",
            "i should ",
            "i have to ",
            "gotta ",
            "also ",
            "and ",
            "please ",
            "todo: ",
        ]
        .iter()
        .find(|prefix| lower.starts_with(**prefix));
        let Some(prefix) = filler else { break };
        title = title[prefix.len()..].trim().to_owned();
    }
    let lower = title.to_ascii_lowercase();
    let trailing = [
        " tomorrow",
        " next week",
        " by monday",
        " by tuesday",
        " by wednesday",
        " by thursday",
        " by friday",
        " by saturday",
        " by sunday",
    ];
    if let Some(index) = trailing
        .iter()
        .filter_map(|suffix| lower.rfind(suffix))
        .min()
    {
        title.truncate(index);
        title = title.trim().to_owned();
    }
    let mut chars = title.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => title,
    }
}

fn parse_deadline(candidate: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let lower = candidate.to_ascii_lowercase();
    let date = if lower == "today" {
        now.date_naive()
    } else if lower == "tomorrow" {
        now.date_naive() + chrono::Duration::days(1)
    } else if let Some(name) = lower.strip_prefix("by ").or(Some(lower.as_str())) {
        let weekdays = [
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
        ];
        let index = weekdays.iter().position(|day| *day == name)?;
        let target = chrono::Weekday::try_from(index as u8 + 1).ok()?;
        let mut days =
            (target.number_from_monday() as i64 - now.weekday().number_from_monday() as i64 + 7)
                % 7;
        if days == 0 {
            days = 7;
        }
        now.date_naive() + chrono::Duration::days(days)
    } else if let Ok(value) = chrono::NaiveDate::parse_from_str(candidate, "%Y-%m-%d") {
        value
    } else if let Ok(value) = chrono::NaiveDate::parse_from_str(candidate, "%m/%d/%Y") {
        value
    } else {
        return None;
    };
    Some(date.and_hms_opt(17, 0, 0)?.and_utc())
}

/// Understand captured text, with a deterministic line fallback when Jev is unavailable.
pub async fn capture_text(text: &str, source: Source, anchor: DateTime<Utc>) -> Vec<Task> {
    let sentences = split_sentences(text);
    if sentences.is_empty() || std::env::var("TYPESAFE_API_KEY").is_err() {
        return tasks_from_text(text)
            .into_iter()
            .map(|mut task| {
                task.source = source;
                task
            })
            .collect();
    }
    let results = join_all(sentences.iter().map(|sentence| async move {
        let candidates = find_date_candidates(sentence, anchor);
        understand(sentence, &candidates, anchor)
            .await
            .map(|judgment| (sentence, candidates, judgment))
    }))
    .await;
    let mut tasks = Vec::new();
    for result in results {
        let Ok((sentence, candidates, judgment)) = result else {
            eprintln!("warning: Jev capture failed; using line-per-task fallback");
            return tasks_from_text(text)
                .into_iter()
                .map(|mut task| {
                    task.source = source;
                    task
                })
                .collect();
        };
        if judgment.is_task < 0.5 {
            continue;
        }
        let deadline = judgment
            .deadline
            .as_deref()
            .and_then(|candidate| parse_deadline(candidate, anchor));
        let _ = candidates;
        tasks.push(Task {
            id: String::new(),
            title: clean_title(sentence),
            notes: String::new(),
            duration_min: i64::from(judgment.duration_min),
            deadline,
            priority: judgment.priority,
            status: "todo".into(),
            confidence: judgment.confidence,
            source,
        });
    }
    tasks
}

fn timezone() -> Tz {
    std::env::var("TZ_NAME")
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(chrono_tz::America::New_York)
}

fn local_time_as_utc(tz: Tz, local: NaiveDateTime) -> DateTime<Utc> {
    tz.from_local_datetime(&local)
        .single()
        .unwrap_or_else(|| tz.from_utc_datetime(&local))
        .with_timezone(&Utc)
}

/// Busy blocks for today in the configured local timezone, returned as UTC.
fn seed_busy(today: DateTime<Utc>) -> Vec<BusyOut> {
    let tz = timezone();
    let local_date = today.with_timezone(&tz).date_naive();
    let at = |h: u32, m: u32| {
        local_time_as_utc(
            tz,
            local_date.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap()),
        )
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
    let mut created =
        capture_text(&body.text, body.source.unwrap_or(Source::Text), Utc::now()).await;
    let mut store = state.store.lock().unwrap();
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
    let tz = timezone();
    let local_now = Utc::now().with_timezone(&tz);
    let local_start = local_now
        .date_naive()
        .and_time(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    let local_end = local_now
        .date_naive()
        .and_time(NaiveTime::from_hms_opt(18, 0, 0).unwrap());
    let start_utc = local_time_as_utc(tz, local_start).time();
    let end_utc = local_time_as_utc(tz, local_end).time();
    let hours = WorkingHours::every_day(start_utc, end_utc);

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

    #[test]
    fn clean_title_removes_hedges_and_deadline() {
        assert_eq!(
            clean_title("Ugh okay so I need to finish the chem lab report by Friday"),
            "Finish the chem lab report"
        );
        assert_eq!(clean_title("um, I should call mom tomorrow"), "Call mom");
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

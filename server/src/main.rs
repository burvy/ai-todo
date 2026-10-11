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
use chrono::{DateTime, Datelike, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use futures::future::join_all;
use judge::{classify_intent, find_date_candidates, split_sentences, understand};
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
    busy: Arc<Mutex<Vec<BusyOut>>>,
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

#[derive(Deserialize)]
struct CreateTaskIn {
    title: String,
    #[serde(default)]
    notes: String,
    #[serde(default = "default_duration")]
    duration_min: i64,
    deadline: Option<DateTime<Utc>>,
    #[serde(default = "default_priority")]
    priority: u8,
    #[serde(default = "default_label")]
    label: String,
}

fn default_duration() -> i64 {
    30
}
fn default_priority() -> u8 {
    1
}
fn default_label() -> String {
    "Other".into()
}

#[derive(Deserialize)]
struct MessageIn {
    text: String,
    #[serde(default)]
    source: Option<Source>,
}

#[derive(Serialize)]
struct MessageOut {
    reply: String,
    intent: String,
    tasks: Vec<Task>,
    placements: Vec<PlacementOut>,
    unscheduled: Vec<UnscheduledOut>,
    focus_date: Option<String>,
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
            label: label_for(line),
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
            "i have a ",
            "i should ",
            "i have to ",
            "i gotta ",
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
        " today",
        " tomorrow",
        " next week",
        " by monday",
        " by tuesday",
        " by wednesday",
        " by thursday",
        " by friday",
        " by saturday",
        " by sunday",
        " monday",
        " tuesday",
        " wednesday",
        " thursday",
        " friday",
        " saturday",
        " sunday",
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

fn label_for(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if ["class", "lab", "homework"]
        .iter()
        .any(|word| lower.contains(word))
    {
        "School"
    } else if ["work", "report", "meeting"]
        .iter()
        .any(|word| lower.contains(word))
    {
        "Work"
    } else if ["doctor", "gym", "health"]
        .iter()
        .any(|word| lower.contains(word))
    {
        "Health"
    } else if ["buy", "shop", "grocer"]
        .iter()
        .any(|word| lower.contains(word))
    {
        "Errands"
    } else if ["family", "friend", "mom"]
        .iter()
        .any(|word| lower.contains(word))
    {
        "Personal"
    } else {
        "Other"
    }
    .into()
}

fn parse_deadline(candidate: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let lower = candidate.to_ascii_lowercase();
    let date = if lower == "today" {
        now.date_naive()
    } else if lower == "tomorrow" {
        now.date_naive() + chrono::Duration::days(1)
    } else if let Some(name) = lower.strip_prefix("by ") {
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
    } else if [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ]
    .contains(&lower.as_str())
    {
        let weekdays = [
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
        ];
        let index = weekdays.iter().position(|day| *day == lower)?;
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
    Some(local_time_as_utc(
        timezone(),
        date.and_time(NaiveTime::from_hms_opt(17, 0, 0).unwrap()),
    ))
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
    let judgments: Result<Vec<_>, _> = results
        .into_iter()
        .map(|result| {
            result.map(|(sentence, candidates, judgment)| (sentence.clone(), candidates, judgment))
        })
        .collect();
    let Ok(judgments) = judgments else {
        eprintln!("warning: Jev capture failed; using line-per-task fallback");
        return tasks_from_text(text)
            .into_iter()
            .map(|mut task| {
                task.source = source;
                task
            })
            .collect();
    };
    tasks_from_judgments(judgments, source, anchor)
}

fn tasks_from_judgments(
    judgments: Vec<(String, Vec<String>, judge::Judgment)>,
    source: Source,
    anchor: DateTime<Utc>,
) -> Vec<Task> {
    let mut tasks = Vec::new();
    for (sentence, _candidates, judgment) in judgments {
        if judgment.is_task < 0.5 {
            continue;
        }
        let deadline = judgment
            .deadline
            .as_deref()
            .and_then(|candidate| parse_deadline(candidate, anchor));
        tasks.push(Task {
            id: String::new(),
            title: clean_title(&sentence),
            notes: String::new(),
            duration_min: i64::from(judgment.duration_min),
            deadline,
            priority: judgment.priority,
            status: "todo".into(),
            confidence: judgment.confidence,
            label: label_for(&sentence),
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

async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskIn>,
) -> (StatusCode, Json<Task>) {
    let mut store = state.store.lock().unwrap();
    store.next_id += 1;
    let task = Task {
        id: format!("t{}", store.next_id),
        title: clean_title(&body.title),
        notes: body.notes,
        duration_min: body.duration_min,
        deadline: body.deadline,
        priority: body.priority,
        status: "todo".into(),
        confidence: 1.0,
        label: body.label,
        source: Source::Text,
    };
    store.tasks.push(task.clone());
    (StatusCode::CREATED, Json(task))
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
        .filter(|task| task.status != "done")
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
        .lock()
        .unwrap()
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
        .lock()
        .unwrap()
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

fn fallback_intent(text: &str) -> &'static str {
    let lower = text.to_ascii_lowercase();
    if lower.contains("cancel") || lower.contains("got cancelled") {
        "cancel_event"
    } else if lower.contains("finished") || lower.contains("completed") || lower.contains("done") {
        "complete_task"
    } else if lower.contains("what does") || lower.contains("what's") || lower.contains("calendar")
    {
        "query_day"
    } else if !lower.trim().is_empty() {
        "add_tasks"
    } else {
        "other"
    }
}

fn mentioned_hour(text: &str) -> Option<u32> {
    text.to_ascii_lowercase()
        .split_whitespace()
        .find_map(|word| {
            let token = word.trim_matches(|character: char| !character.is_ascii_alphanumeric());
            let (digits, suffix) = token.split_at(token.len().saturating_sub(2));
            if !matches!(suffix, "am" | "pm") {
                return None;
            }
            let mut hour = digits.parse::<u32>().ok()?;
            if hour == 0 || hour > 12 {
                return None;
            }
            if suffix == "pm" && hour != 12 {
                hour += 12;
            }
            if suffix == "am" && hour == 12 {
                hour = 0;
            }
            Some(hour)
        })
}

fn task_matches_message(task: &Task, message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    let words: Vec<String> = task
        .title
        .split_whitespace()
        .map(|word| word.trim_matches(|character: char| !character.is_ascii_alphanumeric()))
        .filter(|word| !["a", "an", "and", "i", "my", "the", "to"].contains(word))
        .map(str::to_ascii_lowercase)
        .collect();
    !words.is_empty()
        && words
            .iter()
            .all(|word| message.contains(word) || message.contains(&format!("{word}ed")))
}

async fn message(State(state): State<AppState>, Json(body): Json<MessageIn>) -> Json<MessageOut> {
    let intent = classify_intent(&body.text)
        .await
        .unwrap_or_else(|_| fallback_intent(&body.text).into());
    let mut created = Vec::new();
    let mut focus_date = None;
    match intent.as_str() {
        "add_tasks" => {
            created =
                capture_text(&body.text, body.source.unwrap_or(Source::Text), Utc::now()).await;
            let mut store = state.store.lock().unwrap();
            for task in &mut created {
                store.next_id += 1;
                task.id = format!("t{}", store.next_id);
            }
            store.tasks.extend(created.iter().cloned());
        }
        "cancel_event" => {
            let lower = body.text.to_ascii_lowercase();
            let hour = mentioned_hour(&body.text);
            let mut busy = state.busy.lock().unwrap();
            busy.retain(|event| {
                let title_match = lower.contains(&event.title.to_ascii_lowercase());
                let hour_match = hour == Some(event.start.with_timezone(&timezone()).hour());
                !(title_match || hour_match)
            });
        }
        "complete_task" => {
            let lower = body.text.to_ascii_lowercase();
            let mut store = state.store.lock().unwrap();
            if let Some(task) = store
                .tasks
                .iter_mut()
                .find(|task| task_matches_message(task, &lower))
            {
                task.status = "done".into();
            }
        }
        "query_day" => {
            let lower = body.text.to_ascii_lowercase();
            let date = if lower.contains("tomorrow") {
                Utc::now().date_naive() + chrono::Duration::days(1)
            } else {
                Utc::now().date_naive()
            };
            focus_date = Some(date.to_string());
        }
        _ => {}
    }
    let Json(schedule) = schedule_tasks(State(state.clone())).await;
    let tasks = state.store.lock().unwrap().tasks.clone();
    let reply = match intent.as_str() {
        "add_tasks" => {
            let noun = if created.len() == 1 { "task" } else { "tasks" };
            format!("Got it, {} new {}.", created.len(), noun)
        }
        "cancel_event" => "Got it, the event was cancelled.".into(),
        "complete_task" => "Got it, the task is marked done.".into(),
        "query_day" => "Here is your day.".into(),
        _ => "I can add tasks, cancel events, complete tasks, or show your day.".into(),
    };
    Json(MessageOut {
        reply,
        intent,
        tasks,
        placements: schedule.placements,
        unscheduled: schedule.unscheduled,
        focus_date,
    })
}

fn app(state: AppState) -> Router {
    let mut router = Router::new()
        .route("/api/capture", post(capture))
        .route("/api/message", post(message))
        .route("/api/tasks", get(list_tasks))
        .route("/api/tasks", post(create_task))
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
        busy: Arc::new(Mutex::new(seed_busy(Utc::now()))),
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
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
        assert_eq!(
            clean_title("Ugh okay so I have a calc quiz Thursday"),
            "Calc quiz"
        );
        assert_eq!(
            clean_title("I gotta pick up my prescription today"),
            "Pick up my prescription"
        );
    }

    #[test]
    fn completion_matches_inflected_message() {
        let task = Task {
            id: "t1".into(),
            title: "Finish my chem lab report".into(),
            notes: String::new(),
            duration_min: 30,
            deadline: None,
            priority: 1,
            status: "todo".into(),
            confidence: 1.0,
            label: "School".into(),
            source: Source::Text,
        };
        assert!(task_matches_message(
            &task,
            "I finished the chem lab report"
        ));
    }

    #[test]
    fn fake_judge_capture_keeps_two_tasks_and_friday_deadline() {
        let anchor = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        let sentences = judge::split_sentences(
            "I need to finish my chem lab report by Friday and email Professor Lee tomorrow",
        );
        let judgments = sentences
            .iter()
            .map(|sentence| {
                let candidates = find_date_candidates(sentence, anchor);
                (
                    sentence.clone(),
                    candidates.clone(),
                    judge::Judgment {
                        is_task: 0.9,
                        deadline: candidates.first().cloned(),
                        priority: 1,
                        duration_min: 30,
                        confidence: 1.0,
                    },
                )
            })
            .collect();
        let tasks = tasks_from_judgments(judgments, Source::Text, anchor);
        assert_eq!(tasks.len(), 2);
        assert_eq!(
            tasks[0]
                .deadline
                .unwrap()
                .with_timezone(&timezone())
                .date_naive()
                .to_string(),
            "2026-10-16"
        );
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

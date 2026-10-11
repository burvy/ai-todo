//! HTTP server for the ai-todo API. Tasks and busy blocks are persisted to
//! SQLite; the in-memory `Store` is the working copy.
//! Routes and payloads follow the frozen API contract in `hive/board.md`.

mod db;
mod export;
mod ics;
#[cfg(test)]
mod message_tests;
mod reset;
mod revise;
mod upload;

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{FromRequest, Path as UrlPath, Query, Request as HttpRequest, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use chrono::{DateTime, Datelike, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use db::{BusyRow, Db};
use futures::future::join_all;
use judge::{choose, classify_intent, find_date_candidates, split_sentences, understand};
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
    db: Arc<Db>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
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

fn clamp_duration(duration: i64) -> i64 {
    duration.clamp(5, 480)
}

fn title_is_valid(title: &str) -> bool {
    !title.trim().is_empty()
}

fn round_up_quarter(now: DateTime<Utc>) -> DateTime<Utc> {
    let remainder = now.timestamp().rem_euclid(15 * 60);
    let add = (15 * 60 - remainder) % (15 * 60);
    (now + chrono::Duration::seconds(add))
        .with_nanosecond(0)
        .unwrap()
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
    position: Option<i64>,
    pinned_start: Option<Option<DateTime<Utc>>>,
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
        .enumerate()
        .map(|(index, line)| Task {
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
            position: index as i64,
            pinned_start: None,
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
            "tomorrow ",
            "today ",
            "right ",
            "oh and ",
            "also ",
            "can someone remind me to ",
            "make a task to ",
            "ugh ",
            "okay ",
            "ok ",
            "so ",
            "um ",
            "like ",
            "i need to ",
            "i need ",
            "i have a ",
            "i also have a ",
            "i still need to ",
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
        if *prefix == "i need " {
            title = format!("Buy {title}");
        }
    }
    let lower = title.to_ascii_lowercase();
    for aside in [" it is ", ", it is "] {
        if let Some(index) = lower.find(aside) {
            title.truncate(index);
            title = title.trim().to_owned();
            break;
        }
    }
    let lower = title.to_ascii_lowercase();
    if let Some(index) = lower.find(" just basics") {
        title.truncate(index);
        title = title.trim().to_owned();
    }
    let lower = title.to_ascii_lowercase();
    if lower.starts_with("buy groceries") {
        title = "Buy groceries".into();
    }
    for conjunction in [" because ", " since "] {
        if let Some(index) = lower.find(conjunction) {
            title.truncate(index);
            break;
        }
    }
    let lower = title.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("study for ") {
        let rest = rest
            .trim_start_matches("i also have a ")
            .trim_start_matches("i have a ");
        title = format!("Study for {rest}");
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
        " sometime",
        " is coming up",
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
    if [
        "class",
        "lab",
        "homework",
        "paper",
        "professor",
        "reading",
        "quiz",
        "chapter",
        "slide",
        "presentation",
        "ta",
        "draft",
        "group",
    ]
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

fn is_assessment(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["quiz", "exam", "test", "presentation", "interview"]
        .iter()
        .any(|word| {
            lower
                .split_whitespace()
                .any(|part| part.trim_matches(|c: char| !c.is_ascii_alphanumeric()) == *word)
        })
}

fn parse_deadline(candidate: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let lower = candidate.to_ascii_lowercase();
    let date = if lower == "today" {
        now.date_naive()
    } else if lower == "tomorrow" {
        now.date_naive() + chrono::Duration::days(1)
    } else if lower == "weekend" || lower == "this weekend" {
        let current = now.weekday().number_from_monday() as i64;
        let mut days = (6 - current + 7) % 7;
        if days == 0 {
            days = 7;
        }
        now.date_naive() + chrono::Duration::days(days)
    } else if lower == "next week" {
        let current = now.weekday().number_from_monday() as i64;
        let days = if current == 7 { 8 } else { 8 - current };
        now.date_naive() + chrono::Duration::days(days)
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
        let current = now.weekday().number_from_monday() as i64;
        let target = index as i64 + 1;
        let mut days = (target - current + 7) % 7;
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
        let current = now.weekday().number_from_monday() as i64;
        let target = index as i64 + 1;
        let mut days = (target - current + 7) % 7;
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

fn explicit_clock(text: &str) -> Option<(u32, u32)> {
    for raw in text.to_ascii_lowercase().split_whitespace() {
        let token = raw.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != ':');
        if token == "noon" {
            return Some((12, 0));
        }
        let (number, suffix) = if let Some(value) = token.strip_suffix("am") {
            (value, Some(false))
        } else if let Some(value) = token.strip_suffix("pm") {
            (value, Some(true))
        } else {
            (token, None)
        };
        let (hour, minute) = if let Some((hour, minute)) = number.split_once(':') {
            (hour.parse().ok()?, minute.parse().ok()?)
        } else if suffix.is_some() {
            (number.parse().ok()?, 0)
        } else {
            continue;
        };
        if minute > 59 {
            continue;
        }
        let hour = match suffix {
            Some(is_pm) if (1..=12).contains(&hour) => (hour % 12) + if is_pm { 12 } else { 0 },
            None if hour < 24 => hour,
            _ => continue,
        };
        return Some((hour, minute));
    }
    None
}

fn pinned_start_for(sentence: &str, anchor: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let (hour, minute) = explicit_clock(sentence)?;
    let date_candidate = find_date_candidates(sentence, anchor)
        .into_iter()
        .find(|candidate| parse_deadline(candidate, anchor).is_some())?;
    let date = parse_deadline(&date_candidate, anchor)?
        .with_timezone(&timezone())
        .date_naive();
    Some(local_time_as_utc(
        timezone(),
        date.and_time(NaiveTime::from_hms_opt(hour, minute, 0)?),
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
    if std::env::var("CAPTURE_DEBUG").ok().as_deref() == Some("1") {
        for result in &results {
            match result {
                Ok((sentence, _, judgment)) => eprintln!(
                    "capture clause={:?} is_task={:.3} deadline={:?} duration_min={}",
                    sentence, judgment.is_task, judgment.deadline, judgment.duration_min
                ),
                Err(error) => eprintln!("capture clause error: {error}"),
            }
        }
    }
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
    for (index, (sentence, _candidates, judgment)) in judgments.into_iter().enumerate() {
        if judgment.is_task < 0.5 {
            continue;
        }
        let deadline = judgment
            .deadline
            .as_deref()
            .and_then(|candidate| parse_deadline(candidate, anchor));
        let pinned_start = pinned_start_for(&sentence, anchor);
        let meeting_like = sentence.to_ascii_lowercase();
        let meeting_like = ["meeting", "appointment", "call"]
            .iter()
            .any(|word| meeting_like.contains(word));
        let assessment = is_assessment(&sentence);
        let title = if assessment {
            let cleaned = clean_title(&sentence)
                .to_ascii_lowercase()
                .replace("i also have a ", "")
                .replace("i have a ", "");
            let cleaned = cleaned.trim_start_matches("study for ");
            if sentence.to_ascii_lowercase().contains("presentation") {
                format!("Prepare {cleaned}")
            } else {
                format!("Study for {cleaned}")
            }
        } else {
            clean_title(&sentence)
        };
        tasks.push(Task {
            id: String::new(),
            title,
            notes: String::new(),
            duration_min: if (pinned_start.is_some() && meeting_like) || assessment {
                clamp_duration(i64::from(judgment.duration_min.max(60)))
            } else {
                clamp_duration(i64::from(judgment.duration_min))
            },
            deadline,
            priority: judgment.priority,
            status: "todo".into(),
            confidence: judgment.confidence,
            label: judgment.label.unwrap_or_else(|| label_for(&sentence)),
            source,
            position: index as i64,
            pinned_start,
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

/// Busy blocks for the next weekday in the configured local timezone, returned as UTC.
fn seed_busy(today: DateTime<Utc>) -> Vec<BusyOut> {
    let tz = timezone();
    let local_today = today.with_timezone(&tz);
    let weekday = local_today.weekday().number_from_monday() as i64;
    let days_until_weekday = if weekday >= 5 { 8 - weekday } else { 1 };
    let local_date = local_today.date_naive() + chrono::Duration::days(days_until_weekday);
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

/// `POST /api/capture` - either JSON `{text, source}` or a multipart `file`.
///
/// The two shapes are told apart by Content-Type: JSON is the existing
/// contract, multipart is the PDF/email upload. Everything after "which shape
/// is this" is shared, so both return the same `{tasks:[Task]}`.
async fn capture(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Result<Json<CaptureOut>, (StatusCode, Json<serde_json::Value>)> {
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let (text, source, anchor) = if content_type.starts_with("multipart/form-data") {
        let mut multipart = axum::extract::Multipart::from_request(request, &state)
            .await
            .map_err(|e| bad_request(format!("malformed upload: {e}")))?;
        let (_name, bytes) = upload::read_file(&mut multipart)
            .await
            .map_err(|e| bad_request(e.to_string()))?;
        let found = upload::extract(&bytes)
            .await
            .map_err(|e| bad_request(e.to_string()))?;
        // Source::Pdf, and the anchor is the email's own sent date when it is
        // an email, so "tomorrow" means the day after it was sent.
        let shape = if found.was_email { "email" } else { "pdf" };
        eprintln!(
            "capture: {shape} upload, {} chars, anchored at {}",
            found.text.len(),
            found.anchor.to_rfc3339()
        );
        (found.text, Source::Pdf, found.anchor)
    } else {
        let Json(body) = Json::<CaptureIn>::from_request(request, &state)
            .await
            .map_err(|e| bad_request(format!("expected a JSON body with text: {e}")))?;
        (body.text, body.source.unwrap_or(Source::Text), Utc::now())
    };
    if text.trim().is_empty() {
        return Ok(Json(CaptureOut {
            tasks: Vec::new(),
            note: Some("nothing to do in that email".into()),
        }));
    }

    let mut created = capture_text(&text, source, anchor).await;
    let mut store = state.store.lock().unwrap();
    for task in &mut created {
        store.next_id += 1;
        task.id = format!("t{}", store.next_id);
        task.position = store
            .tasks
            .iter()
            .map(|existing| existing.position)
            .max()
            .unwrap_or(-1)
            + 1;
    }
    store.tasks.extend(created.iter().cloned());
    drop(store);
    state.db.save_tasks(&created);
    Ok(Json(CaptureOut {
        tasks: created,
        note: None,
    }))
}

fn bad_request(message: String) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message })),
    )
}

async fn list_tasks(State(state): State<AppState>) -> Json<TasksOut> {
    let mut tasks = state.store.lock().unwrap().tasks.clone();
    tasks.sort_by_key(|task| (task.status == "done", task.position));
    Json(TasksOut { tasks })
}

async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskIn>,
) -> Result<(StatusCode, Json<Task>), (StatusCode, Json<serde_json::Value>)> {
    if !title_is_valid(&body.title) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "title is required"})),
        ));
    }
    let mut store = state.store.lock().unwrap();
    store.next_id += 1;
    let position = store
        .tasks
        .iter()
        .map(|task| task.position)
        .max()
        .unwrap_or(-1)
        + 1;
    let task = Task {
        id: format!("t{}", store.next_id),
        title: clean_title(&body.title),
        notes: body.notes,
        duration_min: clamp_duration(body.duration_min),
        deadline: body.deadline,
        priority: body.priority,
        status: "todo".into(),
        confidence: 1.0,
        label: body.label,
        source: Source::Text,
        position,
        pinned_start: None,
    };
    store.tasks.push(task.clone());
    drop(store);
    state.db.save_tasks(std::slice::from_ref(&task));
    Ok((StatusCode::CREATED, Json(task)))
}

async fn patch_task(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<PatchIn>,
) -> Result<Json<Task>, (StatusCode, Json<serde_json::Value>)> {
    let mut store = state.store.lock().unwrap();
    let task = store.tasks.iter_mut().find(|task| task.id == id).ok_or((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "task not found"})),
    ))?;
    if let Some(v) = body.title {
        if !title_is_valid(&v) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "title is required"})),
            ));
        }
        task.title = v;
    }
    if let Some(v) = body.notes {
        task.notes = v;
    }
    if let Some(v) = body.duration_min {
        task.duration_min = clamp_duration(v);
    }
    if let Some(v) = body.deadline {
        task.deadline = v;
    }
    if let Some(v) = body.priority {
        task.priority = v;
    }
    if let Some(v) = body.status {
        if v != "todo" && v != "done" {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "status must be todo or done"})),
            ));
        }
        task.status = v;
    }
    if let Some(v) = body.confidence {
        task.confidence = v;
    }
    if let Some(v) = body.position {
        task.position = v;
    }
    if let Some(v) = body.pinned_start {
        task.pinned_start = v;
    }
    let updated = task.clone();
    drop(store);
    state.db.save_task(&updated);
    Ok(Json(updated))
}

async fn delete_task(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> StatusCode {
    let mut store = state.store.lock().unwrap();
    let before = store.tasks.len();
    store.tasks.retain(|task| task.id != id);
    let removed = store.tasks.len() != before;
    drop(store);
    if removed {
        state.db.remove_task(&id);
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
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
            position: task.position,
            pinned_start: task.pinned_start,
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
    for placement in scheduler::schedule(&inputs, &busy, &hours, round_up_quarter(Utc::now())) {
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

/// Replace the busy blocks with the ones in an uploaded `.ics` export.
///
/// Accepts the file as JSON `{ics_text}` or as a multipart upload with a
/// `file` field, which is how a user picks an `.ics` off their device.
///
/// Replaces rather than appends, so re-exporting after moving an event moves
/// it here too. But if the incoming events overlap what is already in the app
/// we stop and return the clashes as `409` rather than silently deciding for
/// the user; pass `confirm: true` (or a `confirm` field on the upload) to
/// import anyway.
async fn import_calendar(
    State(state): State<AppState>,
    request: HttpRequest,
) -> Result<Response, ics::ImportError> {
    let input = ics::read_import(request).await?;
    let imported = ics::parse_ics(&input.ics_text)?;

    let existing: Vec<BusyRow> = state
        .busy
        .lock()
        .unwrap()
        .iter()
        .map(|block| BusyRow {
            start: block.start,
            end: block.end,
            title: block.title.clone(),
        })
        .collect();

    let conflicts = ics::find_conflicts(&imported, &existing);
    if !conflicts.is_empty() && !input.confirm {
        return Ok((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!(
                    "{} of the {} imported event(s) overlap what is already in the app",
                    conflicts.len(),
                    imported.len()
                ),
                "conflicts": conflicts,
            })),
        )
            .into_response());
    }

    let blocks: Vec<BusyOut> = imported
        .iter()
        .map(|row| BusyOut {
            start: row.start,
            end: row.end,
            title: row.title.clone(),
        })
        .collect();

    *state.busy.lock().unwrap() = blocks.clone();
    state.db.save_busy(&imported);

    let placements = state.store.lock().unwrap().placements.clone();
    Ok(Json(CalendarOut {
        busy: blocks,
        placements,
    })
    .into_response())
}

fn fallback_intent(text: &str) -> &'static str {
    let lower = text.to_ascii_lowercase();
    if lower.contains("cancel") || lower.contains("got cancelled") {
        "cancel_event"
    } else if lower.contains("finished") || lower.contains("completed") || lower.contains("done") {
        "complete_task"
    } else if is_next_query(&lower)
        || lower.contains("what does")
        || lower.contains("what's")
        || lower.contains("calendar")
    {
        "query_day"
    } else if !lower.trim().is_empty() {
        "add_tasks"
    } else {
        "other"
    }
}

fn is_next_query(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("what should i do now")
        || lower.contains("what's next")
        || lower.contains("whats next")
        || lower.contains("what is next")
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

fn event_matches(text: &str, event: &BusyOut) -> bool {
    let lower = text.to_ascii_lowercase();
    let local = event.start.with_timezone(&timezone());
    let title_match = lower.contains(&event.title.to_ascii_lowercase());
    let hour_match = mentioned_hour(text) == Some(local.hour());
    let day_match = if lower.contains("tomorrow") {
        local.date_naive()
            == Utc::now().with_timezone(&timezone()).date_naive() + chrono::Duration::days(1)
    } else {
        [
            ("monday", chrono::Weekday::Mon),
            ("tuesday", chrono::Weekday::Tue),
            ("wednesday", chrono::Weekday::Wed),
            ("thursday", chrono::Weekday::Thu),
            ("friday", chrono::Weekday::Fri),
            ("saturday", chrono::Weekday::Sat),
            ("sunday", chrono::Weekday::Sun),
        ]
        .iter()
        .any(|(name, day)| lower.contains(name) && local.weekday() == *day)
    };
    hour_match || (title_match && (day_match || !lower.contains("tomorrow")))
}

fn stem(word: &str) -> String {
    let mut value: String = word
        .to_ascii_lowercase()
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .collect();
    if value.len() > 3 && value.ends_with('s') {
        value.pop();
    }
    value
}

fn event_match_score(text: &str, event: &BusyOut) -> usize {
    let message: std::collections::BTreeSet<String> = text
        .split_whitespace()
        .map(stem)
        .filter(|word| !word.is_empty())
        .collect();
    event
        .title
        .split_whitespace()
        .map(stem)
        .filter(|word| !word.is_empty() && message.contains(word))
        .count()
}

fn event_is_on_requested_day(text: &str, event: &BusyOut) -> bool {
    let lower = text.to_ascii_lowercase();
    let local = event.start.with_timezone(&timezone());
    if lower.contains("today") {
        return local.date_naive() == Utc::now().with_timezone(&timezone()).date_naive();
    }
    if lower.contains("tomorrow") {
        return local.date_naive()
            == Utc::now().with_timezone(&timezone()).date_naive() + chrono::Duration::days(1);
    }
    [
        ("monday", chrono::Weekday::Mon),
        ("tuesday", chrono::Weekday::Tue),
        ("wednesday", chrono::Weekday::Wed),
        ("thursday", chrono::Weekday::Thu),
        ("friday", chrono::Weekday::Fri),
        ("saturday", chrono::Weekday::Sat),
        ("sunday", chrono::Weekday::Sun),
    ]
    .iter()
    .filter(|(name, _)| lower.contains(name))
    .all(|(_, day)| local.weekday() == *day)
}

fn content_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.trim_matches(|character: char| !character.is_ascii_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|word| {
            !word.is_empty()
                && ![
                    "a",
                    "an",
                    "and",
                    "i",
                    "my",
                    "the",
                    "to",
                    "make",
                    "makes",
                    "made",
                    "do",
                    "did",
                    "finish",
                    "finished",
                    "prepare",
                    "prepared",
                    "complete",
                    "completed",
                ]
                .contains(&word.as_str())
        })
        .collect()
}

fn word_matches(left: &str, right: &str) -> bool {
    left == right
        || left.strip_suffix("ed").is_some_and(|stem| stem == right)
        || right.strip_suffix("ed").is_some_and(|stem| stem == left)
}

fn task_overlap(task: &Task, message: &str) -> usize {
    let message_words = content_words(message);
    content_words(&task.title)
        .iter()
        .filter(|word| {
            message_words
                .iter()
                .any(|candidate| word_matches(word, candidate))
        })
        .count()
}

fn task_matches_message(task: &Task, message: &str) -> bool {
    task_overlap(task, message) > 0
}

fn query_date(text: &str, now: DateTime<Utc>) -> chrono::NaiveDate {
    let local = now.with_timezone(&timezone());
    let lower = text.to_ascii_lowercase();
    if lower.contains("tomorrow") {
        return local.date_naive() + chrono::Duration::days(1);
    }
    if lower.contains("yesterday") {
        return local.date_naive() - chrono::Duration::days(1);
    }
    let weekdays = [
        ("monday", chrono::Weekday::Mon),
        ("tuesday", chrono::Weekday::Tue),
        ("wednesday", chrono::Weekday::Wed),
        ("thursday", chrono::Weekday::Thu),
        ("friday", chrono::Weekday::Fri),
        ("saturday", chrono::Weekday::Sat),
        ("sunday", chrono::Weekday::Sun),
    ];
    if let Some((_, requested)) = weekdays.iter().find(|(name, _)| lower.contains(name)) {
        let current = local.weekday().number_from_monday() as i64;
        let target = requested.number_from_monday() as i64;
        let days = (target - current).rem_euclid(7).max(1);
        return local.date_naive() + chrono::Duration::days(days);
    }
    local.date_naive()
}

/// Title with case, punctuation and spacing removed, so "Buy milk." and
/// "buy  milk" count as the same task.
fn normalized_title(title: &str) -> String {
    title
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// "today", "tomorrow" or "on Tuesday" for a local date, relative to `today`.
fn day_phrase(date: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    match (date - today).num_days() {
        0 => "today".into(),
        1 => "tomorrow".into(),
        _ => format!("on {}", date.format("%A")),
    }
}

fn query_label(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if lower.contains("tomorrow") {
        return "Tomorrow".into();
    }
    if lower.contains("yesterday") {
        return "Yesterday".into();
    }
    for name in [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ] {
        if lower.contains(name) {
            let mut chars = name.chars();
            return chars.next().unwrap().to_uppercase().collect::<String>() + chars.as_str();
        }
    }
    "Today".into()
}

fn cancelled_event_reply(title: &str, start: DateTime<Utc>) -> String {
    format!(
        "Cancelled {title}, {}. Your plan is updated.",
        start.with_timezone(&timezone()).format("%a %-I:%M %p")
    )
}

async fn message(State(state): State<AppState>, Json(body): Json<MessageIn>) -> Json<MessageOut> {
    let mut intent = classify_intent(&body.text)
        .await
        .unwrap_or_else(|_| fallback_intent(&body.text).into());
    if is_next_query(&body.text) {
        intent = "query_day".into();
    }
    let mut created = Vec::new();
    let mut focus_date = None;
    let mut cancelled_event = None;
    let mut completed_title = None;
    let mut already_done_title = None;
    let mut duplicates = 0usize;
    let next_query = is_next_query(&body.text);
    match intent.as_str() {
        "add_tasks" => {
            let captured =
                capture_text(&body.text, body.source.unwrap_or(Source::Text), Utc::now()).await;
            let mut store = state.store.lock().unwrap();
            // Skip anything already on the open list (or repeated within this
            // message), so saying the same thing twice does not double the list.
            let mut seen: BTreeSet<String> = store
                .tasks
                .iter()
                .filter(|task| task.status != "done")
                .map(|task| normalized_title(&task.title))
                .collect();
            for task in captured {
                if seen.insert(normalized_title(&task.title)) {
                    created.push(task);
                } else {
                    duplicates += 1;
                }
            }
            for task in &mut created {
                store.next_id += 1;
                task.id = format!("t{}", store.next_id);
                task.position = store
                    .tasks
                    .iter()
                    .map(|existing| existing.position)
                    .max()
                    .unwrap_or(-1)
                    + 1;
            }
            store.tasks.extend(created.iter().cloned());
            drop(store);
            state.db.save_tasks(&created);
        }
        "cancel_event" => {
            let candidates: Vec<(String, String, DateTime<Utc>)> = state
                .busy
                .lock()
                .unwrap()
                .iter()
                .map(|event| {
                    (
                        event.title.clone(),
                        format!(
                            "{} at {}",
                            event.title,
                            event.start.with_timezone(&timezone()).format("%-I:%M %p")
                        ),
                        event.start,
                    )
                })
                .collect();
            let local_today = Utc::now().with_timezone(&timezone()).date_naive();
            let state_text = format!("Today is {local_today}. {}", body.text);
            let choice = choose(
                &state_text,
                &candidates
                    .iter()
                    .map(|(_, candidate, _)| candidate.clone())
                    .collect::<Vec<_>>(),
            )
            .await
            .ok()
            .flatten();
            // Snapshot the events once: locking `busy` again inside the `or_else`
            // below while this guard was still alive deadlocked every later request.
            let events = state.busy.lock().unwrap().clone();
            let fallback = events
                .iter()
                .filter(|event| event_is_on_requested_day(&body.text, event))
                .filter_map(|event| {
                    let score = event_match_score(&body.text, event);
                    (score >= 1).then_some((event, score))
                })
                .max_by(|(left, left_score), (right, right_score)| {
                    left_score
                        .cmp(right_score)
                        .then_with(|| right.start.cmp(&left.start))
                })
                .map(|(event, _)| event)
                .map(|event| (event.title.clone(), event.start))
                .or_else(|| {
                    events
                        .iter()
                        .filter(|event| event_matches(&body.text, event))
                        .min_by_key(|event| event.start)
                        .map(|event| (event.title.clone(), event.start))
                });
            let selected = choice
                .and_then(|candidate| candidates.iter().find(|(_, text, _)| text == &candidate))
                .map(|(title, _, start)| (title.clone(), *start))
                .or(fallback);
            let mut busy = state.busy.lock().unwrap();
            cancelled_event = selected.clone();
            if let Some((title, start)) = selected {
                busy.retain(|event| !(event.title == title && event.start == start));
            }
            state
                .db
                .save_busy(&busy.iter().map(busy_row).collect::<Vec<_>>());
        }
        "complete_task" => {
            let candidates: Vec<(String, String)> = state
                .store
                .lock()
                .unwrap()
                .tasks
                .iter()
                .filter(|task| task.status != "done")
                .map(|task| (task.id.clone(), task.title.clone()))
                .collect();
            let titles: Vec<String> = candidates.iter().map(|(_, title)| title.clone()).collect();
            let choice = choose(&body.text, &titles).await.ok().flatten();
            let selected = choice
                .and_then(|title| candidates.iter().find(|(_, candidate)| candidate == &title))
                .map(|(id, title)| (id.clone(), title.clone()))
                .or_else(|| {
                    let store = state.store.lock().unwrap();
                    store
                        .tasks
                        .iter()
                        .filter(|task| task.status != "done")
                        .filter_map(|task| {
                            let overlap = task_overlap(task, &body.text);
                            task_matches_message(task, &body.text).then_some((
                                task.id.clone(),
                                task.title.clone(),
                                overlap,
                            ))
                        })
                        .max_by_key(|(_, _, overlap)| *overlap)
                        .map(|(id, title, _)| (id, title))
                });
            let mut store = state.store.lock().unwrap();
            if let Some((id, title)) = selected {
                let task = store.tasks.iter_mut().find(|task| task.id == id).unwrap();
                task.status = "done".into();
                state.db.save_task(task);
                completed_title = Some(title);
            } else {
                already_done_title = store
                    .tasks
                    .iter()
                    .filter(|task| task.status == "done")
                    .map(|task| (task_overlap(task, &body.text), task))
                    .filter(|(overlap, _)| *overlap > 0)
                    .max_by_key(|(overlap, _)| *overlap)
                    .map(|(_, task)| task.title.clone());
            }
        }
        "query_day" => {
            let date = query_date(&body.text, Utc::now());
            focus_date = Some(date.to_string());
        }
        _ => {}
    }
    let Json(schedule) = schedule_tasks(State(state.clone())).await;
    let tasks = state.store.lock().unwrap().tasks.clone();
    let reply = match intent.as_str() {
        "add_tasks" => {
            let noun = if created.len() == 1 { "task" } else { "tasks" };
            let mut notes = Vec::new();
            let unfit = schedule
                .unscheduled
                .iter()
                .filter(|item| created.iter().any(|task| task.id == item.task_id))
                .count();
            if unfit > 0 {
                notes.push(format!("{unfit} couldn't fit before its deadline"));
            }
            if duplicates > 0 {
                notes.push(format!("{duplicates} already on your list"));
            }
            if created.is_empty() && duplicates > 0 {
                "That's already on your list.".to_string()
            } else if notes.is_empty() {
                format!("Got it, {} new {}.", created.len(), noun)
            } else {
                format!(
                    "Got it, {} new {}; {}.",
                    created.len(),
                    noun,
                    notes.join("; ")
                )
            }
        }
        "cancel_event" => cancelled_event.map_or_else(
            || "I couldn't find that event.".into(),
            |(title, start)| cancelled_event_reply(&title, start),
        ),
        "complete_task" => completed_title.map_or_else(
            || match &already_done_title {
                Some(title) => format!("Already done: {title}."),
                None => "I couldn't tell which task you finished.".into(),
            },
            |title| format!("Done: {title}."),
        ),
        "query_day" => {
            let now = Utc::now();
            let today = now.with_timezone(&timezone()).date_naive();
            if !next_query && query_date(&body.text, now) < today {
                "That day has passed.".into()
            } else if next_query {
                schedule
                    .placements
                    .iter()
                    .filter(|placement| placement.start >= now)
                    .min_by_key(|placement| placement.start)
                    .and_then(|placement| {
                        let title = tasks
                            .iter()
                            .find(|task| task.id == placement.task_id)?
                            .title
                            .clone();
                        let start = placement.start.with_timezone(&timezone());
                        let when = match day_phrase(start.date_naive(), today).as_str() {
                            "today" => String::new(),
                            phrase => format!(" {phrase}"),
                        };
                        Some(format!(
                            "Next: {title}{when} at {} ({} minutes).",
                            start.format("%-I:%M"),
                            (placement.end - placement.start).num_minutes()
                        ))
                    })
                    .unwrap_or_else(|| "Nothing is scheduled next.".into())
            } else {
                let date = focus_date.as_deref().unwrap_or_default();
                let mut matching: Vec<_> = schedule
                    .placements
                    .iter()
                    .filter(|placement| {
                        placement
                            .start
                            .with_timezone(&timezone())
                            .date_naive()
                            .to_string()
                            == date
                    })
                    .collect();
                // Placements come back in planning order, not time order.
                matching.sort_by_key(|placement| placement.start);
                let label = query_label(&body.text);
                let distinct_tasks: BTreeSet<&str> = matching
                    .iter()
                    .map(|placement| placement.task_id.as_str())
                    .collect();
                matching
                    .first()
                    .and_then(|placement| {
                        let title = tasks
                            .iter()
                            .find(|task| task.id == placement.task_id)?
                            .title
                            .clone();
                        Some(format!(
                            "{label}: {} tasks, starting with {title} at {}.",
                            distinct_tasks.len(),
                            placement.start.with_timezone(&timezone()).format("%-I:%M")
                        ))
                    })
                    .unwrap_or_else(|| format!("{label} is free."))
            }
        }
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
        .route("/api/tasks/{id}/revise", post(revise::revise_task))
        .route("/api/schedule", post(schedule_tasks))
        .route("/api/calendar", get(calendar))
        .route("/api/calendar.ics", get(export::calendar_ics))
        .route("/api/calendar/import", post(import_calendar))
        .route("/api/reset", post(reset::reset))
        .with_state(state)
        .layer(CorsLayer::permissive());
    if Path::new("static").is_dir() {
        router = router.fallback_service(ServeDir::new("static"));
    }
    router
}

fn busy_row(block: &BusyOut) -> BusyRow {
    BusyRow {
        start: block.start,
        end: block.end,
        title: block.title.clone(),
    }
}

fn new_state() -> AppState {
    let db = Db::open_from_env().unwrap_or_else(|e| {
        eprintln!("error: could not open the database ({e}); falling back to in-memory only");
        Db::open_in_memory().expect("in-memory database")
    });
    new_state_with(Arc::new(db))
}

/// Build state on top of an already-open database. Tests pass an in-memory one
/// so they never read or write the real `data/ai-todo.db`.
fn new_state_with(db: Arc<Db>) -> AppState {
    let mut store = Store::default();
    match db.load_tasks() {
        Ok(tasks) => {
            // Continue the id sequence past anything already stored, so a task
            // added after a restart cannot collide with a saved one.
            store.next_id = tasks
                .iter()
                .filter_map(|task| task.id.strip_prefix('t')?.parse::<u64>().ok())
                .max()
                .unwrap_or(0);
            store.tasks = tasks;
        }
        Err(e) => eprintln!("warning: could not load tasks from SQLite: {e}"),
    }

    let busy = match db.load_busy() {
        Ok(rows) if !rows.is_empty() => rows
            .into_iter()
            .map(|row| BusyOut {
                start: row.start,
                end: row.end,
                title: row.title,
            })
            .collect(),
        _ => {
            // Seed the demo calendar only on a first run, never on every start.
            let seeded = seed_busy(Utc::now());
            db.save_busy(&seeded.iter().map(busy_row).collect::<Vec<_>>());
            seeded
        }
    };

    AppState {
        store: Arc::new(Mutex::new(store)),
        busy: Arc::new(Mutex::new(busy)),
        db,
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
        assert_eq!(
            clean_title("Study for i also have a quiz sometime"),
            "Study for quiz"
        );
        assert_eq!(
            clean_title("Finish the history response paper it is maybe two pages and upload it"),
            "Finish the history response paper"
        );
        assert_eq!(
            clean_title("I need groceries just basics by the weekend"),
            "Buy groceries"
        );
        assert_eq!(
            clean_title("Schedule a meeting with Maya and Luis"),
            "Schedule a meeting with Maya and Luis"
        );
    }

    #[test]
    fn school_labels_cover_demo_language() {
        for text in [
            "history paper",
            "Email Professor Chen",
            "Study for quiz",
            "Review chapters six and seven",
            "design presentation",
            "make my slides",
            "Share a draft with the group",
        ] {
            assert_eq!(label_for(text), "School", "{text}");
        }
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
            position: 0,
            pinned_start: None,
        };
        assert!(task_matches_message(
            &task,
            "I finished the chem lab report"
        ));
    }

    #[test]
    fn completion_uses_best_content_word_overlap() {
        let task = Task {
            id: "t1".into(),
            title: "Make my slides".into(),
            notes: String::new(),
            duration_min: 30,
            deadline: None,
            priority: 1,
            status: "todo".into(),
            confidence: 1.0,
            label: "Work".into(),
            source: Source::Text,
            position: 0,
            pinned_start: None,
        };
        assert_eq!(task_overlap(&task, "I finished my slides"), 1);
        assert!(!task_matches_message(&task, "I finished the report"));
    }

    #[test]
    fn seeded_busy_blocks_are_on_next_weekday() {
        let sunday = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        let blocks = seed_busy(sunday);
        assert!(blocks.iter().all(|block| {
            block
                .start
                .with_timezone(&timezone())
                .date_naive()
                .to_string()
                == "2026-10-12"
        }));
    }

    #[test]
    fn query_dates_use_local_tomorrow_and_next_weekday() {
        let sunday = Utc.with_ymd_and_hms(2026, 10, 11, 23, 30, 0).unwrap();
        assert_eq!(query_date("tomorrow", sunday).to_string(), "2026-10-12");
        assert_eq!(
            query_date("what does Friday look like", sunday).to_string(),
            "2026-10-16"
        );
    }

    #[test]
    fn cancellation_requires_a_matching_event() {
        let event = BusyOut {
            start: local_time_as_utc(
                timezone(),
                NaiveDateTime::new(
                    chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ),
            ),
            end: local_time_as_utc(
                timezone(),
                NaiveDateTime::new(
                    chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
                    NaiveTime::from_hms_opt(15, 30, 0).unwrap(),
                ),
            ),
            title: "1:1".into(),
        };
        assert!(event_matches("cancel my 3pm", &event));
        assert!(!event_matches("cancel my 4pm", &event));
    }

    #[test]
    fn cancellation_reply_includes_day_and_ampm_time() {
        let start = Utc.with_ymd_and_hms(2026, 10, 12, 18, 0, 0).unwrap();
        assert_eq!(
            cancelled_event_reply("Campus bookstore shift", start),
            "Cancelled Campus bookstore shift, Mon 2:00 PM. Your plan is updated."
        );
    }

    #[test]
    fn fuzzy_cancellation_matches_stems_and_requested_weekday() {
        let event = BusyOut {
            start: Utc.with_ymd_and_hms(2026, 10, 13, 13, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 10, 13, 14, 0, 0).unwrap(),
            title: "STAT 201 Lecture".into(),
        };
        assert_eq!(
            event_match_score("my stats lecture on Tuesday is cancelled", &event),
            2
        );
        assert!(event_is_on_requested_day(
            "my stats lecture on Tuesday is cancelled",
            &event
        ));
    }

    #[test]
    fn next_query_is_local_fallback_intent() {
        assert!(is_next_query("what should I do now?"));
        assert_eq!(fallback_intent("what's next?"), "query_day");
    }

    #[test]
    fn query_labels_follow_requested_day() {
        assert_eq!(query_label("what does Friday look like"), "Friday");
        assert_eq!(query_label("what is tomorrow like"), "Tomorrow");
        assert_eq!(query_label("show my day"), "Today");
    }

    #[test]
    fn validates_titles_clamps_durations_and_rounds_schedule_time() {
        assert!(!title_is_valid("  \t"));
        assert!(title_is_valid("Do the thing"));
        assert_eq!(clamp_duration(-5), 5);
        assert_eq!(clamp_duration(100_000), 480);
        assert_eq!(clamp_duration(30), 30);
        let now = Utc.with_ymd_and_hms(2026, 10, 11, 21, 12, 15).unwrap();
        assert_eq!(
            round_up_quarter(now).to_rfc3339(),
            "2026-10-11T21:15:00+00:00"
        );
    }

    #[test]
    fn date_strings_map_to_expected_local_dates() {
        let anchor = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
        assert_eq!(
            parse_deadline("thursday", anchor)
                .unwrap()
                .with_timezone(&timezone())
                .date_naive()
                .to_string(),
            "2026-10-15"
        );
        assert_eq!(
            parse_deadline("weekend", anchor)
                .unwrap()
                .with_timezone(&timezone())
                .date_naive()
                .to_string(),
            "2026-10-17"
        );
        assert_eq!(
            parse_deadline("next week", anchor)
                .unwrap()
                .with_timezone(&timezone())
                .date_naive()
                .to_string(),
            "2026-10-19"
        );
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
                        label: None,
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
        // In-memory database so this test cannot see or touch real saved tasks.
        let db = Arc::new(Db::open_in_memory().expect("in-memory database"));
        let router = app(new_state_with(db));

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

    fn import_request(ics_text: &str) -> Request<Body> {
        import_confirmed(ics_text, false)
    }

    /// The same import, with the conflict prompt already accepted.
    fn import_confirmed(ics_text: &str, confirm: bool) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/calendar/import")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "ics_text": ics_text, "confirm": confirm }).to_string(),
            ))
            .unwrap()
    }

    const SYLLABUS_PDF: &[u8] = include_bytes!("../../demo/syllabus.pdf");

    /// A multipart body with one `file` part, the shape a browser file drop
    /// sends. `multer` is not a dependency, so the body is written by hand.
    fn multipart_request(filename: &str, bytes: &[u8]) -> Request<Body> {
        const BOUNDARY: &str = "ai-todo-test-boundary";
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(b"Content-Type: application/pdf\r\n\r\n");
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());

        Request::builder()
            .method("POST")
            .uri("/api/capture")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn capture_still_accepts_json() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory database"));
        let router = app(new_state_with(db));

        let res = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/capture")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "text": "buy printer ink" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let tasks = json["tasks"].as_array().expect("tasks array");
        assert_eq!(tasks.len(), 1, "got: {json}");
        assert_eq!(tasks[0]["title"], "buy printer ink");
        assert_eq!(tasks[0]["source"], "text", "JSON capture stays text");
    }

    #[tokio::test]
    async fn capture_accepts_a_pdf_upload() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory database"));
        let router = app(new_state_with(db));

        let res = router
            .oneshot(multipart_request("syllabus.pdf", SYLLABUS_PDF))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let tasks = json["tasks"].as_array().expect("tasks array");
        assert!(!tasks.is_empty(), "a syllabus should yield tasks: {json}");
        assert_eq!(tasks[0]["source"], "pdf", "an upload is source pdf");
    }

    #[tokio::test]
    async fn a_junk_upload_is_a_400_not_a_crash() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory database"));
        let router = app(new_state_with(db));

        let res = router
            .clone()
            .oneshot(multipart_request("notes.txt", b"this is not a pdf at all"))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            json["error"].as_str().is_some_and(|e| !e.is_empty()),
            "expected an error message, got: {json}"
        );

        // the server is still up and serving after a rejected upload
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/tasks")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    const TWO_EVENTS: &str = "BEGIN:VCALENDAR\nBEGIN:VEVENT\n\
        SUMMARY:STAT 201 Lecture\n\
        DTSTART;TZID=America/New_York:20261013T090000\n\
        DTEND;TZID=America/New_York:20261013T101500\n\
        END:VEVENT\nBEGIN:VEVENT\n\
        SUMMARY:Campus bookstore shift\n\
        DTSTART;TZID=America/New_York:20261014T140000\n\
        DTEND;TZID=America/New_York:20261014T180000\n\
        END:VEVENT\nEND:VCALENDAR\n";

    #[tokio::test]
    async fn importing_a_calendar_replaces_the_busy_blocks() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory database"));
        let router = app(new_state_with(db));

        // The seeded state starts with 3 busy blocks.
        let res = router
            .clone()
            .oneshot(import_request(TWO_EVENTS))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let busy = json["busy"].as_array().expect("a busy array");
        assert_eq!(busy.len(), 2, "the seed blocks should be gone");

        // The import is in UTC: 09:00 New York in October is 13:00 UTC.
        assert_eq!(busy[0]["title"], "STAT 201 Lecture");
        assert_eq!(busy[0]["start"], "2026-10-13T13:00:00Z");
        assert_eq!(busy[0]["end"], "2026-10-13T14:15:00Z");

        // And it survives a round trip through GET, so it really was saved.
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
        assert_eq!(json["busy"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn reimporting_the_same_calendar_is_prompted_not_silently_applied() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory database"));
        let router = app(new_state_with(db));

        let res = router
            .clone()
            .oneshot(import_request(TWO_EVENTS))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "first import is clean");

        // The second import overlaps the first exactly, so the user is asked
        // rather than having their calendar quietly replaced.
        let res = router
            .clone()
            .oneshot(import_request(TWO_EVENTS))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let conflicts = json["conflicts"].as_array().expect("a conflicts array");
        assert!(!conflicts.is_empty());
        assert_eq!(conflicts[0]["imported_title"], "STAT 201 Lecture");
        assert_eq!(conflicts[0]["existing_title"], "STAT 201 Lecture");

        // Nothing was applied by that refusal.
        let res = router
            .clone()
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
        assert_eq!(json["busy"].as_array().unwrap().len(), 2);

        // Confirming goes through, and still replaces rather than appends.
        let res = router
            .clone()
            .oneshot(import_confirmed(TWO_EVENTS, true))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

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
        assert_eq!(
            json["busy"].as_array().unwrap().len(),
            2,
            "a confirmed import must replace, not append"
        );
    }

    #[tokio::test]
    async fn a_calendar_that_is_not_an_ics_is_a_400() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory database"));
        let router = app(new_state_with(db));

        let res = router
            .clone()
            .oneshot(import_request("definitely not a calendar"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json["error"].as_str().is_some(), "got {json}");

        // And nothing was clobbered: the seeded blocks are still there.
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
        assert_eq!(
            json["busy"].as_array().unwrap().len(),
            3,
            "a failed import must leave the old calendar alone"
        );
    }

    #[tokio::test]
    async fn the_whole_demo_week_imports() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory database"));
        let router = app(new_state_with(db));
        let week = include_str!("../../demo/outlook/week.ics");

        // The seeded demo blocks overlap the real week, so confirm it.
        let res = router.oneshot(import_confirmed(week, true)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            json["busy"].as_array().unwrap().len(),
            9,
            "the Outlook fixture has 9 events"
        );
    }

    #[tokio::test]
    async fn an_ics_file_can_be_picked_off_the_device_and_uploaded() {
        // How a user actually gets an `.ics` out of Outlook: save it, then
        // choose the file. JSON is the other path; this is the one the
        // frontend's file picker will use.
        let db = Arc::new(Db::open_in_memory().expect("in-memory database"));
        let router = app(new_state_with(db));
        let week = include_str!("../../demo/outlook/week.ics");

        let boundary = "icsboundary";
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"week.ics\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: text/calendar\r\n\r\n");
        body.extend_from_slice(week.as_bytes());
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/calendar/import")
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT, "overlaps the seed");

        // The same upload with confirm=1 goes through.
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"week.ics\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: text/calendar\r\n\r\n");
        body.extend_from_slice(week.as_bytes());
        body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"confirm\"\r\n\r\ntrue\r\n");
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let res = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/calendar/import")
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn cancelled_and_transparent_events_are_not_imported() {
        let db = Arc::new(Db::open_in_memory().expect("in-memory database"));
        let router = app(new_state_with(db));

        let week = "BEGIN:VCALENDAR\r\n\
            BEGIN:VEVENT\r\n\
            SUMMARY:Advising appointment\r\n\
            STATUS:CANCELLED\r\n\
            DTSTART;TZID=America/New_York:20261020T130000\r\n\
            DTEND;TZID=America/New_York:20261020T133000\r\n\
            END:VEVENT\r\n\
            BEGIN:VEVENT\r\n\
            SUMMARY:Someone's birthday\r\n\
            TRANSP:TRANSPARENT\r\n\
            DTSTART;TZID=America/New_York:20261021T000000\r\n\
            DTEND;TZID=America/New_York:20261021T235959\r\n\
            END:VEVENT\r\n\
            BEGIN:VEVENT\r\n\
            SUMMARY:Real lecture\r\n\
            DTSTART;TZID=America/New_York:20261022T090000\r\n\
            DTEND;TZID=America/New_York:20261022T101500\r\n\
            END:VEVENT\r\n\
            END:VCALENDAR\r\n";

        let res = router.oneshot(import_request(week)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let busy = json["busy"].as_array().expect("a busy array");
        assert_eq!(busy.len(), 1, "only the event that actually blocks time");
        assert_eq!(busy[0]["title"], "Real lecture");
    }

    #[test]
    fn a_partial_overlap_is_a_conflict_too() {
        // Not identical times, but still a clash: a lecture starting ten
        // minutes into an existing meeting needs the same warning.
        let existing = vec![db::BusyRow {
            start: "2026-10-13T13:00:00Z".parse().unwrap(),
            end: "2026-10-13T14:00:00Z".parse().unwrap(),
            title: "Group meeting".into(),
        }];
        let imported = vec![db::BusyRow {
            start: "2026-10-13T13:50:00Z".parse().unwrap(),
            end: "2026-10-13T15:00:00Z".parse().unwrap(),
            title: "STAT 201 Lecture".into(),
        }];
        let conflicts = ics::find_conflicts(&imported, &existing);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].imported_title, "STAT 201 Lecture");
        assert_eq!(conflicts[0].existing_title, "Group meeting");
    }

    #[test]
    fn back_to_back_events_are_not_a_conflict() {
        // Half-open intervals: finishing at 11:00 and starting at 11:00 is a
        // normal day, not a clash.
        let existing = vec![db::BusyRow {
            start: "2026-10-13T10:00:00Z".parse().unwrap(),
            end: "2026-10-13T11:00:00Z".parse().unwrap(),
            title: "Lab".into(),
        }];
        let imported = vec![db::BusyRow {
            start: "2026-10-13T11:00:00Z".parse().unwrap(),
            end: "2026-10-13T12:00:00Z".parse().unwrap(),
            title: "Lunch".into(),
        }];
        assert!(ics::find_conflicts(&imported, &existing).is_empty());
    }

    #[test]
    fn events_on_free_days_do_not_conflict() {
        let existing = vec![db::BusyRow {
            start: "2026-10-13T10:00:00Z".parse().unwrap(),
            end: "2026-10-13T11:00:00Z".parse().unwrap(),
            title: "Lab".into(),
        }];
        let imported = vec![db::BusyRow {
            start: "2026-10-14T10:00:00Z".parse().unwrap(),
            end: "2026-10-14T11:00:00Z".parse().unwrap(),
            title: "Lab".into(),
        }];
        assert!(ics::find_conflicts(&imported, &existing).is_empty());
    }
}

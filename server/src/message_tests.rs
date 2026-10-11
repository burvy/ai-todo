//! Tests for the conversational `/api/message` replies.

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::db::Db;
use crate::{app, day_phrase, new_state_with, normalized_title};

async fn say(router: &axum::Router, text: &str) -> serde_json::Value {
    let res = router
        .clone()
        .oneshot(
            Request::post("/api/message")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "text": text }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn router() -> axum::Router {
    app(new_state_with(Arc::new(Db::open_in_memory().unwrap())))
}

#[tokio::test]
async fn saying_the_same_thing_twice_does_not_duplicate() {
    let router = router();
    let first = say(&router, "I need to buy printer ink").await;
    assert_eq!(first["intent"], "add_tasks");
    let count = first["tasks"].as_array().unwrap().len();
    assert!(count >= 1);

    let second = say(&router, "I need to buy printer ink.").await;
    assert_eq!(second["tasks"].as_array().unwrap().len(), count);
    assert!(
        second["reply"]
            .as_str()
            .unwrap()
            .contains("already on your list"),
        "{}",
        second["reply"]
    );
}

#[tokio::test]
async fn finishing_a_finished_task_says_already_done() {
    let router = router();
    say(&router, "I need to buy printer ink").await;
    let done = say(&router, "I finished buying printer ink").await;
    assert!(
        done["reply"].as_str().unwrap().starts_with("Done:"),
        "{}",
        done["reply"]
    );
    let again = say(&router, "I finished buying printer ink").await;
    assert!(
        again["reply"]
            .as_str()
            .unwrap()
            .starts_with("Already done:"),
        "{}",
        again["reply"]
    );
}

#[tokio::test]
async fn past_days_are_not_answered_as_today() {
    let router = router();
    let res = say(&router, "what does yesterday look like").await;
    assert_eq!(res["reply"], "That day has passed.");
}

#[test]
fn titles_normalise_for_duplicate_checks() {
    assert_eq!(normalized_title("Buy  Milk."), normalized_title("buy milk"));
    assert_ne!(normalized_title("Buy milk"), normalized_title("Buy eggs"));
}

#[test]
fn day_phrase_names_the_day() {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 11).unwrap();
    assert_eq!(day_phrase(today, today), "today");
    assert_eq!(
        day_phrase(today + chrono::Duration::days(1), today),
        "tomorrow"
    );
    assert_eq!(
        day_phrase(today + chrono::Duration::days(2), today),
        "on Tuesday"
    );
}

async fn call(
    router: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (axum::http::StatusCode, serde_json::Value) {
    let mut request = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(value) => {
            request = request.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let res = router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

#[tokio::test]
async fn calendar_follows_tasks_without_calling_schedule() {
    let router = router();
    let (_, task) = call(
        &router,
        "POST",
        "/api/tasks",
        Some(serde_json::json!({ "title": "Write the report", "duration_min": 30 })),
    )
    .await;
    let id = task["id"].as_str().unwrap().to_string();

    let (_, cal) = call(&router, "GET", "/api/calendar", None).await;
    let placed = |cal: &serde_json::Value, id: &str| {
        cal["placements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["task_id"] == id)
    };
    assert!(
        placed(&cal, &id),
        "new task should be on the calendar: {cal}"
    );

    let (_, _) = call(
        &router,
        "PATCH",
        &format!("/api/tasks/{id}"),
        Some(serde_json::json!({ "status": "done" })),
    )
    .await;
    let (_, cal) = call(&router, "GET", "/api/calendar", None).await;
    assert!(!placed(&cal, &id), "done task should leave the calendar");

    let (_, task) = call(
        &router,
        "POST",
        "/api/tasks",
        Some(serde_json::json!({ "title": "Call the bank", "duration_min": 15 })),
    )
    .await;
    let id = task["id"].as_str().unwrap().to_string();
    let (_, cal) = call(&router, "GET", "/api/calendar", None).await;
    assert!(placed(&cal, &id));
    let (status, _) = call(&router, "DELETE", &format!("/api/tasks/{id}"), None).await;
    assert_eq!(status, axum::http::StatusCode::NO_CONTENT);
    let (_, cal) = call(&router, "GET", "/api/calendar", None).await;
    assert!(!placed(&cal, &id), "deleted task should leave the calendar");
}

#[tokio::test]
async fn export_is_in_sync_right_after_a_mutation() {
    let router = router();
    call(
        &router,
        "POST",
        "/api/tasks",
        Some(serde_json::json!({ "title": "Pay rent", "duration_min": 15 })),
    )
    .await;
    let res = router
        .clone()
        .oneshot(
            Request::get("/api/calendar.ics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("SUMMARY:Pay rent"));
}

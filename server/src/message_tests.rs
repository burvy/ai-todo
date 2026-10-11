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

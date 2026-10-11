//! `POST /api/reset`: wipe tasks, placements and busy blocks, in memory and in
//! SQLite, so the demo can be restarted from a clean slate.

use axum::{extract::State, http::StatusCode};

use crate::AppState;

pub async fn reset(State(state): State<AppState>) -> StatusCode {
    {
        let mut store = state.store.lock().unwrap();
        store.tasks.clear();
        store.placements.clear();
        store.next_id = 0;
    }
    state.busy.lock().unwrap().clear();
    state.db.clear();
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::db::Db;
    use crate::{app, new_state_with};

    #[tokio::test]
    async fn reset_clears_memory_and_database() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        let state = new_state_with(db.clone());
        assert!(
            state.busy.lock().unwrap().is_empty(),
            "new calendars start empty"
        );
        let created = app(state.clone())
            .oneshot(
                Request::post("/api/tasks")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"title":"x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(created.status().is_success(), "{}", created.status());
        assert_eq!(db.load_tasks().unwrap().len(), 1);

        let res = app(state.clone())
            .oneshot(Request::post("/api/reset").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        assert!(state.store.lock().unwrap().tasks.is_empty());
        assert!(state.store.lock().unwrap().placements.is_empty());
        assert!(state.busy.lock().unwrap().is_empty());
        assert!(db.load_tasks().unwrap().is_empty());
        assert!(db.load_busy().unwrap().is_empty());
    }
}

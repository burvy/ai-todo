//! Tasks, series, occurrences, settings, order and import: the routes that read and write the user's plan.

use super::*;

/// `GET /api/v1/snapshot?from=YYYY-MM-DD&days=N`: the plan window, today and 7 days
/// by default. The task list is always the whole list.
pub(super) async fn snapshot(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(window): Query<SnapshotWindow>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let app = shared.app.lock_recover();
    let today = app.now().with_timezone(&tz).date_naive();
    let from = match window.from {
        Some(from) => from,
        None => today,
    };
    let days = window.days.unwrap_or(DEFAULT_WINDOW_DAYS);
    if days == 0 || days > MAX_WINDOW_DAYS {
        return Err(bad_request(format!(
            "days must be between 1 and {MAX_WINDOW_DAYS}"
        )));
    }
    // A four-digit year keeps every step of the window far from chrono's limits.
    if !(1..=9999).contains(&from.year()) {
        return Err(bad_request("from must be a date between years 1 and 9999"));
    }
    Ok(Json(app.snapshot_in_window(&user, tz, from, days)?))
}

/// The `?from=`/`?days=` plan window.
#[derive(Default, Deserialize)]
pub(super) struct SnapshotWindow {
    from: Option<NaiveDate>,
    days: Option<u32>,
}

pub(super) async fn create(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(draft): Json<TaskDraft>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.create(draft, &user, tz)?))
}

/// The signed-in user's day-only times: when "by Friday" is due, and where on its day
/// a task with no time goes when they have no habits to learn from.
pub(super) async fn get_settings(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<Json<DaySettings>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let app = shared.app.lock_recover();
    Ok(Json(app.day_settings(&user)?))
}

/// Changes the fields sent (`deadline`, `placement`, each `HH:MM`) and answers with
/// both as they now stand. The next snapshot places tasks by them.
pub(super) async fn patch_settings(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(patch): Json<app::DaySettingsPatch>,
) -> Result<Json<DaySettings>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.patch_day_settings(&user, patch)?))
}

pub(super) async fn update(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(patch): Json<Patch>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.patch(&id, patch, &user, tz)?))
}

/// `POST /api/v1/tasks/restore`: a task the user just deleted, back as it was.
pub(super) async fn restore(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(task): Json<Task>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.restore(task, &user, tz)?))
}

pub(super) async fn remove(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.delete(&id, &user, tz)?))
}

/// `POST /api/v1/series`: a weekly series (ARCHITECTURE.md 7.1).
pub(super) async fn create_series(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(draft): Json<app::SeriesDraft>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.create_series(draft, &user, tz)?))
}

/// `PATCH /api/v1/series/{id}`: edit the rule every occurrence follows.
pub(super) async fn update_series(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(patch): Json<app::SeriesPatch>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.update_series(&id, patch, &user, tz)?))
}

/// `DELETE /api/v1/series/{id}`: the whole series, exceptions with it.
pub(super) async fn remove_series(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.delete_series(&id, &user, tz)?))
}

/// `PATCH /api/v1/series/{id}/dates/{date}`: done / skipped / moved for one
/// occurrence. `{date}` is `YYYY-MM-DD`, the occurrence's local date.
pub(super) async fn patch_occurrence(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path((id, date)): axum::extract::Path<(String, NaiveDate)>,
    Json(body): Json<app::OccurrencePatch>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.patch_occurrence(&id, date, body, &user, tz)?))
}

/// `DELETE /api/v1/series/{id}/dates/{date}`: clear one occurrence's
/// exception, so the day falls back to the rule.
pub(super) async fn clear_occurrence(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path((id, date)): axum::extract::Path<(String, NaiveDate)>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.clear_occurrence(&id, date, &user, tz)?))
}

/// `POST /api/v1/import`: either a multipart `file` with an .ics calendar or a
/// JSON body matching `GET /api/v1/export` (`{ "tasks": [...] }`).
///
/// All imported tasks get fresh IDs and belong to the caller. Bad JSON or an
/// invalid task returns 400 and writes nothing (all or nothing).
pub(super) async fn import(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    request: axum::extract::Request,
) -> Result<Json<ImportResult>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;

    // Decide based on Content-Type.
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if ct.starts_with("multipart/form-data") {
        // Existing .ics import path.
        let mut multipart = Multipart::from_request(request, &())
            .await
            .map_err(bad_request)?;
        let mut ics_text = None;
        while let Some(field) = multipart.next_field().await.map_err(bad_request)? {
            if field.name() == Some("file") {
                ics_text = Some(field.text().await.map_err(bad_request)?);
            }
        }
        let ics_text =
            ics_text.ok_or_else(|| bad_request("send the calendar as multipart field `file`"))?;
        let calendar =
            ics_import::parse_calendar(&ics_text).map_err(|error| bad_request(error.0))?;
        let mut app = shared.app.lock_recover();
        return Ok(Json(app.import(calendar, &user, tz)?));
    }

    // JSON import path: same shape as GET /api/v1/export.
    let body = request.into_body();
    let bytes = axum::body::to_bytes(body, IMPORT_MAX_BODY_BYTES)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    if bytes.len() > IMPORT_MAX_BODY_BYTES {
        return Err(bad_request(format!(
            "request body too large (max {} bytes)",
            IMPORT_MAX_BODY_BYTES
        )));
    }

    #[derive(Deserialize)]
    struct ImportIn {
        tasks: Vec<Task>,
    }

    let import_in: ImportIn =
        serde_json::from_slice(&bytes).map_err(|e| bad_request(format!("invalid JSON: {}", e)))?;

    if import_in.tasks.is_empty() {
        return Err(bad_request("no tasks to import"));
    }
    if import_in.tasks.len() > IMPORT_MAX_TASKS {
        return Err(bad_request(format!(
            "too many tasks (max {})",
            IMPORT_MAX_TASKS
        )));
    }

    // Validate all tasks before writing any (all or nothing).
    // The same limits as any task, so an export always imports again.
    for (i, task) in import_in.tasks.iter().enumerate() {
        app::check_title(&task.title)
            .and_then(|()| app::check_duration(task.duration_min))
            .map_err(|error| bad_request(format!("task {i}: {}", error.message)))?;
    }

    // All valid — now write them, under one lock so a second import cannot pass the
    // room check against the same count or take the same orders.
    let app = shared.app.lock_recover();
    let now = app.now();
    let tasks = app.store().tasks(&user).map_err(store_failed)?;
    app::check_room(tasks.len(), import_in.tasks.len())?;
    let min_order = tasks.iter().map(|t| t.order).min().unwrap_or(0);

    let mut created = Vec::new();
    for (offset, mut task) in import_in.tasks.into_iter().enumerate() {
        // Assign fresh ID — never trust the file's id.
        let new_id = TaskId(format!("t{}", Uuid::new_v4().simple()));
        task.id = new_id.clone();
        task.created = now;
        task.updated_at = now;
        // Newest-first ordering like new tasks.
        task.order = min_order - 1 - offset as i64;
        // Clear external_uid — this is not a calendar import.
        task.external_uid = None;

        app.store().put(&user, task).map_err(|e| {
            HttpError(ApiError {
                code: ErrorCode::Internal,
                message: format!("store error: {}", e),
            })
        })?;
        created.push(new_id);
    }

    let snapshot = app.snapshot(&user, tz)?;
    Ok(Json(ImportResult {
        created,
        updated: Vec::new(),
        series: Vec::new(),
        unmapped: Vec::new(),
        snapshot,
    }))
}

#[derive(Deserialize)]
pub(super) struct OrderIn {
    ids: Vec<String>,
}

pub(super) async fn reorder(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<OrderIn>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.reorder(&body.ids, &user, tz)?))
}

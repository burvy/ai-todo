//! The calendar feed and the full export.

use super::*;

/// `GET /api/v1/calendar.ics`: the plan as an iCalendar file.
pub(super) async fn calendar_ics(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<([(HeaderName, &'static str); 1], String), HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let snapshot = shared.app.lock_recover().snapshot(&user, tz)?;
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/calendar; charset=utf-8",
        )],
        build_ics(&snapshot),
    ))
}

#[derive(Serialize)]
pub(super) struct ExportOut {
    tasks: Vec<Task>,
}

/// `GET /api/v1/export`: everything this caller owns as one JSON download.
///
/// Tasks are all the server stores of a person — the planner's settings live in
/// the browser — so the file is `{ "tasks": [...] }`, shaped so a server-side
/// settings section can join it later without breaking readers.
///
/// A caller with no credentials is still a caller: the anonymous-id middleware
/// hands out an id before this runs, so the download is 200 and an empty list.
/// That is how the rest of the API treats a signed-out visitor, and the file
/// holds their tasks from then on.
pub(super) async fn export(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let app = shared.app.lock_recover();
    // `App::tasks`, not `snapshot`: a download has no plan in it, and planning
    // a list twice the size of what is written out is work nobody asked for.
    let tasks = app.tasks(&user)?;
    let today = app.now().with_timezone(&tz).date_naive();
    let disposition = HeaderValue::from_str(&format!(
        "attachment; filename=\"rambl-export-{today}.json\""
    ))
    .expect("a date and fixed words are valid header text");
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (axum::http::header::CONTENT_DISPOSITION, disposition),
            // A person's whole task list, on a URL any browser can be sent back
            // to. Do not let a cache keep it.
            (
                axum::http::header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            ),
        ],
        Json(ExportOut { tasks }),
    )
        .into_response())
}

//! Capture a ramble into tasks, and revise one task with words.

use super::*;

#[derive(Deserialize)]
pub(super) struct CaptureIn {
    text: String,
}

#[derive(Serialize)]
pub(super) struct CaptureOut {
    created: Vec<TaskId>,
    snapshot: Snapshot,
    /// True when the user's daily Jev cap was spent and `FakeJudge` judged instead.
    jev_fallback: bool,
}

/// `POST /api/v1/capture`: JSON `{text}`, or multipart `file` (.pdf, .eml or text).
///
/// Past the user's daily cap, or from a blocked user or address, the capture is still
/// accepted but judged by [`judge::FakeJudge`] instead of Jev, and `jev_fallback` says
/// so. Failing the request would lose the capture; spending the owner's Jev allowance on
/// one account would take it from everyone.
pub(super) async fn capture(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    request: axum::extract::Request,
) -> Result<Json<CaptureOut>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let ip = client_ip(
        request.headers(),
        peer_ip(&request),
        &shared.trusted_proxies,
    );
    let tz = zone(request.headers())?;
    let (text, source, anchor) = read_capture(request).await?;
    check_capture_size(&text)?;
    let today = shared.app.lock_recover().now();
    // An email's send date anchors its words ("tomorrow"), but the daily caps count on
    // the server's day: a sender can write any date, and a far one would reset them.
    let ctx = LocalContext { now: today, tz };
    let ctx = LocalContext {
        now: anchor.map_or(today, |sent| sent.instant(&ctx)),
        ..ctx
    };
    let plan = if headers.contains_key(axum::http::header::AUTHORIZATION) {
        let app = shared.app.lock_recover();
        app.store().user_plan(&user).map_err(store_failed)?
    } else {
        auth::Plan::Anon
    };
    let jev_fallback = {
        let app = shared.app.lock_recover();
        let store = app.store();
        let day = today.date_naive();
        let blocked = store.is_blocked(&user, &ip).map_err(store_failed)?;
        let used = store.captures_today(&user, day).map_err(store_failed)?;
        let ip_used = store.ip_captures_today(&ip, day).map_err(store_failed)?;
        let cap = shared.caps.for_plan(plan);
        let fallback = blocked || used >= cap || ip_used >= cap;
        store
            .record_capture(&user, &ip, day)
            .map_err(store_failed)?;
        fallback
    };
    let deadline = shared.app.lock_recover().day_settings(&user)?.deadline;
    let drafts = if jev_fallback {
        capture_text(&judge::FakeJudge, &text, source, deadline, &ctx)
            .await
            .map_err(upstream)?
    } else {
        capture_text(&shared.judge, &text, source, deadline, &ctx)
            .await
            .map_err(upstream)?
    };
    let mut app = shared.app.lock_recover();
    let (created, snapshot) = app.add_drafts(drafts, &user, tz)?;
    Ok(Json(CaptureOut {
        created,
        snapshot,
        jev_fallback,
    }))
}

/// Refuse a capture too big to judge fairly: the judge is asked once per clause, all at
/// once, so one huge text would spend far more Jev than the daily cap counts.
pub(super) fn check_capture_size(text: &str) -> Result<(), HttpError> {
    if text.chars().count() > CAPTURE_MAX_CHARS {
        return Err(bad_request(format!(
            "that is too long to capture at once (max {CAPTURE_MAX_CHARS} characters)"
        )));
    }
    if capture::split_clauses(text).len() > CAPTURE_MAX_CLAUSES {
        return Err(bad_request(format!(
            "that has too many separate tasks to capture at once (max {CAPTURE_MAX_CLAUSES})"
        )));
    }
    Ok(())
}

/// The capture text, its source, and an anchor time when the source has a send date.
///
/// Only `.pdf`, `.eml` and `.txt` are accepted. Anything else used to fall through to
/// "treat the bytes as text", which quietly accepted a `.docx` and handed the judge
/// binary noise.
pub(super) async fn read_capture(
    request: axum::extract::Request,
) -> Result<(String, Source, Option<capture::Sent>), HttpError> {
    let multipart = request
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("multipart/form-data"));
    if multipart {
        let mut multipart = Multipart::from_request(request, &())
            .await
            .map_err(bad_request)?;
        while let Some(field) = multipart.next_field().await.map_err(bad_request)? {
            if field.name() == Some("file") {
                let name = field.file_name().unwrap_or_default().to_ascii_lowercase();
                let bytes = field.bytes().await.map_err(bad_request)?;
                return file_text(&name, &bytes);
            }
        }
        return Err(bad_request("send the document as multipart field `file`"));
    }
    let Json(body) = Json::<CaptureIn>::from_request(request, &())
        .await
        .map_err(bad_request)?;
    if body.text.trim().is_empty() {
        return Err(bad_request("text is required"));
    }
    Ok((body.text, Source::Text, None))
}

pub(super) fn file_text(
    name: &str,
    bytes: &[u8],
) -> Result<(String, Source, Option<capture::Sent>), HttpError> {
    if name.ends_with(".pdf") {
        let text = capture::pdf_to_text(bytes).map_err(|capture::PdfError::Unreadable(why)| {
            bad_request(format!("could not read that PDF: {why}"))
        })?;
        return Ok((text, Source::Pdf, None));
    }
    if !(name.ends_with(".eml") || name.ends_with(".txt")) {
        return Err(bad_request("upload a .pdf, .eml or .txt file"));
    }
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_| bad_request("that file is not UTF-8 text"))?;
    if name.ends_with(".eml") {
        let email = capture::parse_email(&text);
        let body = format!("{}\n{}", email.subject, email.body);
        return Ok((body, Source::Email, email.sent));
    }
    Ok((text, Source::Text, None))
}

#[derive(Deserialize)]
pub(super) struct ReviseIn {
    instruction: String,
    /// The task's `updated_at` as the client last saw it; see [`Patch::updated_at`].
    updated_at: Option<chrono::DateTime<Utc>>,
}

/// `POST /api/v1/tasks/{id}/revise`: one sentence re-classifies the task.
pub(super) async fn revise_task(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<ReviseIn>,
) -> Result<Json<todo_core::Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let instruction = body.instruction.trim().to_owned();
    if instruction.is_empty() {
        return Err(bad_request("instruction is required"));
    }
    let (title, now) = {
        let app = shared.app.lock_recover();
        (app.title_of(&user, &id)?, app.now())
    };
    let ctx = LocalContext { now, tz };
    let revision: Revision = capture::revise(&shared.judge, &title, &instruction, &ctx)
        .await
        .map_err(upstream)?;
    let mut app = shared.app.lock_recover();
    Ok(Json(app.apply_revision(
        &id,
        &revision,
        body.updated_at,
        &user,
        tz,
    )?))
}

//! The admin page and its usage, block and plan routes.

use super::*;

/// A request is an admin when it carries a live signed-in session whose email is
/// in `RAMBL_ADMIN_EMAILS`. Anyone else sees 404, so the admin surface does not
/// announce itself.
pub(super) fn admin_of(headers: &HeaderMap, shared: &Shared) -> Result<auth::Me, HttpError> {
    let app = shared.app.lock_recover();
    // A missing or expired session is a 404 here; a failing store is still a failure.
    let user = signed_in(headers, &app).map_err(|error| match error.0.code {
        ErrorCode::Unauthorized => admin_404(),
        _ => error,
    })?;
    let allowed = shared.admin_emails.as_deref().is_some_and(|emails| {
        emails
            .iter()
            .any(|email| *email == user.email.to_ascii_lowercase())
    });
    if !allowed {
        return Err(admin_404());
    }
    Ok(user)
}

pub(super) fn admin_404() -> HttpError {
    HttpError(ApiError {
        code: ErrorCode::NotFound,
        message: "no such page".into(),
    })
}

/// `GET /admin`: the administration page. Off when no emails are configured. A
/// signed-in request (a Bearer token) that is not an admin gets 404; a plain
/// navigation carries no token, so it gets the shell, which signs in the same way
/// the main page does — only a listed email ever sees data.
pub(super) async fn admin_page(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<([(HeaderName, String); 1], Html<&'static str>), HttpError> {
    if shared.admin_emails.is_none() {
        return Err(admin_404());
    }
    if auth::bearer(&headers).is_some() {
        admin_of(&headers, &shared)?;
    }
    Ok((
        [(
            axum::http::header::CONTENT_SECURITY_POLICY,
            super::super::csp::for_html(admin_web::PAGE),
        )],
        Html(admin_web::PAGE),
    ))
}

/// `GET /api/v1/admin/usage`: today's captures by user (with plan) and address,
/// and the totals per plan. One capture is one Jev call.
pub(super) async fn admin_usage(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<Json<admin_web::UsageOut>, HttpError> {
    admin_of(&headers, &shared)?;
    let app = shared.app.lock_recover();
    Ok(Json(
        admin_web::usage(app.store(), app.now().date_naive()).map_err(store_failed)?,
    ))
}

/// `GET /api/v1/admin/blocks`: everything currently blocked.
pub(super) async fn admin_blocks(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<Json<Vec<admin_web::BlockOut>>, HttpError> {
    admin_of(&headers, &shared)?;
    let app = shared.app.lock_recover();
    Ok(Json(admin_web::blocks(app.store()).map_err(store_failed)?))
}

#[derive(Deserialize)]
pub(super) struct BlockIn {
    kind: String,
    value: String,
}

#[derive(Deserialize)]
pub(super) struct PlanIn {
    who: String,
    plan: String,
}

#[derive(Serialize)]
pub(super) struct AdminOut {
    message: String,
}

/// `POST /api/v1/admin/block` `{kind: "user" | "ip", value}`: block a user (by
/// id or email) or an address.
pub(super) async fn admin_block(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<BlockIn>,
) -> Result<Json<AdminOut>, HttpError> {
    admin_of(&headers, &shared)?;
    let app = shared.app.lock_recover();
    let message = admin_web::block(app.store(), &body.kind, &body.value).map_err(bad_request)?;
    Ok(Json(AdminOut { message }))
}

/// `POST /api/v1/admin/unblock` `{kind, value}`: lift a block placed the same way.
pub(super) async fn admin_unblock(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<BlockIn>,
) -> Result<Json<AdminOut>, HttpError> {
    admin_of(&headers, &shared)?;
    let app = shared.app.lock_recover();
    let message = admin_web::unblock(app.store(), &body.kind, &body.value).map_err(bad_request)?;
    Ok(Json(AdminOut { message }))
}

/// `POST /api/v1/admin/plan` `{who: email|id, plan}`: move a user to another plan.
pub(super) async fn admin_set_plan(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<PlanIn>,
) -> Result<Json<AdminOut>, HttpError> {
    admin_of(&headers, &shared)?;
    let app = shared.app.lock_recover();
    let message = admin_web::set_plan(app.store(), &body.who, &body.plan).map_err(bad_request)?;
    Ok(Json(AdminOut { message }))
}

//! Sign-in, sign-out and the signed-in user's own account routes.

use super::*;

/// `GET /api/v1/config`: what the page needs before anyone signs in. The client id is
/// public: Google shows it on its own sign-in page.
pub(super) async fn config(
    State(shared): State<Arc<Shared>>,
    request: axum::extract::Request,
) -> Json<ConfigOut> {
    Json(ConfigOut {
        google_client_id: shared.google_client_id.clone(),
        // Offered only to a caller who could use it, so the tunnel never shows the button.
        local_login: shared.local_login
            && local_login_allowed(request.headers(), peer_ip(&request)),
    })
}

#[derive(Serialize)]
pub(super) struct ConfigOut {
    google_client_id: Option<String>,
    local_login: bool,
}

/// `POST /api/v1/auth/local`: the test-site sign-in as one fixed local user. Absent
/// unless `RAMBL_LOCAL_LOGIN=1`, and then refused for anything but a near peer.
pub(super) async fn sign_in_local(
    State(shared): State<Arc<Shared>>,
    request: axum::extract::Request,
) -> Result<Json<auth::Session>, HttpError> {
    if !shared.local_login || !local_login_allowed(request.headers(), peer_ip(&request)) {
        return Err(HttpError(ApiError {
            code: ErrorCode::Forbidden,
            message: "local sign-in is not available here".into(),
        }));
    }
    let anonymous = request
        .headers()
        .get(&ANON_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_anon_id(value))
        .map(str::to_owned);
    let app = shared.app.lock_recover();
    let session = auth::open_session(
        app.store(),
        &auth::TokenInfo::local_login(),
        app.now(),
        anonymous.as_deref(),
    )
    .map_err(HttpError::from)?;
    Ok(Json(session))
}

/// `POST /api/v1/auth/google`: trade an ID token for a session.
pub(super) async fn sign_in_google(
    State(shared): State<Arc<Shared>>,
    request: axum::extract::Request,
) -> Result<Json<auth::Session>, HttpError> {
    let anonymous = request
        .headers()
        .get(&ANON_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_anon_id(value))
        .map(str::to_owned);
    let ip = client_ip(
        request.headers(),
        peer_ip(&request),
        &shared.trusted_proxies,
    );
    check_sign_in_allowance(&shared, &ip)?;
    let Json(body): Json<SignInIn> = Json::from_request(request, &())
        .await
        .map_err(bad_request)?;
    // Verify before taking the lock. Holding a MutexGuard across an await makes the
    // handler's future non-Send, and axum will not accept a non-Send handler.
    let info = shared
        .verifier
        .verify(&body.credential)
        .await
        .map_err(HttpError::from)?;
    let app = shared.app.lock_recover();
    let session = auth::open_session(app.store(), &info, app.now(), anonymous.as_deref())
        .map_err(HttpError::from)?;
    Ok(Json(session))
}

/// Count this attempt against the address's daily allowance, before Google is asked.
///
/// Kept in the same table as the anonymous-id allowance under a `signin:` prefix, so
/// it survives a restart and is pruned with the rest of the day counts.
pub(super) fn check_sign_in_allowance(shared: &Shared, ip: &str) -> Result<(), HttpError> {
    let app = shared.app.lock_recover();
    let attempts = app
        .store()
        .record_anon_issued(&format!("signin:{ip}"), app.now().date_naive())
        .map_err(store_failed)?;
    if attempts > shared.caps.signins_per_ip {
        return Err(unauthorized(
            "too many sign-in attempts from this address today; try again tomorrow",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
pub(super) struct SignInIn {
    credential: String,
}

/// `POST /api/v1/auth/signout`: end this session only.
pub(super) async fn sign_out(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<StatusCode, HttpError> {
    if !shared.local {
        let token = auth::bearer(&headers)
            .ok_or_else(|| unauthorized("send Authorization: Bearer <token>"))?;
        let app = shared.app.lock_recover();
        auth::sign_out(app.store(), &token).map_err(HttpError::from)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/me`: who this request is.
pub(super) async fn me(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<Json<auth::Me>, HttpError> {
    if shared.local {
        // Resolve anyway, so local mode fails the same way as a real deployment if
        // the local user has somehow gone missing.
        CurrentUser::resolve(&headers, &shared)?;
        return Ok(Json(auth::Me {
            id: auth::LOCAL_USER.into(),
            email: "local@localhost".into(),
            name: "Local user".into(),
            picture: None,
            plan: auth::Plan::Free,
        }));
    }
    let app = shared.app.lock_recover();
    Ok(Json(signed_in(&headers, &app)?))
}

/// The user behind the request's bearer token, while their session lasts.
pub(super) fn signed_in(
    headers: &HeaderMap,
    app: &App<Box<dyn Store>>,
) -> Result<auth::Me, HttpError> {
    let token =
        auth::bearer(headers).ok_or_else(|| unauthorized("send Authorization: Bearer <token>"))?;
    auth::session_user(app.store(), &token, app.now())?
        .ok_or_else(|| unauthorized("that session has expired"))
}

/// `DELETE /api/v1/me`: forget this person, their sessions and all their tasks.
pub(super) async fn delete_me(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<StatusCode, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let app = shared.app.lock_recover();
    auth::delete_user(app.store(), &user).map_err(HttpError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

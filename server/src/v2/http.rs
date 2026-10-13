//! Thin HTTP handlers for `/api/v1`. They read the time zone header, lock the `App`
//! for the whole call, and map errors. No business rules live here.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{ConnectInfo, FromRequest, Multipart, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header::HeaderName},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, patch, post, put},
};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use todo_core::{ApiError, ErrorCode, Snapshot, TaskDraft};

use super::admin_web;
use super::app::{self, App, ImportResult, Patch};
use super::auth::{self, TokenVerifier};
use super::ics_export::build_ics;
use super::ics_import;
use super::store::{Store, StoreError};
use capture::{Revision, capture_text};
use chrono::{DateTime, NaiveDate, Utc};
use judge::AnyJudge;
use todo_core::{LocalContext, Source, TaskId};

const TZ_HEADER: HeaderName = HeaderName::from_static("x-rambl-tz");
const ANON_HEADER: HeaderName = HeaderName::from_static("x-rambl-anon");
/// The state every handler shares: the app (locked for each call), the judge, and how
/// a request is turned into a user.
pub struct Shared {
    app: Mutex<App<Box<dyn Store>>>,
    judge: AnyJudge,
    /// Checks a Google ID token. A trait so the tests never reach the network.
    verifier: Box<dyn TokenVerifier>,
    /// `None` when `GOOGLE_CLIENT_ID` is unset: local mode, one fixed user, no
    /// sign-in.
    local: bool,
    caps: Caps,
    /// The Google client id the page signs in with; `None` when sign-in is off.
    google_client_id: Option<String>,
    /// Emails (lower-cased) allowed to use `/admin`; `None` turns the page off.
    admin_emails: Option<Vec<String>>,
    /// The peers allowed to name the client with `CF-Connecting-IP`; see
    /// [`TrustedProxies`].
    trusted_proxies: TrustedProxies,
}

/// Daily limits. Past a Jev cap, capture still works but is judged by `FakeJudge`.
/// Past the anonymous-id limit, a client address gets no new anonymous id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub anon: u32,
    pub free: u32,
    pub paid: u32,
    /// New anonymous ids one client address may be issued per day.
    pub anon_ids_per_ip: u32,
}

impl Caps {
    /// The defaults, each overridable by its `RAMBL_...` variable.
    fn from_env() -> Self {
        Self {
            anon: env_number("RAMBL_JEV_DAILY_CAP_ANON", DEFAULT_ANON_JEV_DAILY_CAP),
            free: env_number("RAMBL_JEV_DAILY_CAP", DEFAULT_JEV_DAILY_CAP),
            paid: env_number("RAMBL_JEV_DAILY_CAP_PAID", DEFAULT_PAID_JEV_DAILY_CAP),
            anon_ids_per_ip: env_number("RAMBL_ANON_IDS_PER_IP", DEFAULT_ANON_IDS_PER_IP),
        }
    }

    fn for_plan(&self, plan: auth::Plan) -> u32 {
        match plan {
            auth::Plan::Anon => self.anon,
            auth::Plan::Free => self.free,
            auth::Plan::Paid => self.paid,
        }
    }
}

fn env_number(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(default)
}

/// The signed-in user.
///
/// In local mode (`GOOGLE_CLIENT_ID` unset) every request runs as the fixed local user
/// so `run-local.sh` and the tests need no sign-in. Otherwise a request with an
/// `Authorization` header must carry a live session, and one without must carry an
/// anonymous id that the server issued (see [`assign_anonymous_id`]).
pub struct CurrentUser;

impl CurrentUser {
    fn resolve(headers: &HeaderMap, shared: &Shared) -> Result<String, HttpError> {
        if shared.local {
            return Ok(auth::LOCAL_USER.to_owned());
        }
        // A bearer token is the only identity a signed-in client can present. A bad one
        // is refused outright, never quietly replaced by the anonymous id.
        if headers.contains_key(axum::http::header::AUTHORIZATION) {
            let token = auth::bearer(headers)
                .ok_or_else(|| unauthorized("send Authorization: Bearer <token>"))?;
            let app = shared.app.lock().expect("app mutex");
            return auth::session_user(app.store(), &token, Utc::now())
                .map_err(HttpError::from)?
                .map(|user| user.id)
                .ok_or_else(|| unauthorized("that session has expired"));
        }
        let anonymous = headers
            .get(&ANON_HEADER)
            .and_then(|value| value.to_str().ok())
            .filter(|value| valid_anon_id(value));
        let app = shared.app.lock().expect("app mutex");
        match anonymous {
            Some(id) if app.store().anonymous_exists(id).map_err(store_failed)? => {
                // A visit keeps the id alive; see `Store::touch_anonymous`.
                app.store()
                    .touch_anonymous(id, app.now())
                    .map_err(store_failed)?;
                Ok(id.to_owned())
            }
            _ => Err(unauthorized("send Authorization: Bearer <token>")),
        }
    }
}

fn unauthorized(message: &'static str) -> HttpError {
    HttpError(ApiError {
        code: ErrorCode::Unauthorized,
        message: message.into(),
    })
}

/// An error response: `{ "error": { "code", "message" } }` with a status for the code.
pub struct HttpError(ApiError);

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            ErrorCode::BadRequest => StatusCode::BAD_REQUEST,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::Conflict | ErrorCode::StaleUpdate => StatusCode::CONFLICT,
            ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            // A dependency being down is 503, "come back later"; our own
            // failure is 500. The two are different things to a client, which
            // is why they are different codes.
            ErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Upstream => StatusCode::BAD_GATEWAY,
        };
        #[derive(Serialize)]
        struct Body {
            error: ApiError,
        }
        (status, Json(Body { error: self.0 })).into_response()
    }
}

impl From<ApiError> for HttpError {
    fn from(error: ApiError) -> Self {
        Self(error)
    }
}

impl From<auth::AuthError> for HttpError {
    fn from(error: auth::AuthError) -> Self {
        HttpError(error.into())
    }
}

/// The zone from `X-Rambl-TZ`, else `RAMBL_DEFAULT_TZ`, else `TZ_NAME`, else New York.
///
/// `TZ_NAME` is the name v1 and the existing `.env` files use, so a server set up
/// before v2 keeps the zone it was configured with.
pub fn zone(headers: &HeaderMap) -> Result<Tz, HttpError> {
    match headers.get(&TZ_HEADER) {
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|name| name.parse::<Tz>().ok())
            .ok_or_else(|| {
                HttpError(ApiError {
                    code: ErrorCode::BadRequest,
                    message: "X-Rambl-TZ is not a known IANA time zone".into(),
                })
            }),
        None => Ok(zone_from_env(
            std::env::var("RAMBL_DEFAULT_TZ").ok(),
            std::env::var("TZ_NAME").ok(),
        )),
    }
}

/// The zone for a request that named none, from the two configured fallbacks.
///
/// Split out from [`zone`] so it can be tested without touching the process
/// environment: these tests run in parallel threads and `std::env::set_var` is
/// `unsafe` in edition 2024 for exactly that reason.
fn zone_from_env(default_tz: Option<String>, tz_name: Option<String>) -> Tz {
    default_tz
        .or(tz_name)
        .and_then(|name| name.parse().ok())
        .unwrap_or(chrono_tz::America::New_York)
}

/// The emails from `RAMBL_ADMIN_EMAILS` (a comma-separated list), lower-cased.
/// `None` when the variable is unset or empty — an admin page nobody enabled is off.
fn admin_emails_from_env() -> Option<Vec<String>> {
    let raw = std::env::var("RAMBL_ADMIN_EMAILS").ok()?;
    let emails: Vec<String> = raw
        .split(',')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|email| !email.is_empty())
        .collect();
    (!emails.is_empty()).then_some(emails)
}

pub fn router(app: App<Box<dyn Store>>, judge: AnyJudge) -> Router {
    let local = std::env::var("GOOGLE_CLIENT_ID").is_err();
    let verifier: Box<dyn TokenVerifier> = if local {
        // Never reached: in local mode every request is the local user.
        Box::new(auth::FakeVerifier {
            answer: Err("sign-in is disabled".into()),
        })
    } else {
        Box::new(auth::GoogleVerifier::new(
            std::env::var("GOOGLE_CLIENT_ID").expect("checked above"),
        ))
    };
    build_router(
        app,
        judge,
        verifier,
        local,
        Caps::from_env(),
        admin_emails_from_env(),
        TrustedProxies::from_env(),
    )
}

/// The router with a supplied verifier, so tests can sign in without a Google project
/// and without touching the network.
#[cfg(test)]
fn router_with_verifier(
    app: App<Box<dyn Store>>,
    judge: AnyJudge,
    verifier: Box<dyn TokenVerifier>,
    local: bool,
    caps: Caps,
    admin_emails: Option<Vec<String>>,
    trusted_proxies: TrustedProxies,
) -> Router {
    build_router(
        app,
        judge,
        verifier,
        local,
        caps,
        admin_emails,
        trusted_proxies,
    )
}

/// Captures per user per day before capture stops using Jev, for a signed-in user.
pub const DEFAULT_JEV_DAILY_CAP: u32 = 200;
/// The same for a user on the paid plan.
pub const DEFAULT_PAID_JEV_DAILY_CAP: u32 = 1000;
/// The same for an anonymous user.
pub const DEFAULT_ANON_JEV_DAILY_CAP: u32 = 30;
/// Anonymous ids one client address may be issued per day.
pub const DEFAULT_ANON_IDS_PER_IP: u32 = 20;

impl Default for Caps {
    fn default() -> Self {
        Self {
            anon: DEFAULT_ANON_JEV_DAILY_CAP,
            free: DEFAULT_JEV_DAILY_CAP,
            paid: DEFAULT_PAID_JEV_DAILY_CAP,
            anon_ids_per_ip: DEFAULT_ANON_IDS_PER_IP,
        }
    }
}

fn valid_anon_id(value: &str) -> bool {
    let Some(suffix) = value.strip_prefix("anon_") else {
        return false;
    };
    suffix.len() == 32
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn new_anon_id() -> String {
    format!("anon_{}", uuid::Uuid::new_v4().simple())
}

fn build_router(
    app: App<Box<dyn Store>>,
    judge: AnyJudge,
    verifier: Box<dyn TokenVerifier>,
    local: bool,
    caps: Caps,
    admin_emails: Option<Vec<String>>,
    trusted_proxies: TrustedProxies,
) -> Router {
    let shared = Arc::new(Shared {
        app: Mutex::new(app),
        judge,
        verifier,
        local,
        caps,
        google_client_id: std::env::var("GOOGLE_CLIENT_ID")
            .ok()
            .filter(|id| !id.is_empty()),
        admin_emails,
        trusted_proxies,
    });
    Router::new()
        .route("/api/v1/config", get(config))
        .route("/api/v1/auth/google", post(sign_in_google))
        .route("/api/v1/auth/signout", post(sign_out))
        .route("/api/v1/me", get(me).delete(delete_me))
        .route("/api/v1/snapshot", get(snapshot))
        .route("/api/v1/tasks", post(create))
        .route("/api/v1/tasks/order", put(reorder))
        .route("/api/v1/import", post(import))
        .route("/api/v1/tasks/{id}", patch(update).delete(remove))
        .route("/api/v1/series", post(create_series))
        .route(
            "/api/v1/series/{id}",
            patch(update_series).delete(remove_series),
        )
        .route(
            "/api/v1/series/{id}/dates/{date}",
            patch(patch_occurrence).delete(clear_occurrence),
        )
        .route("/api/v1/capture", post(capture))
        .route("/api/v1/tasks/{id}/revise", post(revise_task))
        .route("/api/v1/calendar.ics", get(calendar_ics))
        .route("/admin", get(admin_page))
        .route("/api/v1/admin/usage", get(admin_usage))
        .route("/api/v1/admin/blocks", get(admin_blocks))
        .route("/api/v1/admin/block", post(admin_block))
        .route("/api/v1/admin/unblock", post(admin_unblock))
        .route("/api/v1/admin/plan", post(admin_set_plan))
        .with_state(Arc::clone(&shared))
        .layer(middleware::from_fn_with_state(shared, assign_anonymous_id))
}

/// Gives a request without a bearer token an anonymous id. A presented id that the
/// server issued is reused; anything else gets a fresh one, so a forged or stale id
/// never reaches another person's tasks. The id comes back in `X-Rambl-Anon`.
///
/// Local mode is exempt: every request already resolves to the fixed local user, so
/// nobody needs an id — and issuing one would burn the per-address allowance.
///
/// Sign-in is exempt: it reads the presented id itself, and adopts its tasks only if it
/// really is an anonymous id.
async fn assign_anonymous_id(
    State(shared): State<Arc<Shared>>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    if shared.local {
        return next.run(request).await;
    }
    if request
        .headers()
        .contains_key(axum::http::header::AUTHORIZATION)
        || request.uri().path() == "/api/v1/auth/google"
    {
        return next.run(request).await;
    }
    let presented = request
        .headers()
        .get(&ANON_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_anon_id(value))
        .map(str::to_owned);
    let id = match presented {
        Some(id) if issued_anonymous_id(&shared, &id) => id,
        _ => {
            let ip = client_ip(
                request.headers(),
                peer_ip(&request),
                &shared.trusted_proxies,
            );
            match issue_anonymous_id(&shared, &ip) {
                Ok(id) => id,
                Err(refusal) => return refusal.into_response(),
            }
        }
    };
    let header = HeaderValue::from_str(&id).expect("anonymous ids are ASCII");
    request.headers_mut().insert(ANON_HEADER, header.clone());
    let mut response = next.run(request).await;
    response.headers_mut().insert(ANON_HEADER, header);
    response
}

fn issued_anonymous_id(shared: &Shared, id: &str) -> bool {
    let app = shared.app.lock().expect("app mutex");
    app.store().anonymous_exists(id).unwrap_or(false)
}

/// A new anonymous id, unless this client address has used up its allowance today.
fn issue_anonymous_id(shared: &Shared, ip: &str) -> Result<String, HttpError> {
    let app = shared.app.lock().expect("app mutex");
    let today = app.now().date_naive();
    let issued = app
        .store()
        .record_anon_issued(ip, today)
        .map_err(store_failed)?;
    if issued > shared.caps.anon_ids_per_ip {
        return Err(unauthorized(
            "too many new anonymous sessions from this address today; sign in to keep going",
        ));
    }
    let id = new_anon_id();
    app.store()
        .ensure_anonymous(&id, app.now())
        .map_err(store_failed)?;
    Ok(id)
}

/// The address a request came from.
///
/// Cloudflare's `CF-Connecting-IP` names the real client, but only a proxy we trust
/// may say so: anyone who reaches the server on its own (the LAN, an open port)
/// could put any address in that header and walk past the per-address limits and
/// blocks. The header is therefore read only when the socket peer is a trusted
/// proxy, and it must be a real address — the value lands on the admin page, so a
/// string that is not an address is never stored.
fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>, trusted: &TrustedProxies) -> String {
    let behind_proxy = peer.is_some_and(|ip| trusted.trusts(ip));
    behind_proxy
        .then(|| headers.get("cf-connecting-ip"))
        .flatten()
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<IpAddr>().ok())
        .map(|ip| ip.to_string())
        .or_else(|| peer.map(|ip| ip.to_string()))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The socket peers allowed to name the client with `CF-Connecting-IP`.
///
/// Loopback is trusted without being configured: the Cloudflare tunnel runs on the
/// same machine, so that is the only peer production has. `RAMBL_TRUSTED_PROXIES`
/// adds any other proxy as a comma-separated list of addresses or networks.
#[derive(Clone, Debug, Default)]
struct TrustedProxies(Vec<IpNet>);

impl TrustedProxies {
    fn from_env() -> Self {
        Self::from_list(std::env::var("RAMBL_TRUSTED_PROXIES").ok().as_deref())
    }

    /// The entries naming a real address or network. An entry that does not parse is
    /// dropped, because being wrong here can only make us trust less.
    fn from_list(list: Option<&str>) -> Self {
        Self(
            list.unwrap_or_default()
                .split(',')
                .filter_map(IpNet::parse)
                .collect(),
        )
    }

    fn trusts(&self, peer: IpAddr) -> bool {
        peer.is_loopback() || self.0.iter().any(|network| network.contains(peer))
    }
}

/// One address or network from `RAMBL_TRUSTED_PROXIES`: `10.0.0.1`, `10.0.0.0/8`.
#[derive(Clone, Debug)]
struct IpNet {
    address: IpAddr,
    /// How many leading bits of `address` must match.
    prefix: u8,
}

impl IpNet {
    /// `None` for anything that is not an address with a prefix it could have.
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => {
                (address.parse::<IpAddr>().ok()?, prefix.parse::<u8>().ok()?)
            }
            None => {
                let address: IpAddr = text.parse().ok()?;
                return Some(Self {
                    prefix: if address.is_ipv4() { 32 } else { 128 },
                    address,
                });
            }
        };
        (prefix <= if address.is_ipv4() { 32 } else { 128 }).then_some(Self { address, prefix })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (self.address, ip) {
            (IpAddr::V4(network), IpAddr::V4(ip)) => {
                masked(IpAddr::V4(network), self.prefix) == masked(IpAddr::V4(ip), self.prefix)
            }
            (IpAddr::V6(network), IpAddr::V6(ip)) => {
                masked(IpAddr::V6(network), self.prefix) == masked(IpAddr::V6(ip), self.prefix)
            }
            // An IPv4 peer is not in an IPv6 network, however the bits line up.
            _ => false,
        }
    }
}

/// The address with everything past `prefix` cleared, so two addresses in one network
/// compare equal. The mask is built at the address's own width: a shift by the whole
/// width is out of range, and keeps nothing, which is what prefix 0 asks for anyway.
fn masked(address: IpAddr, prefix: u8) -> IpAddr {
    match address {
        IpAddr::V4(ip) => {
            let keep = u32::BITS - u32::from(prefix.min(32));
            let mask = (!0u32).checked_shl(keep).unwrap_or(0);
            IpAddr::V4(Ipv4Addr::from(u32::from(ip) & mask))
        }
        IpAddr::V6(ip) => {
            let keep = u128::BITS - u32::from(prefix.min(128));
            let mask = (!0u128).checked_shl(keep).unwrap_or(0);
            IpAddr::V6(Ipv6Addr::from(u128::from(ip) & mask))
        }
    }
}

fn peer_ip(request: &axum::extract::Request) -> Option<IpAddr> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0.ip())
}

/// A store that could not answer is our failure, not the caller's.
fn store_failed(error: StoreError) -> HttpError {
    HttpError(ApiError {
        code: ErrorCode::Internal,
        message: error.to_string(),
    })
}

/// `GET /api/v1/config`: what the page needs before anyone signs in. The client id is
/// public: Google shows it on its own sign-in page.
async fn config(State(shared): State<Arc<Shared>>) -> Json<ConfigOut> {
    Json(ConfigOut {
        google_client_id: shared.google_client_id.clone(),
    })
}

#[derive(Serialize)]
struct ConfigOut {
    google_client_id: Option<String>,
}

/// `POST /api/v1/auth/google`: trade an ID token for a session.
async fn sign_in_google(
    State(shared): State<Arc<Shared>>,
    request: axum::extract::Request,
) -> Result<Json<auth::Session>, HttpError> {
    let anonymous = request
        .headers()
        .get(&ANON_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_anon_id(value))
        .map(str::to_owned);
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
    let app = shared.app.lock().expect("app mutex");
    let session = auth::open_session(app.store(), &info, app.now(), anonymous.as_deref())
        .map_err(HttpError::from)?;
    Ok(Json(session))
}

#[derive(Deserialize)]
struct SignInIn {
    credential: String,
}

/// `POST /api/v1/auth/signout`: end this session only.
async fn sign_out(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<StatusCode, HttpError> {
    if !shared.local {
        let token = auth::bearer(&headers)
            .ok_or_else(|| unauthorized("send Authorization: Bearer <token>"))?;
        let app = shared.app.lock().expect("app mutex");
        auth::sign_out(app.store(), &token).map_err(HttpError::from)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/me`: who this request is.
async fn me(
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
    let token =
        auth::bearer(&headers).ok_or_else(|| unauthorized("send Authorization: Bearer <token>"))?;
    let app = shared.app.lock().expect("app mutex");
    let user = auth::session_user(app.store(), &token, Utc::now())
        .map_err(HttpError::from)?
        .ok_or_else(|| unauthorized("that session has expired"))?;
    Ok(Json(user))
}

/// `DELETE /api/v1/me`: forget this person, their sessions and all their tasks.
async fn delete_me(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<StatusCode, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let app = shared.app.lock().expect("app mutex");
    auth::delete_user(app.store(), &user).map_err(HttpError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// A request is an admin when it carries a live signed-in session whose email is
/// in `RAMBL_ADMIN_EMAILS`. Anyone else sees 404, so the admin surface does not
/// announce itself.
fn admin_of(headers: &HeaderMap, shared: &Shared) -> Result<auth::Me, HttpError> {
    let token = auth::bearer(headers).ok_or_else(admin_404)?;
    let app = shared.app.lock().expect("app mutex");
    let user = auth::session_user(app.store(), &token, Utc::now())
        .map_err(HttpError::from)?
        .ok_or_else(admin_404)?;
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

fn admin_404() -> HttpError {
    HttpError(ApiError {
        code: ErrorCode::NotFound,
        message: "no such page".into(),
    })
}

/// `GET /admin`: the administration page. Off when no emails are configured. A
/// signed-in request (a Bearer token) that is not an admin gets 404; a plain
/// navigation carries no token, so it gets the shell, which signs in the same way
/// the main page does — only a listed email ever sees data.
async fn admin_page(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<Html<&'static str>, HttpError> {
    if shared.admin_emails.is_none() {
        return Err(admin_404());
    }
    if auth::bearer(&headers).is_some() {
        admin_of(&headers, &shared)?;
    }
    Ok(Html(admin_web::PAGE))
}

/// `GET /api/v1/admin/usage`: today's captures by user (with plan) and address,
/// and the totals per plan. One capture is one Jev call.
async fn admin_usage(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<Json<admin_web::UsageOut>, HttpError> {
    admin_of(&headers, &shared)?;
    let app = shared.app.lock().expect("app mutex");
    Ok(Json(
        admin_web::usage(app.store(), app.now().date_naive()).map_err(store_failed)?,
    ))
}

/// `GET /api/v1/admin/blocks`: everything currently blocked.
async fn admin_blocks(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<Json<Vec<admin_web::BlockOut>>, HttpError> {
    admin_of(&headers, &shared)?;
    let app = shared.app.lock().expect("app mutex");
    Ok(Json(admin_web::blocks(app.store()).map_err(store_failed)?))
}

#[derive(Deserialize)]
struct BlockIn {
    kind: String,
    value: String,
}

#[derive(Deserialize)]
struct PlanIn {
    who: String,
    plan: String,
}

#[derive(Serialize)]
struct AdminOut {
    message: String,
}

/// `POST /api/v1/admin/block` `{kind: "user" | "ip", value}`: block a user (by
/// id or email) or an address.
async fn admin_block(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<BlockIn>,
) -> Result<Json<AdminOut>, HttpError> {
    admin_of(&headers, &shared)?;
    let app = shared.app.lock().expect("app mutex");
    let message = admin_web::block(app.store(), &body.kind, &body.value).map_err(bad_request)?;
    Ok(Json(AdminOut { message }))
}

/// `POST /api/v1/admin/unblock` `{kind, value}`: lift a block placed the same way.
async fn admin_unblock(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<BlockIn>,
) -> Result<Json<AdminOut>, HttpError> {
    admin_of(&headers, &shared)?;
    let app = shared.app.lock().expect("app mutex");
    let message = admin_web::unblock(app.store(), &body.kind, &body.value).map_err(bad_request)?;
    Ok(Json(AdminOut { message }))
}

/// `POST /api/v1/admin/plan` `{who: email|id, plan}`: move a user to another plan.
async fn admin_set_plan(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<PlanIn>,
) -> Result<Json<AdminOut>, HttpError> {
    admin_of(&headers, &shared)?;
    let app = shared.app.lock().expect("app mutex");
    let message = admin_web::set_plan(app.store(), &body.who, &body.plan).map_err(bad_request)?;
    Ok(Json(AdminOut { message }))
}

/// `GET /api/v1/snapshot?from=YYYY-MM-DD&days=N`: the plan window, today and 7 days
/// by default. The task list is always the whole list.
async fn snapshot(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(window): Query<SnapshotWindow>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let app = shared.app.lock().unwrap();
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
    Ok(Json(app.snapshot_in_window(&user, tz, from, days)?))
}

/// The `?from=`/`?days=` plan window.
#[derive(Default, Deserialize)]
struct SnapshotWindow {
    from: Option<NaiveDate>,
    days: Option<u32>,
}

const DEFAULT_WINDOW_DAYS: u32 = 7;
/// A year, matching the horizon the scheduler itself fills.
const MAX_WINDOW_DAYS: u32 = 366;

async fn create(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(draft): Json<TaskDraft>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.create(draft, &user, tz)?))
}

async fn update(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(patch): Json<Patch>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.patch(&id, patch, &user, tz)?))
}

async fn remove(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.delete(&id, &user, tz)?))
}

/// `POST /api/v1/series`: a weekly series (ARCHITECTURE.md 7.1).
async fn create_series(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(draft): Json<app::SeriesDraft>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.create_series(draft, &user, tz)?))
}

/// `PATCH /api/v1/series/{id}`: edit the rule every occurrence follows.
async fn update_series(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(patch): Json<app::SeriesPatch>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.update_series(&id, patch, &user, tz)?))
}

/// `DELETE /api/v1/series/{id}`: the whole series, exceptions with it.
async fn remove_series(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.delete_series(&id, &user, tz)?))
}

/// `PATCH /api/v1/series/{id}/dates/{date}`: done / skipped / moved for one
/// occurrence. `{date}` is `YYYY-MM-DD`, the occurrence's local date.
async fn patch_occurrence(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path((id, date)): axum::extract::Path<(String, NaiveDate)>,
    Json(body): Json<app::OccurrencePatch>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.patch_occurrence(&id, date, body, &user, tz)?))
}

/// `DELETE /api/v1/series/{id}/dates/{date}`: clear one occurrence's
/// exception, so the day falls back to the rule.
async fn clear_occurrence(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Path((id, date)): axum::extract::Path<(String, NaiveDate)>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.clear_occurrence(&id, date, &user, tz)?))
}

/// `POST /api/v1/import`: a multipart `file` holding an .ics calendar.
async fn import(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<ImportResult>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut ics_text = None;
    while let Some(field) = multipart.next_field().await.map_err(bad_request)? {
        if field.name() == Some("file") {
            ics_text = Some(field.text().await.map_err(bad_request)?);
        }
    }
    let ics_text =
        ics_text.ok_or_else(|| bad_request("send the calendar as multipart field `file`"))?;
    let events = ics_import::parse_ics(&ics_text).map_err(|error| bad_request(error.0))?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.import(events, &user, tz)?))
}

fn bad_request(message: impl std::fmt::Display) -> HttpError {
    HttpError(ApiError {
        code: ErrorCode::BadRequest,
        message: message.to_string(),
    })
}

#[derive(Deserialize)]
struct OrderIn {
    ids: Vec<String>,
}

async fn reorder(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<OrderIn>,
) -> Result<Json<Snapshot>, HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.reorder(&body.ids, &user, tz)?))
}

#[derive(Deserialize)]
struct CaptureIn {
    text: String,
}

#[derive(Serialize)]
struct CaptureOut {
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
async fn capture(
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
    let now = anchor.unwrap_or_else(|| shared.app.lock().unwrap().now());
    let ctx = LocalContext { now, tz };
    let plan = if headers.contains_key(axum::http::header::AUTHORIZATION) {
        let app = shared.app.lock().expect("app mutex");
        app.store().user_plan(&user).map_err(store_failed)?
    } else {
        auth::Plan::Anon
    };
    let jev_fallback = {
        let app = shared.app.lock().expect("app mutex");
        let store = app.store();
        let day = now.date_naive();
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
    let drafts = if jev_fallback {
        capture_text(&judge::FakeJudge, &text, source, &ctx)
            .await
            .map_err(upstream)?
    } else {
        capture_text(&shared.judge, &text, source, &ctx)
            .await
            .map_err(upstream)?
    };
    let mut app = shared.app.lock().unwrap();
    let (created, snapshot) = app.add_drafts(drafts, &user, tz)?;
    Ok(Json(CaptureOut {
        created,
        snapshot,
        jev_fallback,
    }))
}

/// The capture text, its source, and an anchor time when the source has a send date.
///
/// Only `.pdf`, `.eml` and `.txt` are accepted. Anything else used to fall through to
/// "treat the bytes as text", which quietly accepted a `.docx` and handed the judge
/// binary noise.
async fn read_capture(
    request: axum::extract::Request,
) -> Result<(String, Source, Option<DateTime<Utc>>), HttpError> {
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

fn file_text(
    name: &str,
    bytes: &[u8],
) -> Result<(String, Source, Option<DateTime<Utc>>), HttpError> {
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
struct ReviseIn {
    instruction: String,
    /// The task's `updated_at` as the client last saw it; see [`Patch::updated_at`].
    updated_at: Option<chrono::DateTime<Utc>>,
}

/// `POST /api/v1/tasks/{id}/revise`: one sentence re-classifies the task.
async fn revise_task(
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
        let app = shared.app.lock().unwrap();
        (app.title_of(&user, &id)?, app.now())
    };
    let ctx = LocalContext { now, tz };
    let revision: Revision = capture::revise(&shared.judge, &title, &instruction, &ctx)
        .await
        .map_err(upstream)?;
    let mut app = shared.app.lock().unwrap();
    Ok(Json(app.apply_revision(
        &id,
        &revision,
        body.updated_at,
        &user,
        tz,
    )?))
}

/// `GET /api/v1/calendar.ics`: the plan as an iCalendar file.
async fn calendar_ics(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Result<([(HeaderName, &'static str); 1], String), HttpError> {
    let user = CurrentUser::resolve(&headers, &shared)?;
    let tz = zone(&headers)?;
    let snapshot = shared.app.lock().unwrap().snapshot(&user, tz)?;
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/calendar; charset=utf-8",
        )],
        build_ics(&snapshot),
    ))
}

/// A Jev failure, split the way ARCHITECTURE.md section 6 splits it.
///
/// A network error or a timeout means Jev was never reached: the dependency is down,
/// so this is `Unavailable` (503) and the caller should retry. A response that came
/// back but could not be understood is `Upstream` (502): Jev answered, wrongly.
/// Collapsing both into 502 tells the client to give up on a dependency that is
/// merely busy.
fn upstream(error: anyhow::Error) -> HttpError {
    let message = error.to_string();
    let unreachable = error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|error| error.is_timeout() || error.is_connect())
    });
    let code = if unreachable {
        ErrorCode::Unavailable
    } else {
        ErrorCode::Upstream
    };
    HttpError(ApiError {
        code,
        message: format!("judgment failed: {message}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::store::{MemoryStore, SqliteStore};
    use axum::body::Body;
    use axum::http::Request;
    use chrono::{TimeZone, Utc};
    use http_body_util::BodyExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use todo_core::{FixedClock, Label, Priority, Source, SystemClock, When};
    use tower::ServiceExt;

    /// Every code maps to the status a client should see, and the body is
    /// always `{ "error": { code, message } }`. Pinned because the mapping is
    /// the contract, and `Internal` vs `Unavailable` is exactly the kind of
    /// pair that gets quietly swapped.
    #[test]
    fn every_error_code_maps_to_its_status() {
        let expected = [
            (ErrorCode::BadRequest, StatusCode::BAD_REQUEST),
            (ErrorCode::NotFound, StatusCode::NOT_FOUND),
            (ErrorCode::Conflict, StatusCode::CONFLICT),
            (ErrorCode::StaleUpdate, StatusCode::CONFLICT),
            // A dependency is down: come back later.
            (ErrorCode::Unavailable, StatusCode::SERVICE_UNAVAILABLE),
            (ErrorCode::Upstream, StatusCode::BAD_GATEWAY),
            // Our own failure.
            (ErrorCode::Internal, StatusCode::INTERNAL_SERVER_ERROR),
        ];

        for (code, want) in expected {
            let response = HttpError(ApiError {
                code,
                message: "boom".into(),
            })
            .into_response();
            assert_eq!(response.status(), want, "wrong status for {code:?}");
        }
    }

    #[test]
    fn an_error_body_is_error_code_message() {
        let response = HttpError(ApiError {
            code: ErrorCode::Internal,
            message: "storage is unhappy".into(),
        })
        .into_response();

        let bytes =
            futures::executor::block_on(axum::body::to_bytes(response.into_body(), usize::MAX))
                .expect("read body");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("body is JSON");

        assert_eq!(json["error"]["code"], "internal");
        assert_eq!(json["error"]["message"], "storage is unhappy");
    }

    fn router_at(now: chrono::DateTime<Utc>) -> Router {
        super::router(
            App::new(
                Box::new(MemoryStore::default()) as Box<dyn Store>,
                Arc::new(FixedClock(now)),
            ),
            AnyJudge::Fake(judge::FakeJudge),
        )
    }

    /// The snapshot invariant from ARCHITECTURE.md section 9: every open pinned task
    /// shows its placement, and no placement exists for a done or missing task.
    fn assert_invariant(snapshot: &Snapshot) {
        for placement in &snapshot.plan.placements {
            let task = snapshot
                .tasks
                .iter()
                .find(|task| task.id == placement.task)
                .expect("placement names a task in the snapshot");
            assert!(!task.done, "placement for a done task");
        }
        for task in snapshot.tasks.iter().filter(|task| !task.done) {
            if let When::At(start) = task.when {
                let shown: Vec<_> = snapshot
                    .plan
                    .placements
                    .iter()
                    .filter(|placement| placement.task == task.id)
                    .collect();
                assert_eq!(
                    shown.len(),
                    1,
                    "one placement for pinned task {}",
                    task.title
                );
                assert_eq!(shown[0].start, start);
            }
        }
    }

    async fn send(
        router: &Router,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-rambl-tz", "America/New_York");
        let body = match body {
            Some(text) => {
                builder = builder.header("content-type", "application/json");
                Body::from(text.to_owned())
            }
            None => Body::empty(),
        };
        let response = router
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    /// Sunday 2026-10-11 21:00 in New York.
    fn sunday_night() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn create_then_snapshot_shows_the_task_and_its_placement() {
        let router = router_at(sunday_night());
        let (status, body) = send(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(r#"{"title":"Write essay","duration_min":60,"when":{"kind":"anytime"},"label":"School","priority":"high","source":"text"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        assert_eq!(snapshot.tasks.len(), 1);
        assert_eq!(snapshot.plan.placements.len(), 1);
        assert_eq!(snapshot.tz, "America/New_York");
        assert_invariant(&snapshot);
    }

    #[tokio::test]
    async fn pinned_task_keeps_its_time_and_blocks_floating_work() {
        let router = router_at(sunday_night());
        let pinned = Utc.with_ymd_and_hms(2026, 10, 12, 19, 0, 0).unwrap(); // 15:00 EDT Monday
        let draft = format!(
            r#"{{"title":"Meet Maya","duration_min":60,"when":{{"kind":"at","time":"{}"}},"label":"Work","priority":"low","source":"text"}}"#,
            pinned.to_rfc3339()
        );
        send(&router, "POST", "/api/v1/tasks", Some(&draft)).await;
        let (_, body) = send(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(r#"{"title":"Read","duration_min":30,"when":{"kind":"anytime"},"label":"School","priority":"low","source":"text"}"#),
        )
        .await;
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        assert_invariant(&snapshot);
        let meet = snapshot
            .tasks
            .iter()
            .find(|t| t.title == "Meet Maya")
            .unwrap();
        let read = snapshot.tasks.iter().find(|t| t.title == "Read").unwrap();
        let read_place = snapshot
            .plan
            .placements
            .iter()
            .find(|p| p.task == read.id)
            .unwrap();
        let meet_place = snapshot
            .plan
            .placements
            .iter()
            .find(|p| p.task == meet.id)
            .unwrap();
        assert!(read_place.end <= meet_place.start || read_place.start >= meet_place.end);
    }

    #[tokio::test]
    async fn patch_done_removes_the_placement() {
        let router = router_at(sunday_night());
        let (_, body) = send(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(r#"{"title":"Pay rent","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#),
        )
        .await;
        let id = serde_json::from_value::<Snapshot>(body).unwrap().tasks[0]
            .id
            .0
            .clone();
        let (status, body) = send(
            &router,
            "PATCH",
            &format!("/api/v1/tasks/{id}"),
            Some(r#"{"done":true}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        assert!(snapshot.plan.placements.is_empty());
        assert!(snapshot.tasks[0].done);
        assert_invariant(&snapshot);
    }

    #[tokio::test]
    async fn a_patch_made_against_an_older_version_is_a_409_and_changes_nothing() {
        let router = router_at(sunday_night());
        let (_, body) = send(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(r#"{"title":"Pay rent","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#),
        )
        .await;
        let task = serde_json::from_value::<Snapshot>(body)
            .unwrap()
            .tasks
            .remove(0);
        let path = format!("/api/v1/tasks/{}", task.id.0);

        let stale = r#"{"title":"Overwritten","updated_at":"1970-01-01T00:00:00Z"}"#;
        let (status, body) = send(&router, "PATCH", &path, Some(stale)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"]["code"], "stale_update");

        let current = format!(
            r#"{{"title":"Pay the rent","updated_at":"{}"}}"#,
            task.updated_at.to_rfc3339()
        );
        let (status, body) = send(&router, "PATCH", &path, Some(&current)).await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        assert_eq!(
            snapshot.tasks[0].title, "Pay the rent",
            "the refused edit applied"
        );
    }

    #[tokio::test]
    async fn delete_removes_the_task_and_unknown_ids_are_404() {
        let router = router_at(sunday_night());
        let (_, body) = send(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(r#"{"title":"Gone","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#),
        )
        .await;
        let id = serde_json::from_value::<Snapshot>(body).unwrap().tasks[0]
            .id
            .0
            .clone();
        let (status, body) = send(&router, "DELETE", &format!("/api/v1/tasks/{id}"), None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            serde_json::from_value::<Snapshot>(body)
                .unwrap()
                .tasks
                .is_empty()
        );
        let (status, body) = send(&router, "DELETE", "/api/v1/tasks/nope", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "not_found");
    }

    #[tokio::test]
    async fn reorder_needs_every_id_once_and_sets_the_order() {
        let router = router_at(sunday_night());
        let mut ids = Vec::new();
        for title in ["First", "Second"] {
            let draft = format!(
                r#"{{"title":"{title}","duration_min":30,"when":{{"kind":"anytime"}},"label":"Other","priority":"low","source":"text"}}"#
            );
            let (_, body) = send(&router, "POST", "/api/v1/tasks", Some(&draft)).await;
            ids = serde_json::from_value::<Snapshot>(body)
                .unwrap()
                .tasks
                .iter()
                .map(|t| t.id.0.clone())
                .collect();
        }
        let reversed = format!(r#"{{"ids":["{}","{}"]}}"#, ids[1], ids[0]);
        let (status, body) = send(&router, "PUT", "/api/v1/tasks/order", Some(&reversed)).await;
        assert_eq!(status, StatusCode::OK);
        // Newest-first made the snapshot list Second above First, so the
        // "reversed" request asks for First back on top; the response must show
        // that order, not the creation order.
        assert_eq!(
            serde_json::from_value::<Snapshot>(body)
                .unwrap()
                .tasks
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            ["First", "Second"]
        );

        let (status, body) = send(
            &router,
            "PUT",
            "/api/v1/tasks/order",
            Some(&format!(r#"{{"ids":["{}"]}}"#, ids[0])),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "bad_request");
    }

    #[tokio::test]
    async fn bad_input_is_a_400_and_a_bad_zone_is_a_400() {
        let router = router_at(sunday_night());
        let (status, _) = send(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(r#"{"title":"  ","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let request = Request::builder()
            .uri("/api/v1/snapshot")
            .header("x-rambl-tz", "Mars/Olympus")
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    const SYNTHETIC_ICS: &str = include_str!("../../tests/fixtures/synthetic.ics");

    async fn import_ics(router: &Router, ics: &str) -> (StatusCode, serde_json::Value) {
        let boundary = "XRAMBLBOUNDARY";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"cal.ics\"\r\nContent-Type: text/calendar\r\n\r\n{ics}\r\n--{boundary}--\r\n"
        );
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/import")
            .header("x-rambl-tz", "America/New_York")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn import_makes_pinned_tasks_and_reimport_updates_them_by_uid() {
        let router = router_at(sunday_night());
        let (status, first) = import_ics(&router, SYNTHETIC_ICS).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            first["created"].as_array().unwrap().len(),
            2,
            "cancelled event is skipped"
        );
        assert!(first["updated"].as_array().unwrap().is_empty());
        let snapshot: Snapshot = serde_json::from_value(first["snapshot"].clone()).unwrap();
        assert_invariant(&snapshot);
        assert!(
            snapshot
                .tasks
                .iter()
                .all(|task| task.source == Source::Calendar)
        );
        assert!(
            snapshot
                .tasks
                .iter()
                .all(|task| matches!(task.when, When::At(_)))
        );

        let (_, second) = import_ics(&router, SYNTHETIC_ICS).await;
        assert!(second["created"].as_array().unwrap().is_empty());
        assert_eq!(second["updated"].as_array().unwrap().len(), 2);
        let again: Snapshot = serde_json::from_value(second["snapshot"].clone()).unwrap();
        assert_eq!(again.tasks.len(), 2, "re-import must not duplicate");
    }

    #[tokio::test]
    async fn import_without_a_file_field_is_a_400() {
        let router = router_at(sunday_night());
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/import")
            .header("content-type", "multipart/form-data; boundary=B")
            .body(Body::from(
                "--B--
",
            ))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn event_with_no_end_gets_thirty_minutes() {
        let router = router_at(sunday_night());
        let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:no-end@example.test\nDTSTART:20261012T130000Z\nSUMMARY:No end\nEND:VEVENT\nEND:VCALENDAR\n";
        let (status, body) = import_ics(&router, ics).await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
        assert_eq!(snapshot.tasks[0].duration_min, 30);
        assert_invariant(&snapshot);
    }

    /// Runs against burvy's real calendar. Personal data, so never committed: point
    /// `RAMBL_SAMPLE_ICS` at a local file and run `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore = "reads a local calendar named by RAMBL_SAMPLE_ICS"]
    async fn import_of_the_local_sample_calendar_keeps_the_invariant() {
        let path = std::env::var("RAMBL_SAMPLE_ICS").expect("set RAMBL_SAMPLE_ICS");
        let ics = std::fs::read_to_string(path).expect("read the sample calendar");
        let router = router_at(sunday_night());
        let (status, body) = import_ics(&router, &ics).await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
        assert_invariant(&snapshot);
    }

    async fn send_raw(
        router: &Router,
        method: &str,
        uri: &str,
        content_type: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value, String) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-rambl-tz", "America/New_York")
            .header("content-type", content_type)
            .body(Body::from(body.to_owned()))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, json, headers)
    }

    #[tokio::test]
    async fn capture_json_creates_tasks_and_returns_their_ids() {
        let router = router_at(sunday_night());
        let (status, body, _) = send_raw(
            &router,
            "POST",
            "/api/v1/capture",
            "application/json",
            r#"{"text":"Buy milk"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["created"].as_array().unwrap().len(), 1);
        let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
        assert_eq!(snapshot.tasks.len(), 1);
        assert_invariant(&snapshot);
    }

    #[tokio::test]
    async fn capture_multipart_text_file_is_read_as_text() {
        let router = router_at(sunday_night());
        let boundary = "XRAMBLCAP";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"notes.txt\"\r\nContent-Type: text/plain\r\n\r\nSubmit the lab report\r\n--{boundary}--\r\n"
        );
        let (status, json, _) = send_raw(
            &router,
            "POST",
            "/api/v1/capture",
            &format!("multipart/form-data; boundary={boundary}"),
            &body,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(json["snapshot"].clone()).unwrap();
        assert_eq!(snapshot.tasks[0].source, Source::Text);
        assert_invariant(&snapshot);
    }

    #[tokio::test]
    async fn capture_with_a_bad_pdf_or_empty_text_is_a_400() {
        let router = router_at(sunday_night());
        let boundary = "XRAMBLBAD";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x.pdf\"\r\n\r\nnot a pdf\r\n--{boundary}--\r\n"
        );
        let (status, _, _) = send_raw(
            &router,
            "POST",
            "/api/v1/capture",
            &format!("multipart/form-data; boundary={boundary}"),
            &body,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _, _) = send_raw(
            &router,
            "POST",
            "/api/v1/capture",
            "application/json",
            r#"{"text":"   "}"#,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    async fn create_task(router: &Router, title: &str) -> String {
        let draft = format!(
            r#"{{"title":"{title}","duration_min":30,"when":{{"kind":"anytime"}},"label":"Other","priority":"low","source":"text"}}"#
        );
        let (_, body) = send(router, "POST", "/api/v1/tasks", Some(&draft)).await;
        serde_json::from_value::<Snapshot>(body)
            .unwrap()
            .tasks
            .iter()
            .find(|task| task.title == title)
            .unwrap()
            .id
            .0
            .clone()
    }

    #[tokio::test]
    async fn revise_thursday_at_two_pins_the_task() {
        let router = router_at(sunday_night());
        let id = create_task(&router, "Meet Maya").await;
        let (status, body) = send(
            &router,
            "POST",
            &format!("/api/v1/tasks/{id}/revise"),
            Some(r#"{"instruction":"make it Thursday at 2"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        let task = snapshot.tasks.iter().find(|task| task.id.0 == id).unwrap();
        // Thursday 2026-10-15 14:00 EDT is 18:00 UTC.
        assert_eq!(
            task.when,
            When::At(Utc.with_ymd_and_hms(2026, 10, 15, 18, 0, 0).unwrap())
        );
        assert_invariant(&snapshot);
    }

    #[tokio::test]
    async fn revise_errors_unknown_id_and_empty_instruction() {
        let router = router_at(sunday_night());
        let (status, _) = send(
            &router,
            "POST",
            "/api/v1/tasks/nope/revise",
            Some(r#"{"instruction":"tomorrow"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let id = create_task(&router, "Read").await;
        let (status, _) = send(
            &router,
            "POST",
            &format!("/api/v1/tasks/{id}/revise"),
            Some(r#"{"instruction":"  "}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn calendar_ics_exports_the_plan_as_events() {
        let router = router_at(sunday_night());
        let id = create_task(&router, "Read").await;
        assert!(!id.is_empty());
        let request = Request::builder()
            .uri("/api/v1/calendar.ics")
            .header("x-rambl-tz", "America/New_York")
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "text/calendar; charset=utf-8"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.starts_with("BEGIN:VCALENDAR"));
        let calendar: icalendar::Calendar = text.parse().unwrap();
        assert_eq!(
            calendar.events().count(),
            1,
            "one anytime task, placed once"
        );
    }

    /// A series over `/api/v1/series`: created, edited, and marked occurrence by
    /// occurrence (docs/ARCHITECTURE.md 7.1).
    #[tokio::test]
    async fn a_series_is_created_and_its_occurrences_are_marked() {
        let router = router_at(sunday_night());
        let (status, body) = send(
            &router,
            "POST",
            "/api/v1/series",
            Some(
                r#"{"title":"Gym","duration_min":60,"weekdays":["Mon","Tue","Thu"],
                    "start_day":"2026-10-12","time_of_day":"07:00:00","until":"2026-12-18"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        assert_eq!(snapshot.series.len(), 1);
        let id = snapshot.series[0].id.0.clone();
        assert_eq!(snapshot.series[0].zone, "America/New_York");
        assert_eq!(
            snapshot.series[0].rule.until,
            chrono::NaiveDate::from_ymd_opt(2026, 12, 18),
            "the end sent with the series is kept"
        );
        let mut rows: Vec<String> = snapshot
            .tasks
            .iter()
            .filter(|task| task.source == todo_core::Source::Series)
            .map(|task| task.id.0.clone())
            .collect();
        rows.sort();
        assert_eq!(
            rows,
            vec![
                format!("{id}:2026-10-12"),
                format!("{id}:2026-10-13"),
                format!("{id}:2026-10-15")
            ],
            "one row per repeated day this week"
        );
        assert_invariant(&snapshot);

        // Done: the row stays, marked, and holds no slot.
        let (status, body) = send(
            &router,
            "PATCH",
            &format!("/api/v1/series/{id}/dates/2026-10-12"),
            Some(r#"{"kind":"done"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        let monday = format!("{id}:2026-10-12");
        assert!(
            snapshot
                .tasks
                .iter()
                .any(|task| task.id.0 == monday && task.done)
        );
        assert!(
            !snapshot
                .plan
                .placements
                .iter()
                .any(|placement| placement.task.0 == monday),
            "a done occurrence holds no slot"
        );
        assert_invariant(&snapshot);

        // Skipped: the row is gone.
        let (status, body) = send(
            &router,
            "PATCH",
            &format!("/api/v1/series/{id}/dates/2026-10-13"),
            Some(r#"{"kind":"skipped"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        assert!(
            !snapshot
                .tasks
                .iter()
                .any(|task| task.id.0.ends_with("2026-10-13")),
            "a skipped day produces no row"
        );

        // Moved: the row keeps its date and takes the new instant.
        let (status, body) = send(
            &router,
            "PATCH",
            &format!("/api/v1/series/{id}/dates/2026-10-15"),
            Some(r#"{"kind":"moved","when":{"kind":"at","time":"2026-10-15T22:00:00Z"}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        let moved = snapshot
            .tasks
            .iter()
            .find(|task| task.id.0 == format!("{id}:2026-10-15"))
            .expect("the moved row is there under its original date");
        assert_eq!(
            moved.when,
            When::At(Utc.with_ymd_and_hms(2026, 10, 15, 22, 0, 0).unwrap())
        );
        assert_invariant(&snapshot);

        // Clearing puts a day back to the rule.
        let (status, _) = send(
            &router,
            "DELETE",
            &format!("/api/v1/series/{id}/dates/2026-10-15"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Editing the series replaces its rule.
        let (status, body) = send(
            &router,
            "PATCH",
            &format!("/api/v1/series/{id}"),
            Some(r#"{"title":"Yoga","weekdays":["Wed"]}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        assert_eq!(snapshot.series[0].title, "Yoga");
        assert_eq!(
            snapshot
                .tasks
                .iter()
                .filter(|task| task.source == todo_core::Source::Series)
                .map(|task| task.id.0.clone())
                .collect::<Vec<String>>(),
            vec![format!("{id}:2026-10-14")],
            "only Wednesdays now, and the days the rule no longer covers are not rows"
        );

        // Deleting takes the rows with it.
        let (status, body) = send(&router, "DELETE", &format!("/api/v1/series/{id}"), None).await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        assert!(snapshot.series.is_empty());
        assert!(
            !snapshot
                .tasks
                .iter()
                .any(|task| task.source == todo_core::Source::Series)
        );
        assert_invariant(&snapshot);
    }

    /// The window a snapshot asks for is the window the series is expanded over.
    #[tokio::test]
    async fn a_snapshot_expands_a_series_over_the_window_it_asks_for() {
        let router = router_at(sunday_night());
        let (_, body) = send(
            &router,
            "POST",
            "/api/v1/series",
            Some(
                r#"{"title":"Gym","duration_min":60,"weekdays":["Mon","Thu"],
                    "start_day":"2026-10-12","time_of_day":"07:00:00"}"#,
            ),
        )
        .await;
        let id = serde_json::from_value::<Snapshot>(body).unwrap().series[0]
            .id
            .0
            .clone();

        let (_, body) = send(
            &router,
            "GET",
            "/api/v1/snapshot?from=2026-10-19&days=7",
            None,
        )
        .await;
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        let mut dates: Vec<String> = snapshot
            .tasks
            .iter()
            .filter(|task| task.source == todo_core::Source::Series)
            .map(|task| task.id.0.clone())
            .collect();
        dates.sort();
        assert_eq!(
            dates,
            vec![format!("{id}:2026-10-19"), format!("{id}:2026-10-22")],
            "the week asked for, not the week it was created in"
        );
        assert_invariant(&snapshot);
    }

    /// What a client cannot do: mark a day the series does not repeat, name a series it
    /// does not hold, send a date that is not a date, or create a rule that can never
    /// fire.
    #[tokio::test]
    async fn an_occurrence_the_rule_does_not_cover_is_refused() {
        let router = router_at(sunday_night());
        let (_, body) = send(
            &router,
            "POST",
            "/api/v1/series",
            Some(
                r#"{"title":"Gym","duration_min":60,"weekdays":["Mon"],
                    "start_day":"2026-10-12","time_of_day":"07:00:00"}"#,
            ),
        )
        .await;
        let id = serde_json::from_value::<Snapshot>(body).unwrap().series[0]
            .id
            .0
            .clone();

        for (path, body) in [
            (
                format!("/api/v1/series/{id}/dates/2026-10-13"),
                r#"{"kind":"done"}"#, // a Tuesday: not in the rule
            ),
            (
                format!("/api/v1/series/{id}/dates/2026-10-05"),
                r#"{"kind":"done"}"#, // a Monday, but before the series starts
            ),
            (
                format!("/api/v1/series/{id}/dates/2026-10-12"),
                r#"{"kind":"moved"}"#, // a move with no instant
            ),
            (
                format!("/api/v1/series/{id}/dates/2026-10-12"),
                r#"{"kind":"whenever"}"#, // a kind that is not one of the three
            ),
        ] {
            let (status, _) = send(&router, "PATCH", &path, Some(body)).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{path} with {body} should be refused"
            );
        }
        // A date that is not a date at all never reaches the app.
        let (status, _) = send(
            &router,
            "PATCH",
            &format!("/api/v1/series/{id}/dates/banana"),
            Some(r#"{"kind":"done"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // A series this user does not hold is a 404, not a 400: it is not there at all.
        let (status, _) = send(
            &router,
            "PATCH",
            "/api/v1/series/s-made-up/dates/2026-10-13",
            Some(r#"{"kind":"done"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // A series that could never fire is refused, and nothing is stored.
        let (status, _) = send(
            &router,
            "POST",
            "/api/v1/series",
            Some(r#"{"title":"Nothing","duration_min":30,"weekdays":[],"start_day":"2026-10-12"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = send(
            &router,
            "POST",
            "/api/v1/series",
            Some(
                r#"{"title":"Backwards","duration_min":30,"weekdays":["Mon"],
                    "start_day":"2026-10-12","until":"2026-10-01"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (_, body) = send(&router, "GET", "/api/v1/snapshot", None).await;
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        assert_eq!(
            snapshot.series.len(),
            1,
            "only the one that worked is stored"
        );
    }

    /// An occurrence row's id is not a task id: a task edit with one says where to send
    /// it instead of reporting a task that is not there.
    #[tokio::test]
    async fn patching_an_occurrence_as_a_task_is_a_400() {
        let router = router_at(sunday_night());
        let (_, body) = send(
            &router,
            "POST",
            "/api/v1/series",
            Some(
                r#"{"title":"Gym","duration_min":60,"weekdays":["Mon"],
                    "start_day":"2026-10-12","time_of_day":"07:00:00"}"#,
            ),
        )
        .await;
        let id = serde_json::from_value::<Snapshot>(body).unwrap().series[0]
            .id
            .0
            .clone();

        let (status, _) = send(
            &router,
            "PATCH",
            &format!("/api/v1/tasks/{id}:2026-10-12"),
            Some(r#"{"done":true}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // The occurrence itself is untouched by the refused edit.
        let (_, body) = send(&router, "GET", "/api/v1/snapshot", None).await;
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        assert!(
            !snapshot
                .tasks
                .iter()
                .any(|task| task.id.0 == format!("{id}:2026-10-12") && task.done)
        );
    }

    #[tokio::test]
    async fn sunday_night_tomorrow_is_monday_in_the_users_zone() {
        // A task due Monday comes back due at 17:00 New York on 2026-10-12.
        let router = router_at(sunday_night());
        let (_, body) = send(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(r#"{"title":"Tomorrow thing","duration_min":30,"when":{"kind":"due_by","time":"2026-10-12T21:00:00Z"},"label":"Other","priority":"low","source":"text"}"#),
        )
        .await;
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        let due = snapshot.tasks[0].when;
        assert_eq!(
            due,
            When::DueBy(Utc.with_ymd_and_hms(2026, 10, 12, 21, 0, 0).unwrap())
        );
        assert_invariant(&snapshot);
    }

    // ---- the audit findings this change fixes ----

    /// A TRANSPARENT event is "show this, do not treat it as busy". v1 already
    /// skipped it; the v2 import route must not turn a birthday into a task.
    #[tokio::test]
    async fn a_transparent_event_is_skipped_by_the_v2_import_route() {
        let router = router_at(sunday_night());
        let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//rambl//test//EN\n\
                   BEGIN:VEVENT\nUID:birthday@example.test\n\
                   SUMMARY:Someone's birthday\nTRANSP:TRANSPARENT\n\
                   DTSTART;TZID=America/New_York:20261014T090000\n\
                   DTEND;TZID=America/New_York:20261014T100000\n\
                   END:VEVENT\nEND:VCALENDAR\n";
        let (status, body) = import_ics(&router, ics).await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
        assert!(
            snapshot.tasks.is_empty(),
            "a transparent event is not an obligation, got {:?}",
            snapshot.tasks.iter().map(|t| &t.title).collect::<Vec<_>>()
        );
        assert_invariant(&snapshot);
    }

    /// The fixture is a real Outlook export with TZID times. The point of the fix is
    /// that 09:00 America/New_York stays 09:00 local rather than drifting to 13:00 UTC
    /// on the wall.
    #[tokio::test]
    async fn a_tzid_event_lands_at_the_right_local_time() {
        let router = router_at(sunday_night());
        let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//rambl//test//EN\n\
                   BEGIN:VEVENT\nUID:tzid@example.test\nSUMMARY:Standup\n\
                   DTSTART;TZID=America/New_York:20261013T090000\n\
                   DTEND;TZID=America/New_York:20261013T093000\n\
                   END:VEVENT\nEND:VCALENDAR\n";
        let (status, body) = import_ics(&router, ics).await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
        // 09:00 New York on 2026-10-13 is 13:00Z (EDT, UTC-4).
        assert_eq!(
            snapshot.tasks[0].when,
            When::At(Utc.with_ymd_and_hms(2026, 10, 13, 13, 0, 0).unwrap()),
            "the TZID offset was not applied"
        );
        assert_invariant(&snapshot);
    }

    /// The week.ics fixture carries TZID times for a whole week; every one of them
    /// must keep its local wall-clock time after the import.
    #[tokio::test]
    async fn the_week_fixture_keeps_every_tzid_event_at_its_local_time() {
        let router = router_at(sunday_night());
        let week = include_str!("../../tests/fixtures/week.ics");
        let (status, body) = import_ics(&router, week).await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body["snapshot"].clone()).unwrap();
        assert!(!snapshot.tasks.is_empty(), "the fixture imported nothing");

        let tz = chrono_tz::America::New_York;
        for task in &snapshot.tasks {
            let When::At(start) = task.when else {
                panic!(
                    "an imported event should be pinned, {} is {:?}",
                    task.title, task.when
                );
            };
            let local = start.with_timezone(&tz).time();
            // Working-hours events must sit inside 09:00-17:00 local. A TZID that
            // was treated as UTC lands at 05:00 or 13:00 and fails here.
            assert!(
                local >= chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap()
                    && local <= chrono::NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
                "{} is at {} local, outside working hours",
                task.title,
                local
            );
        }
        assert_invariant(&snapshot);
    }

    /// `?from=`/`?days=` bound the plan. The task list must not shrink with the window:
    /// a task the caller cannot see yet still exists.
    #[tokio::test]
    async fn the_snapshot_window_bounds_the_plan_but_not_the_tasks() {
        let router = router_at(sunday_night());
        let (_, _) = send(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(r#"{"title":"Write essay","duration_min":60,"when":{"kind":"anytime"},"label":"School","priority":"high","source":"text"}"#),
        )
        .await;

        let (status, body) = send(&router, "GET", "/api/v1/snapshot?days=7", None).await;
        assert_eq!(status, StatusCode::OK);
        let week: Snapshot = serde_json::from_value(body).unwrap();
        assert_eq!(week.tasks.len(), 1);
        assert_eq!(
            week.plan.placements.len(),
            1,
            "the week holds the placement"
        );
        assert_invariant(&week);

        // A window entirely in the past still lists the task, with no placement.
        let (status, body) = send(
            &router,
            "GET",
            "/api/v1/snapshot?from=2026-01-05&days=1",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let past: Snapshot = serde_json::from_value(body).unwrap();
        assert_eq!(past.tasks.len(), 1, "the task list is not windowed");
        assert!(
            past.plan.placements.is_empty(),
            "a placement outside the window should not be drawn"
        );
        assert_invariant(&past);
    }

    /// A pinned task is a fact about a fixed instant, so narrowing the plan window must
    /// never hide it: the section 9 invariant requires every open pinned task to show a
    /// placement, and the client still needs to know the time is taken.
    #[tokio::test]
    async fn a_pinned_task_keeps_its_placement_outside_the_window() {
        let router = router_at(sunday_night());
        let (_, _) = send(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(r#"{"title":"Dentist in March","duration_min":60,"when":{"kind":"at","time":"2027-03-04T15:00:00Z"},"label":"Health","priority":"low","source":"text"}"#),
        )
        .await;

        // A one-day window in October: nowhere near March 2027.
        let (status, body) = send(
            &router,
            "GET",
            "/api/v1/snapshot?from=2026-10-12&days=1",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(body).unwrap();
        assert_eq!(snapshot.tasks.len(), 1, "the task is still listed");
        assert_eq!(
            snapshot.plan.placements.len(),
            1,
            "a pinned task outside the window keeps its placement"
        );
        assert_invariant(&snapshot);
    }

    #[tokio::test]
    async fn a_bad_snapshot_window_is_a_400() {
        let router = router_at(sunday_night());
        for query in ["?days=0", "?days=9999", "?from=not-a-date"] {
            let (status, body) =
                send(&router, "GET", &format!("/api/v1/snapshot{query}"), None).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{query} should be rejected, got {body}"
            );
        }
    }

    /// Only pdf, eml and txt are documents Rambl reads. Anything else used to be read
    /// as text and handed to the judge as binary noise.
    #[tokio::test]
    async fn capture_accepts_only_pdf_eml_and_txt() {
        let router = router_at(sunday_night());
        for filename in [
            "notes.docx",
            "sheet.xlsx",
            "photo.png",
            "archive.zip",
            "noextension",
        ] {
            let boundary = "XRAMBLTYPE";
            let body = format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\r\nhello\r\n--{boundary}--\r\n"
            );
            let (status, _, _) = send_raw(
                &router,
                "POST",
                "/api/v1/capture",
                &format!("multipart/form-data; boundary={boundary}"),
                &body,
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{filename} should be refused, not read as text"
            );
        }
        // .txt is still fine.
        let boundary = "XRAMBLTXT";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"list.txt\"\r\n\r\nbuy milk\r\n--{boundary}--\r\n"
        );
        let (status, json, _) = send_raw(
            &router,
            "POST",
            "/api/v1/capture",
            &format!("multipart/form-data; boundary={boundary}"),
            &body,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let snapshot: Snapshot = serde_json::from_value(json["snapshot"].clone()).unwrap();
        assert_eq!(snapshot.tasks[0].source, Source::Text);
        assert_invariant(&snapshot);
    }

    /// A Jev outage is a 503 so the client retries; a Jev answer that makes no sense
    /// is a 502. Both used to be 502.
    ///
    /// The unreachable half uses a real connection to a port nothing is listening on,
    /// so it is a genuine `reqwest` connect error and the test needs no network.
    #[tokio::test]
    async fn a_jev_outage_is_unavailable_and_a_bad_answer_is_upstream() {
        let refused = reqwest::get("http://127.0.0.1:1/jev")
            .await
            .expect_err("nothing listens on port 1");
        let outage = upstream(anyhow::Error::new(refused));
        assert!(
            matches!(outage.0.code, ErrorCode::Unavailable),
            "an unreachable Jev is a dependency being down, got {:?}",
            outage.0.code
        );

        let bad_answer = upstream(anyhow::anyhow!("Jev answer missing is_a_task"));
        assert!(
            matches!(bad_answer.0.code, ErrorCode::Upstream),
            "a malformed answer is a bad gateway, got {:?}",
            bad_answer.0.code
        );
    }

    /// The v2 store is SQLite on disk by default, so a restart must not lose tasks.
    /// This is the one test that would have caught the app running in memory.
    #[tokio::test]
    async fn tasks_survive_a_restart_on_disk() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("rambl.db");

        let mut first = App::new(
            SqliteStore::open(&path).expect("open"),
            Arc::new(FixedClock(sunday_night())),
        );
        let snapshot = first
            .create(
                TaskDraft {
                    title: "Write the lab report".into(),
                    duration_min: 60,
                    when: When::Anytime,
                    label: todo_core::Label::School,
                    priority: todo_core::Priority::High,
                    source: Source::Text,
                },
                auth::LOCAL_USER,
                chrono_tz::America::New_York,
            )
            .expect("create");

        assert_eq!(snapshot.tasks.len(), 1);
        assert_invariant(&snapshot);
        drop(first);

        // A brand new store over the same file: the task has to still be there.
        let second = App::new(
            SqliteStore::open(&path).expect("reopen"),
            Arc::new(FixedClock(sunday_night())),
        );
        let reread = second
            .snapshot(auth::LOCAL_USER, chrono_tz::America::New_York)
            .expect("snapshot");
        assert_eq!(
            reread.tasks.len(),
            1,
            "the task was lost when the store closed"
        );
        assert_eq!(reread.tasks[0].title, "Write the lab report");
        assert_invariant(&reread);
    }

    /// `RAMBL_DEFAULT_TZ` is the v2 name and `TZ_NAME` is what the old `.env` files set,
    /// so both have to work or an existing deployment silently moves to New York.
    #[test]
    fn tz_name_still_works_as_the_fallback() {
        let london = || Some("Europe/London".to_owned());
        let berlin = || Some("Europe/Berlin".to_owned());

        assert_eq!(zone_from_env(london(), berlin()), chrono_tz::Europe::London);
        assert_eq!(
            zone_from_env(None, berlin()),
            chrono_tz::Europe::Berlin,
            "TZ_NAME is the fallback when RAMBL_DEFAULT_TZ is unset"
        );
        assert_eq!(
            zone_from_env(None, None),
            chrono_tz::America::New_York,
            "with neither configured we fall back to New York"
        );
        assert_eq!(
            zone_from_env(Some("Not/AZone".to_owned()), None),
            chrono_tz::America::New_York,
            "an unreadable value falls through instead of failing the request"
        );
    }

    // ---- accounts ----

    /// A router with real (non-local) sign-in and a fake verifier, so the tests
    /// exercise the session path without a Google project or a network call.
    fn accounts_router(now: chrono::DateTime<Utc>, sub: &str, email: &str) -> Router {
        accounts_router_with_admins(now, sub, email, None)
    }

    fn accounts_router_with_admins(
        now: chrono::DateTime<Utc>,
        sub: &str,
        email: &str,
        admins: Option<Vec<String>>,
    ) -> Router {
        let verifier = auth::FakeVerifier {
            answer: Ok(auth::TokenInfo::for_test(sub, email)),
        };
        super::router_with_verifier(
            App::new(
                Box::new(MemoryStore::default()) as Box<dyn Store>,
                Arc::new(FixedClock(now)),
            ),
            AnyJudge::Fake(judge::FakeJudge),
            Box::new(verifier),
            false,
            Caps::default(),
            admins,
            TrustedProxies::default(),
        )
    }

    async fn send_as(
        router: &Router,
        method: &str,
        uri: &str,
        body: Option<&str>,
        token: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-rambl-tz", "America/New_York");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let body = match body {
            Some(text) => {
                builder = builder.header("content-type", "application/json");
                Body::from(text.to_owned())
            }
            None => Body::empty(),
        };
        let response = router
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    async fn sign_in(router: &Router) -> String {
        let (status, body) = send_as(
            router,
            "POST",
            "/api/v1/auth/google",
            Some(r#"{"credential":"fake-id-token"}"#),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "sign in failed: {body}");
        body["token"].as_str().expect("a session token").to_owned()
    }

    const DRAFT: &str = r#"{"title":"Write the report","duration_min":30,"when":{"kind":"anytime"},"label":"Other","priority":"low","source":"text"}"#;

    /// The verifier is behind a trait, so this runs with no network and no Google
    /// project: the answer is fixed by the fake.
    #[tokio::test]
    async fn signing_in_returns_a_token_and_the_user() {
        let router = accounts_router(sunday_night(), "google-1", "Sam@Example.com");
        let (status, body) = send_as(
            &router,
            "POST",
            "/api/v1/auth/google",
            Some(r#"{"credential":"fake-id-token"}"#),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["token"].as_str().is_some_and(|t| !t.is_empty()));
        assert_eq!(body["user"]["id"], "google-1");
        // Emails are stored lower-cased so the same person cannot end up twice.
        assert_eq!(body["user"]["email"], "sam@example.com");

        // The token works.
        let (status, body) = send_as(
            &router,
            "GET",
            "/api/v1/me",
            None,
            Some(body["token"].as_str().unwrap()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["id"], "google-1");
    }

    #[tokio::test]
    async fn without_credentials_uses_anonymous_access() {
        let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
        // A well-formed body, so the request reaches the handler rather than being
        // rejected by an extractor. Otherwise `Json` answers 415 for a missing
        // content-type and the auth check never runs.
        for (method, uri, body) in [
            ("GET", "/api/v1/snapshot", None),
            ("POST", "/api/v1/tasks", Some(DRAFT)),
            ("POST", "/api/v1/capture", Some(r#"{"text":"buy milk"}"#)),
            ("PUT", "/api/v1/tasks/order", Some(r#"{"ids":[]}"#)),
            ("GET", "/api/v1/calendar.ics", None),
        ] {
            let (status, _, _) = send_raw_status(&router, method, uri, body).await;
            assert_eq!(status, StatusCode::OK, "{method} {uri} was rejected");
        }
    }

    /// No `RAMBL_ADMIN_EMAILS` means the whole admin surface is off: the page is
    /// 404 even to the person who would have been an admin.
    #[tokio::test]
    async fn admin_is_off_when_no_emails_are_configured() {
        let router = accounts_router(sunday_night(), "google-1", "boss@example.com");
        let token = sign_in(&router).await;
        let (status, _) = send_as(&router, "GET", "/admin", None, Some(&token)).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "the page must be 404 without RAMBL_ADMIN_EMAILS"
        );
        let (status, _) = send_as(&router, "GET", "/api/v1/admin/blocks", None, Some(&token)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "the API must be 404 too");
    }

    /// Only a signed-in user whose email is in `RAMBL_ADMIN_EMAILS` sees the admin
    /// surface. Everyone else gets 404 — friends, anonymous visitors, and signed-in
    /// users with unlisted emails.
    #[tokio::test]
    async fn non_admins_get_404_on_the_page_and_the_api() {
        let router = accounts_router_with_admins(
            sunday_night(),
            "google-1",
            "sam@example.com",
            Some(vec!["boss@example.com".into()]),
        );
        let token = sign_in(&router).await;

        let (status, _) = send_as(&router, "GET", "/admin", None, Some(&token)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "a non-admin gets no page");
        for (method, uri, body) in [
            ("GET", "/api/v1/admin/usage", None),
            ("GET", "/api/v1/admin/blocks", None),
            (
                "POST",
                "/api/v1/admin/block",
                Some(r#"{"kind":"ip","value":"203.0.113.9"}"#),
            ),
        ] {
            let (status, _) = send_as(&router, method, uri, body, Some(&token)).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{method} {uri} must hide from non-admins"
            );
        }

        // Anonymous visitors never see data, but they do get the sign-in shell — the
        // page signs in the same way the main page does.
        let (status, _) = send_as(&router, "GET", "/api/v1/admin/blocks", None, None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "anonymous access is never admin"
        );
        let (status, _) = send_as(&router, "GET", "/admin", None, None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the one place that must load is the shell itself"
        );
    }

    /// An admin can read today's usage, block and unblock an address, and move a
    /// user between plans — everything the console can, over the web.
    #[tokio::test]
    async fn an_admin_can_block_unblock_see_usage_and_set_plans() {
        let router = accounts_router_with_admins(
            sunday_night(),
            "google-1",
            "boss@example.com",
            Some(vec!["boss@example.com".into()]),
        );
        let token = sign_in(&router).await;
        let bearer = format!("Bearer {token}");

        // A capture today, so usage has a row.
        let (status, _, _) = call(
            &router,
            "POST",
            "/api/v1/capture",
            Some(r#"{"text":"buy milk tomorrow"}"#),
            &[("authorization", &bearer)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, usage) =
            send_as(&router, "GET", "/api/v1/admin/usage", None, Some(&token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            usage["per_plan"]["free"], 1,
            "the free user's capture is one Jev call"
        );
        assert_eq!(usage["users"][0]["id"], "google-1");
        assert_eq!(usage["users"][0]["plan"], "free");

        let (status, body) = send_as(
            &router,
            "POST",
            "/api/v1/admin/block",
            Some(r#"{"kind":"ip","value":"203.0.113.9"}"#),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["message"], "blocked ip 203.0.113.9");

        let (status, blocks) =
            send_as(&router, "GET", "/api/v1/admin/blocks", None, Some(&token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(blocks[0]["kind"], "ip");
        assert_eq!(blocks[0]["value"], "203.0.113.9");

        let (status, body) = send_as(
            &router,
            "POST",
            "/api/v1/admin/unblock",
            Some(r#"{"kind":"ip","value":"203.0.113.9"}"#),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["message"], "unblocked ip 203.0.113.9");
        let (status, blocks) =
            send_as(&router, "GET", "/api/v1/admin/blocks", None, Some(&token)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(blocks.as_array().unwrap().is_empty());

        let (status, body) = send_as(
            &router,
            "POST",
            "/api/v1/admin/plan",
            Some(r#"{"who":"google-1","plan":"paid"}"#),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["message"], "google-1 is now on the paid plan");

        // The plan change is visible to the user themselves (`/me` keeps the
        // enum's serialized name; the admin API speaks `as_str`).
        let (status, me) = send_as(&router, "GET", "/api/v1/me", None, Some(&token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["plan"], "Paid");
    }

    /// A non-IP `CF-Connecting-IP` is never stored, so it cannot be echoed on the
    /// admin page (the stored-XSS issue Pam raised on #161).
    #[tokio::test]
    async fn a_non_ip_proxy_header_is_never_stored_or_echoed() {
        let router = accounts_router_with_admins(
            sunday_night(),
            "google-1",
            "boss@example.com",
            Some(vec!["boss@example.com".into()]),
        );
        let token = sign_in(&router).await;
        let bearer = format!("Bearer {token}");

        let payload = "<img src=x onerror=alert(1)>";
        // From the tunnel, so the peer is trusted and the value alone is refused.
        let (status, _, _) = call_from(
            &router,
            "POST",
            "/api/v1/capture",
            Some(r#"{"text":"buy milk tomorrow"}"#),
            &[("authorization", &bearer), ("cf-connecting-ip", payload)],
            Some(TUNNEL),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "the capture itself must not fail");

        let (status, usage) =
            send_as(&router, "GET", "/api/v1/admin/usage", None, Some(&token)).await;
        assert_eq!(status, StatusCode::OK);
        let ips: Vec<String> = usage["ips"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["ip"].as_str().unwrap().to_owned())
            .collect();
        assert!(
            ips.iter().all(|ip| !ip.contains('<') && !ip.contains('>')),
            "a header string must never reach the admin page: {ips:?}"
        );
    }

    #[tokio::test]
    async fn invalid_bearer_token_is_401() {
        let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
        let (status, _, _) = send_raw_status_with_headers(
            &router,
            "GET",
            "/api/v1/snapshot",
            None,
            &[("authorization", "Bearer expired")],
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    async fn send_raw_status(
        router: &Router,
        method: &str,
        uri: &str,
        json: Option<&str>,
    ) -> (StatusCode, String, String) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-rambl-tz", "America/New_York");
        let body = match json {
            Some(text) => {
                builder = builder.header("content-type", "application/json");
                Body::from(text.to_owned())
            }
            None => Body::empty(),
        };
        let response = router
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            String::new(),
            String::from_utf8_lossy(&bytes).to_string(),
        )
    }

    async fn send_raw_status_with_headers(
        router: &Router,
        method: &str,
        uri: &str,
        json: Option<&str>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, String, String) {
        let mut builder = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let body = json.map_or_else(Body::empty, |text| Body::from(text.to_owned()));
        let response = router
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            String::new(),
            String::from_utf8_lossy(&bytes).to_string(),
        )
    }

    /// The promise the whole feature rests on.
    #[tokio::test]
    async fn two_users_never_see_each_others_tasks() {
        let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
        let sam = sign_in(&router).await;

        // Sign in a second person. The fake verifier answers the same way, so the two
        // sessions differ by user id rather than by credential; give the second one a
        // distinct id by re-routing with its own verifier.
        let second = super::router_with_verifier(
            App::new(
                Box::new(MemoryStore::default()) as Box<dyn Store>,
                Arc::new(FixedClock(sunday_night())),
            ),
            AnyJudge::Fake(judge::FakeJudge),
            Box::new(auth::FakeVerifier {
                answer: Ok(auth::TokenInfo::for_test("google-2", "alex@example.com")),
            }),
            false,
            Caps::default(),
            None,
            TrustedProxies::default(),
        );
        let alex = sign_in(&second).await;

        let (status, _) = send_as(&router, "POST", "/api/v1/tasks", Some(DRAFT), Some(&sam)).await;
        assert_eq!(status, StatusCode::OK);

        // Sam sees her task.
        let (status, body) = send_as(&router, "GET", "/api/v1/snapshot", None, Some(&sam)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["tasks"].as_array().unwrap().len(), 1);

        // Alex, on a different router and a different store, cannot see it, and her
        // own snapshot is empty.
        let (status, body) = send_as(&second, "GET", "/api/v1/snapshot", None, Some(&alex)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body["tasks"].as_array().unwrap().is_empty(),
            "one person saw another's tasks: {body}"
        );

        // And Sam's token is useless on Alex's server.
        let (status, _) = send_as(&second, "GET", "/api/v1/snapshot", None, Some(&sam)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let _ = sam;
    }

    /// A session past its expiry is not a session.
    #[tokio::test]
    async fn an_expired_session_is_401() {
        let store = MemoryStore::default();
        let app = App::new(Box::new(store) as Box<dyn Store>, Arc::new(SystemClock));
        let token = auth::new_token();
        let user = auth::Me {
            id: "google-1".into(),
            email: "sam@example.com".into(),
            name: "Sam".into(),
            picture: None,
            plan: auth::Plan::Free,
        };
        // A session that ended a minute ago.
        let past = Utc::now() - chrono::Duration::minutes(1);
        app.store()
            .upsert_user_and_session(&user, &token, past)
            .expect("store the session");

        assert!(
            auth::session_user(app.store(), &token, Utc::now())
                .unwrap()
                .is_none(),
            "an expired session still resolves to a user"
        );
        assert!(
            auth::session_user(app.store(), &token, past - chrono::Duration::minutes(1))
                .unwrap()
                .is_some(),
            "a live session should resolve"
        );
    }

    /// Deleting an account takes their tasks with it.
    #[tokio::test]
    async fn deleting_an_account_removes_their_tasks() {
        let store = MemoryStore::default();
        let app = App::new(Box::new(store) as Box<dyn Store>, Arc::new(SystemClock));
        let user = "google-1";
        app.store()
            .put(
                user,
                todo_core::Task {
                    id: todo_core::TaskId("t1".into()),
                    title: DRAFT_TITLE.into(),
                    duration_min: 30,
                    when: When::Anytime,
                    done: false,
                    order: 0,
                    label: Label::Other,
                    priority: Priority::Low,
                    source: Source::Text,
                    created: Utc::now(),
                    updated_at: Utc::now(),
                    due_tz: None,
                    external_uid: None,
                },
            )
            .unwrap();
        assert_eq!(app.store().tasks(user).unwrap().len(), 1);

        app.store().delete_user(user).expect("delete");
        assert!(
            app.store().tasks(user).unwrap().is_empty(),
            "the tasks outlived the account"
        );
        assert!(
            app.store()
                .session_user(&auth::new_token(), Utc::now())
                .unwrap()
                .is_none()
        );
    }

    const DRAFT_TITLE: &str = "Write the report";

    /// Past the daily cap, capture still works but Jev is not called.
    #[tokio::test]
    async fn over_the_daily_cap_capture_falls_back_and_says_so() {
        let store = MemoryStore::default();
        let app = App::new(
            Box::new(store) as Box<dyn Store>,
            Arc::new(FixedClock(sunday_night())),
        );
        // A cap of one: the first capture is judged, the second falls back.
        let router = super::router_with_verifier(
            app,
            AnyJudge::Fake(judge::FakeJudge),
            Box::new(auth::FakeVerifier {
                answer: Ok(auth::TokenInfo::for_test("google-1", "sam@example.com")),
            }),
            false,
            Caps {
                free: 1,
                ..Caps::default()
            },
            None,
            TrustedProxies::default(),
        );
        let token = sign_in(&router).await;

        let (status, first) = send_as(
            &router,
            "POST",
            "/api/v1/capture",
            Some(r#"{"text":"buy milk tomorrow"}"#),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(
            first["jev_fallback"], false,
            "the first capture is under the cap"
        );

        let (status, second) = send_as(
            &router,
            "POST",
            "/api/v1/capture",
            Some(r#"{"text":"call the dentist"}"#),
            Some(&token),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the cap must not fail the capture: {second}"
        );
        assert_eq!(
            second["jev_fallback"], true,
            "over the cap, and the response must say so: {second}"
        );
        assert!(
            second["created"].as_array().is_some_and(|c| !c.is_empty()),
            "the fallback still creates the task"
        );
    }

    // ---- anonymous use ----

    /// One request with arbitrary headers, returning its status, headers and JSON body.
    async fn call(
        router: &Router,
        method: &str,
        uri: &str,
        body: Option<&str>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
        call_from(router, method, uri, body, headers, None).await
    }

    /// [`call`], with the socket the request really arrived on. The peer decides
    /// whether `CF-Connecting-IP` is believed at all.
    async fn call_from(
        router: &Router,
        method: &str,
        uri: &str,
        body: Option<&str>,
        headers: &[(&str, &str)],
        peer: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-rambl-tz", "America/New_York");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let mut request = match body {
            Some(text) => builder
                .header("content-type", "application/json")
                .body(Body::from(text.to_owned()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        if let Some(peer) = peer {
            request.extensions_mut().insert(ConnectInfo(
                peer.parse::<SocketAddr>().expect("a socket address"),
            ));
        }
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or_default(),
        )
    }

    /// The anonymous id the server attached to a response.
    fn anonymous_id_of(headers: &axum::http::HeaderMap) -> String {
        headers
            .get("x-rambl-anon")
            .expect("the response names the anonymous id")
            .to_str()
            .expect("ASCII")
            .to_owned()
    }

    const FOREIGN_ID: &str = "anon_00000000000000000000000000000000";

    /// A forged id must never reach someone else's tasks. Each forgery is a value a
    /// client could plausibly send: another account's id, junk, or a well-formed id the
    /// server never issued.
    #[tokio::test]
    async fn forged_anonymous_ids_never_reach_someone_elses_tasks() {
        let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
        let sam = sign_in(&router).await;
        let bearer = format!("Bearer {sam}");
        let (status, _, _) = call(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(DRAFT),
            &[("authorization", &bearer)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        for forged in ["google-1", "anon_zz", "anon_ABCDEF", FOREIGN_ID] {
            let (status, headers, body) = call(
                &router,
                "GET",
                "/api/v1/snapshot",
                None,
                &[("x-rambl-anon", forged)],
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{forged}: {body}");
            assert!(
                body["tasks"].as_array().unwrap().is_empty(),
                "a forged id ({forged}) reached someone's tasks: {body}"
            );
            assert_ne!(
                anonymous_id_of(&headers),
                forged,
                "the server must issue its own id, not echo a forged one"
            );
        }
    }

    /// An anonymous id the server issued keeps its own tasks from one request to the
    /// next.
    #[tokio::test]
    async fn an_issued_anonymous_id_keeps_its_tasks() {
        let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
        let (_, headers, _) = call(&router, "GET", "/api/v1/snapshot", None, &[]).await;
        let id = anonymous_id_of(&headers);

        let (status, _, _) = call(
            &router,
            "POST",
            "/api/v1/tasks",
            Some(DRAFT),
            &[("x-rambl-anon", &id)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, _, body) = call(
            &router,
            "GET",
            "/api/v1/snapshot",
            None,
            &[("x-rambl-anon", &id)],
        )
        .await;
        assert_eq!(body["tasks"].as_array().unwrap().len(), 1, "{body}");
    }

    fn router_with_caps(store: MemoryStore, caps: Caps) -> Router {
        super::router_with_verifier(
            App::new(
                Box::new(store) as Box<dyn Store>,
                Arc::new(FixedClock(sunday_night())),
            ),
            AnyJudge::Fake(judge::FakeJudge),
            Box::new(auth::FakeVerifier {
                answer: Err("unused".into()),
            }),
            false,
            caps,
            None,
            TrustedProxies::default(),
        )
    }

    /// Watches whether the anonymous-id machinery ever reached the store.
    struct CountingAnonStore {
        inner: MemoryStore,
        /// Calls to `ensure_anonymous`, i.e. anonymous user rows created.
        issued: Arc<AtomicUsize>,
        /// Calls to `record_anon_issued`, i.e. per-address allowance use.
        counted: Arc<AtomicUsize>,
    }

    impl Store for CountingAnonStore {
        fn tasks(&self, user: &str) -> Result<Vec<todo_core::Task>, StoreError> {
            self.inner.tasks(user)
        }
        fn put(&self, user: &str, task: todo_core::Task) -> Result<(), StoreError> {
            self.inner.put(user, task)
        }
        fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError> {
            self.inner.delete(user, id)
        }
        fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError> {
            self.inner.set_order(user, ids)
        }
        fn find_by_uid(
            &self,
            user: &str,
            uid: &str,
        ) -> Result<Option<todo_core::Task>, StoreError> {
            self.inner.find_by_uid(user, uid)
        }
        fn series(&self, user: &str) -> Result<Vec<todo_core::Series>, StoreError> {
            self.inner.series(user)
        }
        fn put_series(&self, user: &str, series: todo_core::Series) -> Result<(), StoreError> {
            self.inner.put_series(user, series)
        }
        fn delete_series(&self, user: &str, id: &todo_core::SeriesId) -> Result<bool, StoreError> {
            self.inner.delete_series(user, id)
        }
        fn exceptions(
            &self,
            user: &str,
            id: &todo_core::SeriesId,
        ) -> Result<Vec<todo_core::SeriesException>, StoreError> {
            self.inner.exceptions(user, id)
        }
        fn put_exception(
            &self,
            user: &str,
            id: &todo_core::SeriesId,
            exception: todo_core::SeriesException,
        ) -> Result<(), StoreError> {
            self.inner.put_exception(user, id, exception)
        }
        fn delete_exception(
            &self,
            user: &str,
            id: &todo_core::SeriesId,
            date: chrono::NaiveDate,
        ) -> Result<bool, StoreError> {
            self.inner.delete_exception(user, id, date)
        }
        fn upsert_user_and_session(
            &self,
            user: &crate::v2::auth::Me,
            token: &str,
            expires_at: chrono::DateTime<Utc>,
        ) -> Result<(), StoreError> {
            self.inner.upsert_user_and_session(user, token, expires_at)
        }
        fn delete_session(&self, token: &str) -> Result<(), StoreError> {
            self.inner.delete_session(token)
        }
        fn session_user(
            &self,
            token: &str,
            now: chrono::DateTime<Utc>,
        ) -> Result<Option<crate::v2::auth::Me>, StoreError> {
            self.inner.session_user(token, now)
        }
        fn delete_user(&self, user: &str) -> Result<(), StoreError> {
            self.inner.delete_user(user)
        }
        fn captures_today(&self, user: &str, day: NaiveDate) -> Result<u32, StoreError> {
            self.inner.captures_today(user, day)
        }
        fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError> {
            self.inner.adopt_anonymous(anonymous, user)
        }
        fn user_plan(&self, user: &str) -> Result<crate::v2::auth::Plan, StoreError> {
            self.inner.user_plan(user)
        }
        fn set_user_plan(&self, user: &str, plan: crate::v2::auth::Plan) -> Result<(), StoreError> {
            self.inner.set_user_plan(user, plan)
        }
        fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError> {
            self.inner.anonymous_exists(user)
        }
        fn ensure_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
            self.issued.fetch_add(1, Ordering::SeqCst);
            self.inner.ensure_anonymous(user, now)
        }
        fn touch_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
            self.inner.touch_anonymous(user, now)
        }
        fn expire_anonymous(&self, cutoff: DateTime<Utc>) -> Result<usize, StoreError> {
            self.inner.expire_anonymous(cutoff)
        }
        fn record_capture(&self, user: &str, ip: &str, day: NaiveDate) -> Result<(), StoreError> {
            self.inner.record_capture(user, ip, day)
        }
        fn ip_captures_today(&self, user: &str, day: NaiveDate) -> Result<u32, StoreError> {
            self.inner.ip_captures_today(user, day)
        }
        fn record_anon_issued(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError> {
            self.counted.fetch_add(1, Ordering::SeqCst);
            self.inner.record_anon_issued(ip, day)
        }
        fn block(&self, target: &crate::v2::store::Block) -> Result<(), StoreError> {
            self.inner.block(target)
        }
        fn unblock(&self, target: &crate::v2::store::Block) -> Result<bool, StoreError> {
            self.inner.unblock(target)
        }
        fn is_blocked(&self, user: &str, ip: &str) -> Result<bool, StoreError> {
            self.inner.is_blocked(user, ip)
        }
        fn blocks(&self) -> Result<Vec<crate::v2::store::Block>, StoreError> {
            self.inner.blocks()
        }
        fn usage_today(
            &self,
            day: NaiveDate,
            top: usize,
        ) -> Result<crate::v2::store::Usage, StoreError> {
            self.inner.usage_today(day, top)
        }
        fn user_id_for_email(&self, email: &str) -> Result<Option<String>, StoreError> {
            self.inner.user_id_for_email(email)
        }
    }

    /// Local mode (`GOOGLE_CLIENT_ID` unset) answers as the fixed local user, so
    /// the anonymous-id middleware must not issue ids — or spend the per-address
    /// allowance on ids nobody uses. A fresh browser profile is a request with no
    /// `X-Rambl-Anon`; 30 of them is past the default 20-a-day allowance, and
    /// Dwight found the install locked itself out there.
    #[tokio::test]
    async fn local_mode_issues_no_anonymous_ids() {
        let issued = Arc::new(AtomicUsize::new(0));
        let counted = Arc::new(AtomicUsize::new(0));
        let store = CountingAnonStore {
            inner: MemoryStore::default(),
            issued: Arc::clone(&issued),
            counted: Arc::clone(&counted),
        };
        let router = super::router_with_verifier(
            App::new(
                Box::new(store) as Box<dyn Store>,
                Arc::new(FixedClock(sunday_night())),
            ),
            AnyJudge::Fake(judge::FakeJudge),
            Box::new(auth::FakeVerifier {
                answer: Err("unused".into()),
            }),
            true, // local mode
            Caps::default(),
            None,
            TrustedProxies::default(),
        );
        let from = [("cf-connecting-ip", "203.0.113.9")];
        for _ in 0..30 {
            let (status, headers, _) = call(&router, "GET", "/api/v1/snapshot", None, &from).await;
            assert_eq!(
                status,
                StatusCode::OK,
                "local mode must never refuse a fresh profile"
            );
            assert!(
                !headers.contains_key(ANON_HEADER),
                "local mode must not hand out anonymous ids"
            );
        }
        assert_eq!(
            issued.load(Ordering::SeqCst),
            0,
            "no anonymous user row may be created in local mode"
        );
        assert_eq!(
            counted.load(Ordering::SeqCst),
            0,
            "no per-address issuance may be counted in local mode"
        );
    }

    /// The tunnel's socket on this machine, the peer production requests arrive on.
    const TUNNEL: &str = "127.0.0.1:41234";

    /// Past the per-address allowance, a client gets no new anonymous id.
    #[tokio::test]
    async fn new_anonymous_ids_are_limited_per_address() {
        let router = router_with_caps(
            MemoryStore::default(),
            Caps {
                anon_ids_per_ip: 2,
                ..Caps::default()
            },
        );
        let from = [("cf-connecting-ip", "203.0.113.7")];
        for _ in 0..2 {
            let (status, _, _) = call_from(
                &router,
                "GET",
                "/api/v1/snapshot",
                None,
                &from,
                Some(TUNNEL),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, _, _) = call_from(
            &router,
            "GET",
            "/api/v1/snapshot",
            None,
            &from,
            Some(TUNNEL),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let other = [("cf-connecting-ip", "203.0.113.8")];
        let (status, _, _) = call_from(
            &router,
            "GET",
            "/api/v1/snapshot",
            None,
            &other,
            Some(TUNNEL),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "another address is not affected");
    }

    /// The reason the header is only read from a trusted peer: someone on the LAN
    /// sends a fresh address every request and must not get a fresh allowance with it.
    #[tokio::test]
    async fn a_direct_client_cannot_buy_allowance_by_forging_the_proxy_header() {
        let router = router_with_caps(
            MemoryStore::default(),
            Caps {
                anon_ids_per_ip: 1,
                ..Caps::default()
            },
        );
        let direct = Some("198.51.100.4:5555");
        let (status, _, _) = call_from(
            &router,
            "GET",
            "/api/v1/snapshot",
            None,
            &[("cf-connecting-ip", "203.0.113.1")],
            direct,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the first new id is within the allowance"
        );

        for forged in ["203.0.113.2", "203.0.113.3"] {
            let (status, _, _) = call_from(
                &router,
                "GET",
                "/api/v1/snapshot",
                None,
                &[("cf-connecting-ip", forged)],
                direct,
            )
            .await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{forged} bought a second new id from one address"
            );
        }
    }

    /// Past the anonymous cap, capture still works but is judged by `FakeJudge`.
    #[tokio::test]
    async fn anonymous_capture_falls_back_past_the_anonymous_cap() {
        let router = router_with_caps(
            MemoryStore::default(),
            Caps {
                anon: 1,
                ..Caps::default()
            },
        );
        let (_, headers, first) = call(
            &router,
            "POST",
            "/api/v1/capture",
            Some(r#"{"text":"buy milk tomorrow"}"#),
            &[],
        )
        .await;
        assert_eq!(first["jev_fallback"], false, "{first}");
        let id = anonymous_id_of(&headers);

        let (status, _, second) = call(
            &router,
            "POST",
            "/api/v1/capture",
            Some(r#"{"text":"call the dentist"}"#),
            &[("x-rambl-anon", &id)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{second}");
        assert_eq!(second["jev_fallback"], true, "{second}");
    }

    /// A blocked user still uses the app, but capture is judged by `FakeJudge`.
    #[tokio::test]
    async fn a_blocked_user_captures_with_fakejudge() {
        let store = MemoryStore::default();
        store
            .block(&crate::v2::store::Block::User("google-1".into()))
            .unwrap();
        let router = super::router_with_verifier(
            App::new(
                Box::new(store) as Box<dyn Store>,
                Arc::new(FixedClock(sunday_night())),
            ),
            AnyJudge::Fake(judge::FakeJudge),
            Box::new(auth::FakeVerifier {
                answer: Ok(auth::TokenInfo::for_test("google-1", "sam@example.com")),
            }),
            false,
            Caps::default(),
            None,
            TrustedProxies::default(),
        );
        let token = sign_in(&router).await;
        let bearer = format!("Bearer {token}");
        let (status, _, body) = call(
            &router,
            "POST",
            "/api/v1/capture",
            Some(r#"{"text":"buy milk tomorrow"}"#),
            &[("authorization", &bearer)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["jev_fallback"], true,
            "a blocked user is not judged by Jev"
        );
    }

    /// A blocked address is judged by `FakeJudge` whoever sends from it.
    #[tokio::test]
    async fn a_blocked_address_captures_with_fakejudge() {
        let store = MemoryStore::default();
        store
            .block(&crate::v2::store::Block::Ip("203.0.113.9".into()))
            .unwrap();
        let router = router_with_caps(store, Caps::default());
        let (status, _, body) = call_from(
            &router,
            "POST",
            "/api/v1/capture",
            Some(r#"{"text":"buy milk tomorrow"}"#),
            &[("cf-connecting-ip", "203.0.113.9")],
            Some(TUNNEL),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["jev_fallback"], true, "{body}");
    }

    /// With no Google client id configured the page is told so, and shows no sign-in.
    #[tokio::test]
    async fn config_without_a_google_client_id_is_null() {
        let router = accounts_router(sunday_night(), "google-1", "sam@example.com");
        let (status, _, body) = call(&router, "GET", "/api/v1/config", None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["google_client_id"].is_null(), "{body}");
    }

    /// Headers carrying `CF-Connecting-IP: 203.0.113.5`.
    fn from_cloudflare() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", HeaderValue::from_static("203.0.113.5"));
        headers
    }

    /// The tunnel runs on this machine, so a loopback peer may name the client.
    #[test]
    fn the_proxy_header_is_honoured_from_loopback() {
        let peer = Some("127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(
            client_ip(&from_cloudflare(), peer, &TrustedProxies::default()),
            "203.0.113.5"
        );
        // The whole loopback network, and IPv6 loopback, are the same machine.
        for loopback in ["127.0.0.5", "::1"] {
            let peer: IpAddr = loopback.parse().unwrap();
            assert_eq!(
                client_ip(&from_cloudflare(), Some(peer), &TrustedProxies::default()),
                "203.0.113.5",
                "{loopback} is this machine too"
            );
        }
    }

    /// Anyone reaching the server directly can send any address, so from a peer we do
    /// not trust the header is worth nothing: it is the socket that counts.
    #[test]
    fn the_proxy_header_is_ignored_from_an_untrusted_peer() {
        let peer = Some("198.51.100.4".parse::<IpAddr>().unwrap());
        assert_eq!(
            client_ip(&from_cloudflare(), peer, &TrustedProxies::default()),
            "198.51.100.4"
        );

        let trusted = TrustedProxies::from_list(Some("203.0.113.0/24"));
        assert_eq!(
            client_ip(&from_cloudflare(), peer, &trusted),
            "198.51.100.4",
            "a different address must not become trusted by another entry"
        );

        // Nothing at all to fall back on, and no peer to check: the header is still
        // not taken, because no socket vouched for it.
        assert_eq!(
            client_ip(&from_cloudflare(), None, &TrustedProxies::default()),
            "unknown"
        );
    }

    /// `RAMBL_TRUSTED_PROXIES` names the proxies allowed to speak for the client.
    #[test]
    fn a_listed_proxy_may_name_the_client() {
        let trusted = TrustedProxies::from_list(Some("198.51.100.4, 10.0.0.0/8, fd00::/8"));
        for peer in ["198.51.100.4", "10.1.2.3", "fd00::1"] {
            let peer: IpAddr = peer.parse().unwrap();
            assert_eq!(
                client_ip(&from_cloudflare(), Some(peer), &trusted),
                "203.0.113.5",
                "{peer} is listed, so it may name the client"
            );
        }
        let edge = Some("198.51.100.5".parse::<IpAddr>().unwrap());
        assert_eq!(
            client_ip(&from_cloudflare(), edge, &trusted),
            "198.51.100.5",
            "an unlisted neighbour is not a proxy"
        );
    }

    /// A trusted proxy that sends something that is not an address must not get it
    /// stored: the value is echoed on the admin page, so a client must not be able to
    /// smuggle an HTML string in there.
    #[test]
    fn a_header_that_is_not_an_address_falls_back_to_the_peer() {
        let peer = Some("127.0.0.1".parse::<IpAddr>().unwrap());
        for junk in [
            "<img src=x onerror=alert(1)>",
            "not-an-ip",
            "203.0.113.5 203.0.113.6",
            "  ",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("cf-connecting-ip", HeaderValue::from_static(junk));
            assert_eq!(
                client_ip(&headers, peer, &TrustedProxies::default()),
                "127.0.0.1",
                "{junk} must not be stored"
            );
        }
        assert_eq!(
            client_ip(&HeaderMap::new(), peer, &TrustedProxies::default()),
            "127.0.0.1",
            "with no header the peer is the client"
        );
        assert_eq!(
            client_ip(&HeaderMap::new(), None, &TrustedProxies::default()),
            "unknown"
        );
    }

    /// The list is read once at startup, so a typo in it is dropped rather than
    /// taking the whole list down with it.
    #[test]
    fn only_real_addresses_become_trusted_proxies() {
        let trusted =
            TrustedProxies::from_list(Some("198.51.100.4,, not-an-ip, 10.0.0.0/33, fd00::/8"));
        for peer in ["198.51.100.4", "fd00::1"] {
            assert!(
                trusted.trusts(peer.parse().unwrap()),
                "{peer} should have been kept"
            );
        }
        for peer in ["198.51.100.5", "10.1.2.3"] {
            assert!(
                !trusted.trusts(peer.parse().unwrap()),
                "{peer} should not have been kept"
            );
        }
        assert!(
            !TrustedProxies::default().trusts("198.51.100.4".parse().unwrap()),
            "an unset list trusts nothing but loopback"
        );
    }

    #[test]
    fn anonymous_ids_require_prefix_and_32_lowercase_hex_digits() {
        assert!(valid_anon_id("anon_0123456789abcdef0123456789abcdef"));
        assert!(!valid_anon_id("0123456789abcdef0123456789abcdef"));
        assert!(!valid_anon_id("anon_0123456789ABCDEF0123456789abcdef"));
        assert!(!valid_anon_id("anon_0123"));
    }
}

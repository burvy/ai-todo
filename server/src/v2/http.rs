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
use todo_core::tuning::DEFAULT_WINDOW_DAYS;
use todo_core::{ApiError, DaySettings, ErrorCode, Snapshot, Task, TaskDraft};
use uuid::Uuid;

use super::LockExt;
use super::admin_web;
use super::app::{self, App, ImportResult, Patch};
use super::auth::{self, TokenVerifier};
use super::ics_export::build_ics;
use super::ics_import;
use super::store::{Store, StoreError};
use super::tuning::{
    CAPTURE_MAX_CHARS, CAPTURE_MAX_CLAUSES, DEFAULT_ANON_IDS_PER_IP, DEFAULT_ANON_JEV_DAILY_CAP,
    DEFAULT_JEV_DAILY_CAP, DEFAULT_PAID_JEV_DAILY_CAP, DEFAULT_SIGNINS_PER_IP,
    IMPORT_MAX_BODY_BYTES, IMPORT_MAX_TASKS, MAX_WINDOW_DAYS,
};
use capture::{Revision, capture_text};
use chrono::{Datelike, NaiveDate, Utc};
use judge::AnyJudge;
use todo_core::{LocalContext, Source, TaskId};

mod account;
mod admin;
mod feed;
mod ramble;
mod tasks;

use self::{account::*, admin::*, feed::*, ramble::*, tasks::*};

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
    /// Whether `POST /api/v1/auth/local` exists; see [`local_login_from_env`].
    local_login: bool,
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
    /// Sign-in attempts one client address may make per day. Each one costs a request
    /// to Google, and nothing else limits a caller who has no account yet.
    pub signins_per_ip: u32,
}

impl Caps {
    /// The defaults, each overridable by its `RAMBL_...` variable.
    fn from_env() -> Self {
        Self {
            anon: env_number("RAMBL_JEV_DAILY_CAP_ANON", DEFAULT_ANON_JEV_DAILY_CAP),
            free: env_number("RAMBL_JEV_DAILY_CAP", DEFAULT_JEV_DAILY_CAP),
            paid: env_number("RAMBL_JEV_DAILY_CAP_PAID", DEFAULT_PAID_JEV_DAILY_CAP),
            anon_ids_per_ip: env_number("RAMBL_ANON_IDS_PER_IP", DEFAULT_ANON_IDS_PER_IP),
            signins_per_ip: env_number("RAMBL_SIGNINS_PER_IP", DEFAULT_SIGNINS_PER_IP),
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
            let app = shared.app.lock_recover();
            let user = signed_in(headers, &app)?;
            return Self::unpaused(app.store(), user.id);
        }
        let anonymous = headers
            .get(&ANON_HEADER)
            .and_then(|value| value.to_str().ok())
            .filter(|value| valid_anon_id(value));
        let app = shared.app.lock_recover();
        match anonymous {
            Some(id) if app.store().anonymous_exists(id).map_err(store_failed)? => {
                // A visit keeps the id alive; see `Store::touch_anonymous`.
                app.store()
                    .touch_anonymous(id, app.now())
                    .map_err(store_failed)?;
                Self::unpaused(app.store(), id.to_owned())
            }
            _ => Err(unauthorized("send Authorization: Bearer <token>")),
        }
    }

    /// A paused account stops using the app: every user-facing call is refused, not just
    /// the ones that spend Jev. The admin console and the admin page are separate
    /// (`admin_of`), so the operator is never locked out by this.
    fn unpaused(store: &dyn Store, user: String) -> Result<String, HttpError> {
        if store.is_user_blocked(&user).map_err(store_failed)? {
            return Err(paused());
        }
        Ok(user)
    }
}

/// A paused account, told what to do about it.
fn paused() -> HttpError {
    HttpError(ApiError {
        code: ErrorCode::Forbidden,
        message: "this account is paused".into(),
    })
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
            // A paused account: the request was well-formed and the session is
            // real, so neither 400 nor 401 would be true.
            ErrorCode::Forbidden => StatusCode::FORBIDDEN,
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

/// The Google client id from `GOOGLE_CLIENT_ID`; `None`, local mode, when it is unset.
///
/// Set but blank is an `Err` for `main` to stop on. Local mode gives every visitor one
/// shared task list, so a public server whose id was blanked must not slide into it.
pub fn google_client_id_from_env() -> Result<Option<String>, String> {
    client_id_setting(std::env::var("GOOGLE_CLIENT_ID").ok().as_deref())
}

fn client_id_setting(raw: Option<&str>) -> Result<Option<String>, String> {
    match raw.map(str::trim) {
        None => Ok(None),
        Some("") => Err(
            "GOOGLE_CLIENT_ID is set but empty: unset it for local mode or give a client id".into(),
        ),
        Some(id) => Ok(Some(id.to_owned())),
    }
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
    let client_id = google_client_id_from_env();
    // `main` has already refused a blank id; failing closed here means a blank id still
    // keeps visitors apart, with a verifier that refuses everyone.
    let local = matches!(client_id, Ok(None));
    let verifier: Box<dyn TokenVerifier> = match client_id {
        Ok(Some(id)) => Box::new(auth::GoogleVerifier::new(id)),
        _ => Box::new(auth::FakeVerifier {
            answer: Err("sign-in is disabled".into()),
        }),
    };
    // `main` has already refused an unsafe setting; failing closed here keeps any other
    // caller of `router` safe too.
    let local_login = !local && local_login_from_env().unwrap_or(false);
    build_router_with(
        app,
        judge,
        verifier,
        local,
        Caps::from_env(),
        admin_emails_from_env(),
        TrustedProxies::from_env(),
        local_login,
    )
}

/// Whether `RAMBL_LOCAL_LOGIN=1` turns on the test-site "Continue locally" sign-in.
///
/// It is refused (an `Err` for `main` to stop on) while `RAMBL_TRUSTED_PROXIES` is set,
/// because a deployment that names a proxy is one that sits behind a tunnel on the
/// internet, which is exactly where this bypass must never exist.
pub fn local_login_from_env() -> Result<bool, String> {
    local_login_setting(
        std::env::var("RAMBL_LOCAL_LOGIN").ok().as_deref(),
        std::env::var("RAMBL_TRUSTED_PROXIES").ok().as_deref(),
    )
}

fn local_login_setting(login: Option<&str>, proxies: Option<&str>) -> Result<bool, String> {
    if login.map(str::trim) != Some("1") {
        return Ok(false);
    }
    if proxies.is_some_and(|list| !list.trim().is_empty()) {
        return Err("RAMBL_LOCAL_LOGIN=1 is refused while RAMBL_TRUSTED_PROXIES is set:              the local sign-in bypass is for a test site that is not on the internet"
            .into());
    }
    Ok(true)
}

/// The loud startup warning for an enabled local login.
pub const LOCAL_LOGIN_WARNING: &str = "WARNING: RAMBL_LOCAL_LOGIN=1: anyone on this machine or     LAN can sign in as the local user without Google. Test sites only; never in production.";

/// May this request use the local sign-in? Only a peer on this machine or the private
/// LAN, and only if no proxy header says the request came from further away.
fn local_login_allowed(headers: &HeaderMap, peer: Option<IpAddr>) -> bool {
    let near = peer.is_some_and(|ip| match ip.to_canonical() {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
    });
    near && !headers.contains_key("cf-connecting-ip") && !headers.contains_key("x-forwarded-for")
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
    build_router_with(
        app,
        judge,
        verifier,
        local,
        caps,
        admin_emails,
        trusted_proxies,
        false,
    )
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            anon: DEFAULT_ANON_JEV_DAILY_CAP,
            free: DEFAULT_JEV_DAILY_CAP,
            paid: DEFAULT_PAID_JEV_DAILY_CAP,
            anon_ids_per_ip: DEFAULT_ANON_IDS_PER_IP,
            signins_per_ip: DEFAULT_SIGNINS_PER_IP,
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

#[allow(clippy::too_many_arguments)]
fn build_router_with(
    app: App<Box<dyn Store>>,
    judge: AnyJudge,
    verifier: Box<dyn TokenVerifier>,
    local: bool,
    caps: Caps,
    admin_emails: Option<Vec<String>>,
    trusted_proxies: TrustedProxies,
    local_login: bool,
) -> Router {
    let shared = Arc::new(Shared {
        app: Mutex::new(app),
        judge,
        verifier,
        local,
        caps,
        google_client_id: google_client_id_from_env().ok().flatten(),
        admin_emails,
        trusted_proxies,
        local_login,
    });
    Router::new()
        .route("/api/v1/config", get(config))
        .route("/api/v1/auth/google", post(sign_in_google))
        .route("/api/v1/auth/local", post(sign_in_local))
        .route("/api/v1/auth/signout", post(sign_out))
        .route("/api/v1/me", get(me).delete(delete_me))
        .route("/api/v1/snapshot", get(snapshot))
        .route("/api/v1/tasks", post(create))
        .route("/api/v1/tasks/order", put(reorder))
        .route("/api/v1/tasks/restore", post(restore))
        .route("/api/v1/settings", get(get_settings).patch(patch_settings))
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
        .route("/api/v1/export", get(export))
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
        || request.uri().path() == "/api/v1/auth/local"
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
    let app = shared.app.lock_recover();
    app.store().anonymous_exists(id).unwrap_or(false)
}

/// A new anonymous id, unless this client address has used up its allowance today.
fn issue_anonymous_id(shared: &Shared, ip: &str) -> Result<String, HttpError> {
    let app = shared.app.lock_recover();
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

fn bad_request(message: impl std::fmt::Display) -> HttpError {
    HttpError(ApiError {
        code: ErrorCode::BadRequest,
        message: message.to_string(),
    })
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
mod tests;

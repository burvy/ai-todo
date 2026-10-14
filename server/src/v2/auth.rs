//! Signing in, sessions, and working out who sent a request.
//!
//! The pattern follows `plgroup-server/src/auth.rs` from the PL group site: Google
//! checks the ID token's signature and expiry, and we check the token was made for us
//! (the audience check) before trusting anything else in it. A sign-in is a row in
//! `users` plus a row in `sessions`; every later request carries the session token.
//!
//! When `GOOGLE_CLIENT_ID` is not set the server is in local mode: there is no real
//! client id to check an audience against, so every request runs as one fixed local
//! user and no sign-in is needed. That keeps `run-local.sh` and the tests working
//! without a Google project.

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};

use super::store::{Store, StoreError};
use super::tuning::{GOOGLE_TIMEOUT, SESSION_LENGTH};
use todo_core::ErrorCode;

/// The user every request runs as when `GOOGLE_CLIENT_ID` is unset.
///
/// A fixed id, not a generated one, so a restart keeps finding the same local user's
/// tasks instead of appearing to lose them.
pub const LOCAL_USER: &str = "local";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Plan {
    Anon,
    Free,
    Paid,
}

impl Plan {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anon => "anon",
            Self::Free => "free",
            Self::Paid => "paid",
        }
    }
}

/// What Google's `tokeninfo` endpoint reports about an ID token.
///
/// Every field is a string, including the booleans and the audience: that is what the
/// endpoint returns, so this mirrors it rather than being strict.
#[derive(Clone, Debug, Deserialize)]
pub struct TokenInfo {
    /// "audience" — only accept IDs minted for this client.
    pub aud: String,
    /// "subject" — permanently unique Google id for the person.
    pub sub: String,
    pub email: String,
    /// "true"/"false" as a string, so it must be compared to `"true"`.
    pub email_verified: String,
    pub name: Option<String>,
    pub picture: Option<String>,
}

/// The person behind a verified ID token.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Me {
    pub id: String,
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
    pub plan: Plan,
}

impl Me {
    fn from_token(info: &TokenInfo) -> Self {
        let email = info.email.to_lowercase();
        Self {
            id: info.sub.clone(),
            email: email.clone(),
            name: info.name.clone().unwrap_or_else(|| email.clone()),
            picture: info.picture.clone(),
            plan: Plan::Free,
        }
    }
}

/// The answer to a sign-in: a token to send back, and who it belongs to.
#[derive(Clone, Debug, Serialize)]
pub struct Session {
    pub token: String,
    pub user: Me,
}

/// Why a credential was not accepted.
#[derive(Debug)]
pub enum AuthError {
    /// Google said no, or the token was not minted for us.
    Rejected(String),
    /// We could not ask Google. This is our failure to reach them, not the caller's.
    Unavailable(anyhow::Error),
    /// Our own database failed while signing in or checking a session.
    Store(StoreError),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Rejected(why) => write!(f, "invalid Google sign-in: {why}"),
            AuthError::Unavailable(_) => write!(f, "could not reach Google"),
            AuthError::Store(error) => write!(f, "server error: {error}"),
        }
    }
}

/// Checks an ID token. Behind a trait so tests never touch the network.
pub trait TokenVerifier: Send + Sync {
    fn verify<'a>(
        &'a self,
        credential: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<TokenInfo, AuthError>> + Send + 'a>,
    >;
}

/// The real one: asks Google's `tokeninfo` and checks the audience is ours.
pub struct GoogleVerifier {
    http: reqwest::Client,
    client_id: String,
}

impl GoogleVerifier {
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            // A Google that never answers must not hold a sign-in open for ever.
            http: reqwest::Client::builder()
                .timeout(GOOGLE_TIMEOUT)
                .build()
                .unwrap_or_default(),
            client_id: client_id.into(),
        }
    }
}

impl TokenVerifier for GoogleVerifier {
    fn verify<'a>(
        &'a self,
        credential: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<TokenInfo, AuthError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let response = self
                .http
                .get("https://oauth2.googleapis.com/tokeninfo")
                .query(&[("id_token", credential)])
                .send()
                .await
                .map_err(|why| AuthError::Unavailable(anyhow::anyhow!("{why}")))?;
            if !response.status().is_success() {
                return Err(AuthError::Rejected(format!(
                    "tokeninfo returned {}",
                    response.status()
                )));
            }
            let info: TokenInfo = response
                .json()
                .await
                .map_err(|why| AuthError::Rejected(format!("unreadable tokeninfo: {why}")))?;
            accept(info, &self.client_id)
        })
    }
}

/// The two checks that matter once Google has vouched for a token. An audience that
/// is not ours means the token is real but for someone else's app: reject rather
/// than trust it.
fn accept(info: TokenInfo, client_id: &str) -> Result<TokenInfo, AuthError> {
    if info.aud != client_id {
        return Err(AuthError::Rejected(
            "token was not made for this app".into(),
        ));
    }
    if info.email_verified != "true" {
        return Err(AuthError::Rejected("email is not verified".into()));
    }
    Ok(info)
}

/// Verifies with a fixed answer. Tests use this, and local mode wires one in that
/// refuses every credential, since nobody signs in there.
pub struct FakeVerifier {
    pub answer: Result<TokenInfo, String>,
}

impl TokenVerifier for FakeVerifier {
    fn verify<'a>(
        &'a self,
        _credential: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<TokenInfo, AuthError>> + Send + 'a>,
    > {
        let answer = self.answer.clone();
        Box::pin(async move { answer.map_err(AuthError::Rejected) })
    }
}

impl TokenInfo {
    /// The fixed identity behind `RAMBL_LOCAL_LOGIN`. Its own id, so it never collides
    /// with a Google `sub` or with [`LOCAL_USER`].
    pub fn local_login() -> Self {
        Self {
            aud: "local-login".into(),
            sub: "local-login".into(),
            email: "local-login@localhost".into(),
            email_verified: "true".into(),
            name: Some("Local user".into()),
            picture: None,
        }
    }

    /// A tokeninfo answer a test can hand to [`FakeVerifier`].
    #[cfg(test)]
    pub fn for_test(sub: &str, email: &str) -> Self {
        Self {
            aud: "test-client".into(),
            sub: sub.into(),
            email: email.into(),
            email_verified: "true".into(),
            name: Some(format!("Test {sub}")),
            picture: None,
        }
    }
}

/// A random session token.
pub fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Create or refresh the user and open a session. The store half of a sign-in.
///
/// Kept separate from verification so a caller can finish the network call *before*
/// taking the store lock: holding a `MutexGuard` across an `await` makes the handler's
/// future non-`Send`, and axum rejects a non-`Send` handler.
pub fn open_session<S: Store + ?Sized>(
    store: &S,
    info: &TokenInfo,
    now: DateTime<Utc>,
    anonymous: Option<&str>,
) -> Result<Session, AuthError> {
    let user = Me::from_token(info);
    let token = new_token();
    let expires_at = now + ChronoDuration::from_std(SESSION_LENGTH).unwrap_or_default();
    store
        .upsert_user_and_session(&user, &token, now, expires_at)
        .map_err(store_auth_error)?;
    if let Some(anonymous) = anonymous {
        store
            .adopt_anonymous(anonymous, &user.id)
            .map_err(store_auth_error)?;
    }
    Ok(Session { token, user })
}

/// End one session. Signing out is not signing in everywhere else.
pub fn sign_out<S: Store + ?Sized>(store: &S, token: &str) -> Result<(), AuthError> {
    store.delete_session(token).map_err(store_auth_error)
}

/// The user a session token belongs to, if the session has not expired.
pub fn session_user<S: Store + ?Sized>(
    store: &S,
    token: &str,
    now: DateTime<Utc>,
) -> Result<Option<Me>, AuthError> {
    store.session_user(token, now).map_err(store_auth_error)
}

/// Forget a person and everything they own.
pub fn delete_user<S: Store + ?Sized>(store: &S, user: &str) -> Result<(), AuthError> {
    store.delete_user(user).map_err(store_auth_error)
}

fn store_auth_error(error: StoreError) -> AuthError {
    AuthError::Store(error)
}

/// An [`AuthError`] as the error a client sees.
impl From<AuthError> for todo_core::ApiError {
    fn from(error: AuthError) -> Self {
        let code = match error {
            // We could not reach Google: that is a dependency being down, so the
            // caller should retry. Not the caller's credential.
            AuthError::Unavailable(_) => ErrorCode::Unavailable,
            // Our own fault: neither Google nor the caller's credential.
            AuthError::Store(_) => ErrorCode::Internal,
            AuthError::Rejected(_) => ErrorCode::BadRequest,
        };
        todo_core::ApiError {
            code,
            message: error.to_string(),
        }
    }
}

/// The bearer token from an `Authorization` header, if there is one.
pub fn bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejection(result: Result<TokenInfo, AuthError>) -> String {
        match result {
            Err(AuthError::Rejected(why)) => why,
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_verified_token_for_this_app_is_accepted() {
        let info = TokenInfo::for_test("google-1", "sam@example.com");
        let accepted = accept(info, "test-client").expect("accepted");
        assert_eq!(accepted.sub, "google-1");
    }

    #[test]
    fn a_token_made_for_another_app_is_rejected() {
        let info = TokenInfo::for_test("google-1", "sam@example.com");
        assert!(rejection(accept(info, "our-client")).contains("not made for this app"));
    }

    #[test]
    fn an_unverified_email_is_rejected() {
        for verified in ["false", "", "TRUE"] {
            let info = TokenInfo {
                email_verified: verified.into(),
                ..TokenInfo::for_test("google-1", "sam@example.com")
            };
            assert!(
                rejection(accept(info, "test-client")).contains("not verified"),
                "email_verified={verified:?}"
            );
        }
    }

    #[test]
    fn a_rejection_is_a_bad_request_and_an_outage_is_unavailable() {
        let rejected: todo_core::ApiError = AuthError::Rejected("no".into()).into();
        assert_eq!(rejected.code, ErrorCode::BadRequest);
        let outage: todo_core::ApiError = AuthError::Unavailable(anyhow::anyhow!("down")).into();
        assert_eq!(outage.code, ErrorCode::Unavailable);
    }

    /// A database failure while checking a session is our fault: a 500, and never
    /// blamed on Google.
    #[test]
    fn a_store_failure_is_a_server_error_not_google() {
        let failed: todo_core::ApiError = store_auth_error(StoreError::Backend(
            super::super::store::Failure::plain("session_user", "disk I/O error".into()),
        ))
        .into();
        assert_eq!(failed.code, ErrorCode::Internal);
        assert!(
            failed.message.starts_with("server error"),
            "{}",
            failed.message
        );
        assert!(!failed.message.contains("Google"), "{}", failed.message);
    }

    #[test]
    fn a_bearer_token_is_read_trimmed_and_never_empty() {
        let header = |value: &str| {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(axum::http::header::AUTHORIZATION, value.parse().unwrap());
            bearer(&headers)
        };
        assert_eq!(header("Bearer abc ").as_deref(), Some("abc"));
        assert_eq!(header("Bearer   "), None);
        assert_eq!(header("Basic abc"), None);
        assert_eq!(bearer(&axum::http::HeaderMap::new()), None);
    }
}

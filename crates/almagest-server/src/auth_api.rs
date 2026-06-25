// SPDX-License-Identifier: MIT OR Apache-2.0

//! User-facing auth endpoints (doc 13): the first-admin setup flow, login,
//! logout, the `me` bootstrap probe, and self-service password change.
//!
//! The login UI itself is a view in the Svelte SPA (one frontend for every mode)
//! rather than a separate binary-embedded HTML page — these endpoints back it.

use crate::auth::{
    self, CurrentUser, hash_password, issue_session, validate_password, verify_password,
};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, ServerEvent};
use almagest_core::{Role, User};
use axum::Json;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use serde::{Deserialize, Serialize};

/// `GET /api/almagest/auth/me` — bootstrap probe the SPA calls on load: is auth
/// enabled, does the file still need its first admin, and who (if anyone) is the
/// current session's user.
#[derive(Serialize)]
pub struct AuthMe {
    /// Whether auth is enforced for this file.
    pub auth_enabled: bool,
    /// True when auth is enabled but no users exist yet (show the setup form).
    pub needs_setup: bool,
    /// The currently authenticated user, if the request carried a valid session.
    pub user: Option<User>,
}

/// `GET /api/almagest/auth/me`.
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<AuthMe>> {
    let enabled = state.auth.enabled();
    let (needs_setup, user) = {
        let file = state.file();
        let needs_setup = enabled && file.count_users()? == 0;
        // Resolve the session inline (the middleware doesn't inject a user on the
        // public `me` route when unauthenticated).
        let user = resolve_current_user(&state, &file, &headers);
        (needs_setup, user)
    };
    Ok(Json(AuthMe {
        auth_enabled: enabled,
        needs_setup,
        user,
    }))
}

/// Resolve the request's session into a [`User`], if any. Mirrors the
/// middleware's logic but returns the full user record for the `me` response.
fn resolve_current_user(
    state: &AppState,
    file: &almagest_core::AlmagestFile,
    headers: &HeaderMap,
) -> Option<User> {
    let secret = state.auth.secret()?;
    let token = auth::cookie_value(headers, auth::SESSION_COOKIE)?;
    let now = chrono::Utc::now().timestamp();
    let claims = auth::verify_session(&secret, &token, now)?;
    file.user_by_id(&claims.user_id).ok().flatten()
}

/// Body for the first-admin setup and for login.
#[derive(Deserialize)]
pub struct Credentials {
    /// Login name.
    pub username: String,
    /// Plaintext password (never stored or logged).
    pub password: String,
    /// Optional email (informational).
    #[serde(default)]
    pub email: Option<String>,
}

/// What login / setup return: the user plus the CSRF token the SPA echoes back
/// in the `X-CSRF-Token` header on subsequent mutating requests.
#[derive(Serialize)]
pub struct AuthSession {
    /// The signed-in user.
    pub user: User,
    /// The double-submit CSRF token (also set as a JS-readable cookie).
    pub csrf_token: String,
}

/// `POST /api/almagest/auth/setup` — create the first admin and enable auth.
///
/// Only valid while the file has no users (the "set this file up for team
/// sharing" flow). It generates the file's session-signing secret, enables auth,
/// creates the admin, and logs them in. Idempotency guard: once any user exists
/// this returns 409.
pub async fn setup(
    State(state): State<AppState>,
    Json(body): Json<Credentials>,
) -> ApiResult<(HeaderMap, Json<AuthSession>)> {
    state.ensure_writable()?;
    validate_password(&body.password)?;
    let hash = hash_password(&body.password)?;

    let (user, secret, lifetime) = {
        let file = state.file();
        if file.count_users()? > 0 {
            return Err(ApiError::conflict(
                "this file already has user accounts; use login",
            ));
        }
        let secret = auth::generate_secret();
        file.set_session_secret(&secret)?;
        file.set_auth_enabled(true)?;
        let user = file.create_user(
            body.username.trim(),
            &hash,
            Role::Admin,
            body.email.as_deref(),
        )?;
        file.record_login(&user.id)?;
        file.append_history("auth_enabled", None, Some(&user.id), None)?;
        file.append_history("user_created", Some(&user.id), Some(&user.id), None)?;
        file.append_history("login", None, Some(&user.id), None)?;
        let lifetime = file.auth_config()?.session_lifetime_secs;
        (user, secret, lifetime)
    };
    // Refresh the cached auth runtime now that the file enabled auth.
    state.reload_auth()?;
    state.emit(ServerEvent::AuthChanged);

    let (headers, csrf_token) = session_headers(&secret, &user, lifetime)?;
    Ok((headers, Json(AuthSession { user, csrf_token })))
}

/// `POST /api/almagest/auth/login` — verify credentials and start a session.
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<Credentials>,
) -> ApiResult<(HeaderMap, Json<AuthSession>)> {
    if !state.auth.enabled() {
        return Err(ApiError::bad_request("auth is not enabled for this file"));
    }
    let now = chrono::Utc::now().timestamp();
    let username = body.username.trim().to_string();

    // Throttle check before touching the password.
    match state.auth.throttle.check(&username, now) {
        auth::ThrottleVerdict::Allow => {}
        auth::ThrottleVerdict::RateLimited => {
            return Err(ApiError::too_many_requests(
                "too many login attempts; wait a few minutes and try again",
            ));
        }
        auth::ThrottleVerdict::Locked => {
            return Err(ApiError::too_many_requests(
                "account locked after repeated failures; ask an admin to unlock it",
            ));
        }
    }

    let secret = state
        .auth
        .secret()
        .ok_or_else(|| ApiError::internal("auth enabled but no session secret"))?;
    let lifetime = state.auth.lifetime_secs();

    let creds = {
        let file = state.file();
        file.user_credentials(&username)?
    };
    let (user, hash) = match creds {
        Some(v) => v,
        None => {
            // Run a real verify against a throwaway hash to keep login timing
            // roughly uniform whether or not the username exists.
            let _ = verify_password(&body.password, dummy_hash());
            state.auth.throttle.record_failure(&username, now);
            record_failed_login(&state, &username);
            return Err(ApiError::unauthorized("invalid username or password"));
        }
    };

    if !verify_password(&body.password, &hash) {
        state.auth.throttle.record_failure(&username, now);
        record_failed_login(&state, &username);
        return Err(ApiError::unauthorized("invalid username or password"));
    }

    state.auth.throttle.record_success(&username);
    {
        let file = state.file();
        file.record_login(&user.id)?;
        file.append_history("login", None, Some(&user.id), None)?;
    }

    let (headers, csrf_token) = session_headers(&secret, &user, lifetime)?;
    Ok((headers, Json(AuthSession { user, csrf_token })))
}

/// `POST /api/almagest/auth/logout` — clear the session and CSRF cookies.
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<HeaderMap> {
    // Best-effort audit attribution if the caller had a session.
    if let Some(secret) = state.auth.secret()
        && let Some(token) = auth::cookie_value(&headers, auth::SESSION_COOKIE)
        && let Some(claims) = auth::verify_session(&secret, &token, chrono::Utc::now().timestamp())
    {
        let _ = state
            .file()
            .append_history("logout", None, Some(&claims.user_id), None);
    }
    let mut out = HeaderMap::new();
    for c in auth::clear_cookies() {
        out.append(header::SET_COOKIE, header_value(&c)?);
    }
    Ok(out)
}

/// Body for self-service password change.
#[derive(Deserialize)]
pub struct ChangePassword {
    /// The caller's current password (re-verified).
    pub current_password: String,
    /// The desired new password.
    pub new_password: String,
}

/// `POST /api/almagest/auth/change-password` — change the caller's own password.
pub async fn change_password(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<ChangePassword>,
) -> ApiResult<StatusCode> {
    validate_password(&body.new_password)?;
    let new_hash = hash_password(&body.new_password)?;
    {
        let file = state.file();
        let (_, hash) = file
            .user_credentials(&user.username)?
            .ok_or_else(|| ApiError::unauthorized("user no longer exists"))?;
        if !verify_password(&body.current_password, &hash) {
            return Err(ApiError::unauthorized("current password is incorrect"));
        }
        file.set_password_hash(&user.id, &new_hash)?;
        file.append_history("password_changed", Some(&user.id), Some(&user.id), None)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

// --- helpers -----------------------------------------------------------------

/// A real Argon2 hash (of a fixed throwaway string) verified against on the
/// unknown-user login path so timing doesn't trivially reveal whether a username
/// exists. Computed once and cached.
fn dummy_hash() -> &'static str {
    use std::sync::OnceLock;
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        hash_password("almagest-timing-uniformity-placeholder")
            .unwrap_or_else(|_| "$argon2id$invalid".to_string())
    })
}

/// Build the session + CSRF `Set-Cookie` headers for a freshly authenticated
/// user, returning the headers and the CSRF token (echoed in the JSON body).
fn session_headers(secret: &[u8], user: &User, lifetime: i64) -> ApiResult<(HeaderMap, String)> {
    let now = chrono::Utc::now().timestamp();
    let token = issue_session(secret, &user.id, user.role, now, lifetime);
    let csrf = auth::new_csrf_token();
    let mut headers = HeaderMap::new();
    headers.append(
        header::SET_COOKIE,
        header_value(&auth::session_cookie(&token, lifetime))?,
    );
    headers.append(
        header::SET_COOKIE,
        header_value(&auth::csrf_cookie(&csrf, lifetime))?,
    );
    Ok((headers, csrf))
}

fn header_value(s: &str) -> ApiResult<HeaderValue> {
    HeaderValue::from_str(s).map_err(|e| ApiError::internal(format!("bad header value: {e}")))
}

fn record_failed_login(state: &AppState, username: &str) {
    let _ = state.file().append_history(
        "login_failed",
        None,
        None,
        Some(&format!("{{\"username\":{}}}", json_str(username))),
    );
}

/// Minimal JSON string escaping for the audit payload.
fn json_str(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

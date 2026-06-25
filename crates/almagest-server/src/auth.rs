// SPDX-License-Identifier: MIT OR Apache-2.0

//! Server-side auth primitives (doc 13): Argon2id password hashing, HMAC-signed
//! stateless session cookies, the current-user identity, a double-submit CSRF
//! token, and an in-memory login throttle.
//!
//! Crypto lives here (not in `almagest-core`) so the format crate stays free of
//! crypto dependencies — core only stores opaque password-hash strings and the
//! file-scoped signing secret.
//!
//! ## Sessions
//!
//! Sessions are **stateless**: the cookie carries `user_id|role|expiry`, signed
//! with an HMAC secret stored in the `.alm`. No server-side session table, so
//! sessions survive restarts and there's nothing to garbage-collect — at the
//! cost that an individual session can't be revoked before it expires (changing
//! the file secret invalidates *all* sessions; that's the available big hammer).
//! Expiry is absolute (24h default); sliding idle-refresh is a later refinement.

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use almagest_core::Role;
use argon2::Argon2;
use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Mutex;

type HmacSha256 = Hmac<Sha256>;

/// Name of the HttpOnly session cookie.
pub const SESSION_COOKIE: &str = "alm_session";
/// Name of the (JS-readable) double-submit CSRF cookie.
pub const CSRF_COOKIE: &str = "alm_csrf";
/// Header the client echoes the CSRF token back in.
pub const CSRF_HEADER: &str = "x-csrf-token";

/// Minimum password length. Kept simple (no breached-password check in v1).
pub const MIN_PASSWORD_LEN: usize = 8;

// --- password hashing --------------------------------------------------------

/// Hash a password with Argon2id (default params) and a fresh random salt.
pub fn hash_password(password: &str) -> ApiResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| ApiError::internal(format!("password hash failed: {e}")))?;
    Ok(hash.to_string())
}

/// Verify a password against a stored Argon2 hash. A malformed stored hash
/// verifies as `false` (never panics).
pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    match PasswordHash::new(stored_hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Enforce the minimum password policy. Returns a client-facing message on fail.
pub fn validate_password(password: &str) -> ApiResult<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(ApiError::bad_request(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

// --- random helpers ----------------------------------------------------------

/// A fresh 32-byte session-signing secret.
pub fn generate_secret() -> Vec<u8> {
    let mut buf = vec![0u8; 32];
    OsRng.fill_bytes(&mut buf);
    buf
}

/// A short random token (URL-safe base64), for CSRF tokens.
fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    B64.encode(buf)
}

/// A human-typable temporary password (admin password reset). Alphanumeric so it
/// survives copy/paste and shell quoting.
pub fn generate_temp_password() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789";
    let mut raw = [0u8; 16];
    OsRng.fill_bytes(&mut raw);
    raw.iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect()
}

// --- session tokens ----------------------------------------------------------

/// The validated contents of a session cookie. The live role is re-read from the
/// file on each request (so it can't go stale), so `role`/`expires_at` here are
/// the token's own claims — used in tests and available to callers, not the
/// authorization source of truth.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SessionClaims {
    /// The authenticated user's id.
    pub user_id: String,
    /// The user's role at issue time.
    pub role: Role,
    /// Absolute expiry (unix seconds).
    pub expires_at: i64,
}

/// Sign a session token for `user` valid for `lifetime_secs` from `now`.
///
/// Format: `base64url(user_id|role|exp) . base64url(hmac)` — the MAC is over the
/// base64 payload so a tampered field fails verification.
pub fn issue_session(
    secret: &[u8],
    user_id: &str,
    role: Role,
    now: i64,
    lifetime_secs: i64,
) -> String {
    let exp = now + lifetime_secs;
    let payload = format!("{user_id}|{}|{exp}", role.as_str());
    let payload_b64 = B64.encode(payload.as_bytes());
    let tag = sign(secret, payload_b64.as_bytes());
    format!("{payload_b64}.{tag}")
}

/// Verify a session token's signature and expiry against `now`. Returns the
/// claims only if the MAC checks out and the token hasn't expired.
pub fn verify_session(secret: &[u8], token: &str, now: i64) -> Option<SessionClaims> {
    let (payload_b64, tag_b64) = token.split_once('.')?;
    // Constant-time MAC check first.
    let tag = B64.decode(tag_b64).ok()?;
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(payload_b64.as_bytes());
    mac.verify_slice(&tag).ok()?;

    let payload = String::from_utf8(B64.decode(payload_b64).ok()?).ok()?;
    let mut parts = payload.split('|');
    let user_id = parts.next()?.to_string();
    let role = parts.next()?.parse::<Role>().ok()?;
    let exp: i64 = parts.next()?.parse().ok()?;
    if now >= exp {
        return None;
    }
    Some(SessionClaims {
        user_id,
        role,
        expires_at: exp,
    })
}

fn sign(secret: &[u8], payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(payload);
    B64.encode(mac.finalize().into_bytes())
}

// --- cookies -----------------------------------------------------------------

/// A fresh CSRF token value.
pub fn new_csrf_token() -> String {
    random_token(18)
}

/// Build the `Set-Cookie` value for the session cookie.
///
/// `Secure` is intentionally **omitted**: Almagest is served over plain HTTP on
/// localhost (desktop) and commonly behind a host's TLS-terminating proxy
/// (headless), where setting `Secure` would drop the cookie. Deployments
/// terminating TLS directly should front Almagest with a proxy that adds it; a
/// `--secure-cookies` flag is a later addition.
pub fn session_cookie(token: &str, max_age: i64) -> String {
    format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}")
}

/// Build the `Set-Cookie` value for the (JS-readable) CSRF cookie.
pub fn csrf_cookie(token: &str, max_age: i64) -> String {
    format!("{CSRF_COOKIE}={token}; Path=/; SameSite=Strict; Max-Age={max_age}")
}

/// Build the two `Set-Cookie` values that clear both cookies (logout).
pub fn clear_cookies() -> [String; 2] {
    [
        format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
        format!("{CSRF_COOKIE}=; Path=/; SameSite=Strict; Max-Age=0"),
    ]
}

/// Read a cookie value from a request's `Cookie` header.
pub fn cookie_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=')
            && k == name
        {
            return Some(v.to_string());
        }
    }
    None
}

// --- current user ------------------------------------------------------------

/// The identity attached to a request by the auth middleware. When auth is
/// disabled this is a synthetic local admin so every handler can uniformly check
/// roles without branching on whether auth is on.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    /// User id (`"local"` for the synthetic no-auth admin).
    pub id: String,
    /// Username (`"local"` for the synthetic no-auth admin).
    pub username: String,
    /// Effective role.
    pub role: Role,
    /// Whether this identity came from a real authenticated session (vs. the
    /// synthetic no-auth admin). Audit logging uses this to attribute actions.
    pub authenticated: bool,
}

impl CurrentUser {
    /// The synthetic admin used when auth is disabled.
    pub fn local_admin() -> Self {
        Self {
            id: "local".to_string(),
            username: "local".to_string(),
            role: Role::Admin,
            authenticated: false,
        }
    }

    /// The user id to attribute audit events to, or `None` for the synthetic
    /// no-auth admin.
    pub fn audit_id(&self) -> Option<&str> {
        self.authenticated.then_some(self.id.as_str())
    }

    /// Require at least `required`, else a 403.
    pub fn require(&self, required: Role) -> ApiResult<()> {
        if self.role.satisfies(required) {
            Ok(())
        } else {
            Err(ApiError::new(
                axum::http::StatusCode::FORBIDDEN,
                "forbidden",
                format!(
                    "this action requires the '{}' role; you are '{}'",
                    required.as_str(),
                    self.role.as_str()
                ),
            ))
        }
    }

    /// Require edit (editor or admin) rights.
    pub fn require_editor(&self) -> ApiResult<()> {
        self.require(Role::Editor)
    }

    /// Require admin rights.
    pub fn require_admin(&self) -> ApiResult<()> {
        self.require(Role::Admin)
    }
}

// --- middleware --------------------------------------------------------------

/// Axum middleware that enforces local-account auth (doc 13).
///
/// - Auth **disabled** → inject a synthetic local admin and pass through, so
///   every handler can check roles uniformly (everyone is admin in single-user
///   mode).
/// - Auth **enabled** → the SPA shell / static assets and the bootstrap auth
///   endpoints stay public (so the login page can load); every other API route
///   requires a valid session cookie (else `401`), and state-changing requests
///   additionally require a matching double-submit CSRF token (else `403`).
pub async fn local_auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    if !state.auth.enabled() {
        req.extensions_mut().insert(CurrentUser::local_admin());
        return next.run(req).await;
    }

    let path = req.uri().path().to_string();
    if !path.starts_with("/api/almagest") {
        // SPA shell + static assets are always served (the login view lives there).
        return next.run(req).await;
    }

    let method = req.method().clone();
    let current = resolve_request_user(&state, req.headers());

    match current {
        Some(user) => {
            if is_state_changing(&method) && !is_csrf_exempt(&path) && !csrf_ok(req.headers()) {
                return ApiError::forbidden("missing or invalid CSRF token").into_response();
            }
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        None if is_public_api(&path) => next.run(req).await,
        None => ApiError::unauthorized("authentication required").into_response(),
    }
}

/// Resolve a request's session cookie into a [`CurrentUser`], re-reading the
/// live role from the file (so role changes / deletions take effect immediately
/// and revoke stale sessions). The file lock is released before returning.
fn resolve_request_user(state: &AppState, headers: &axum::http::HeaderMap) -> Option<CurrentUser> {
    let secret = state.auth.secret()?;
    let token = cookie_value(headers, SESSION_COOKIE)?;
    let now = chrono::Utc::now().timestamp();
    let claims = verify_session(&secret, &token, now)?;
    let user = state.file().user_by_id(&claims.user_id).ok().flatten()?;
    Some(CurrentUser {
        id: user.id,
        username: user.username,
        role: user.role,
        authenticated: true,
    })
}

/// API routes reachable without a session (bootstrap + the login/setup flow).
fn is_public_api(path: &str) -> bool {
    matches!(
        path,
        "/api/almagest"
            | "/api/almagest/"
            | "/api/almagest/auth/me"
            | "/api/almagest/auth/login"
            | "/api/almagest/auth/setup"
            | "/api/almagest/auth/logout"
    )
}

/// Endpoints exempt from CSRF: they establish or clear the session itself, so
/// there is no pre-existing session state for a forged request to abuse.
fn is_csrf_exempt(path: &str) -> bool {
    matches!(
        path,
        "/api/almagest/auth/login" | "/api/almagest/auth/setup" | "/api/almagest/auth/logout"
    )
}

fn is_state_changing(m: &axum::http::Method) -> bool {
    use axum::http::Method;
    matches!(
        *m,
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    )
}

/// Double-submit CSRF check: the `X-CSRF-Token` header must match the JS-readable
/// `alm_csrf` cookie.
fn csrf_ok(headers: &axum::http::HeaderMap) -> bool {
    let cookie = cookie_value(headers, CSRF_COOKIE);
    let header = headers
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    matches!((cookie, header), (Some(c), Some(h)) if !c.is_empty() && c == h)
}

// --- login throttle ----------------------------------------------------------

/// Attempts allowed in the window before further attempts are rate-limited.
const MAX_ATTEMPTS: u32 = 5;
/// Sliding window (seconds) for counting failed attempts.
const WINDOW_SECS: i64 = 15 * 60;
/// Cumulative failures (in-window) that lock the account until an admin unlocks.
const LOCK_THRESHOLD: u32 = 10;

#[derive(Debug, Clone)]
struct Attempt {
    failures: u32,
    window_start: i64,
    locked: bool,
}

/// Why a login was refused before even checking the password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThrottleVerdict {
    /// Proceed to verify the password.
    Allow,
    /// Too many recent attempts; back off.
    RateLimited,
    /// Account locked after repeated failures; needs an admin unlock.
    Locked,
}

/// In-memory per-username login throttle and lockout. Not persisted: it resets
/// on restart (a deliberate, documented simplification — restart is a coarse
/// reset, and admin unlock clears a single entry).
#[derive(Default)]
pub struct LoginThrottle {
    inner: Mutex<HashMap<String, Attempt>>,
}

impl LoginThrottle {
    /// Decide whether a login attempt for `username` may proceed.
    pub fn check(&self, username: &str, now: i64) -> ThrottleVerdict {
        let map = self.inner.lock().expect("throttle mutex");
        match map.get(&key(username)) {
            Some(a) if a.locked => ThrottleVerdict::Locked,
            Some(a) if a.failures >= MAX_ATTEMPTS && now - a.window_start < WINDOW_SECS => {
                ThrottleVerdict::RateLimited
            }
            _ => ThrottleVerdict::Allow,
        }
    }

    /// Record a failed attempt; may transition the account to locked.
    pub fn record_failure(&self, username: &str, now: i64) {
        let mut map = self.inner.lock().expect("throttle mutex");
        let a = map.entry(key(username)).or_insert(Attempt {
            failures: 0,
            window_start: now,
            locked: false,
        });
        if now - a.window_start >= WINDOW_SECS {
            a.failures = 0;
            a.window_start = now;
        }
        a.failures += 1;
        if a.failures >= LOCK_THRESHOLD {
            a.locked = true;
        }
    }

    /// Clear the throttle/lockout for a user after a successful login.
    pub fn record_success(&self, username: &str) {
        self.inner
            .lock()
            .expect("throttle mutex")
            .remove(&key(username));
    }

    /// Admin action: clear a user's lockout/throttle.
    pub fn unlock(&self, username: &str) {
        self.inner
            .lock()
            .expect("throttle mutex")
            .remove(&key(username));
    }
}

fn key(username: &str) -> String {
    username.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("x", "not-a-hash"));
    }

    #[test]
    fn session_round_trip_and_tamper() {
        let secret = generate_secret();
        let tok = issue_session(&secret, "u1", Role::Editor, 1000, 3600);
        let claims = verify_session(&secret, &tok, 1000).unwrap();
        assert_eq!(claims.user_id, "u1");
        assert_eq!(claims.role, Role::Editor);
        // Expired.
        assert!(verify_session(&secret, &tok, 9999).is_none());
        // Wrong secret.
        assert!(verify_session(&generate_secret(), &tok, 1000).is_none());
        // Tampered payload.
        let mut bad = tok.clone();
        bad.insert(0, 'x');
        assert!(verify_session(&secret, &bad, 1000).is_none());
    }

    #[test]
    fn throttle_locks_after_threshold() {
        let t = LoginThrottle::default();
        assert_eq!(t.check("bob", 0), ThrottleVerdict::Allow);
        for i in 0..MAX_ATTEMPTS {
            t.record_failure("bob", i as i64);
        }
        assert_eq!(t.check("bob", 0), ThrottleVerdict::RateLimited);
        for i in MAX_ATTEMPTS..LOCK_THRESHOLD {
            t.record_failure("bob", i as i64);
        }
        assert_eq!(t.check("bob", 0), ThrottleVerdict::Locked);
        t.unlock("bob");
        assert_eq!(t.check("bob", 0), ThrottleVerdict::Allow);
    }

    #[test]
    fn throttle_window_resets() {
        let t = LoginThrottle::default();
        for _ in 0..MAX_ATTEMPTS {
            t.record_failure("amy", 0);
        }
        assert_eq!(t.check("amy", 0), ThrottleVerdict::RateLimited);
        // After the window, attempts are allowed again.
        assert_eq!(t.check("amy", WINDOW_SECS + 1), ThrottleVerdict::Allow);
    }
}

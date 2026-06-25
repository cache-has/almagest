// SPDX-License-Identifier: MIT OR Apache-2.0

//! Admin-only account management and audit endpoints (doc 13).
//!
//! Every handler here requires the `admin` role (enforced via the injected
//! [`CurrentUser`]). When auth is *disabled* the middleware injects a synthetic
//! local admin, so these endpoints work in single-user mode too — handy for the
//! CLI/management flows that land in later phases.

use crate::auth::{CurrentUser, generate_temp_password, hash_password, validate_password};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, ServerEvent};
use almagest_core::{HistoryEntry, HistoryFilter, Role, User};
use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// `GET /api/almagest/admin/users` — list all accounts (no password hashes).
pub async fn list_users(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
) -> ApiResult<Json<Vec<User>>> {
    user.require_admin()?;
    Ok(Json(state.file().list_users()?))
}

/// Body to create a user.
#[derive(Deserialize)]
pub struct CreateUser {
    /// Login name.
    pub username: String,
    /// Initial password.
    pub password: String,
    /// Role string (`admin` / `editor` / `viewer`).
    pub role: String,
    /// Optional email.
    #[serde(default)]
    pub email: Option<String>,
}

/// `POST /api/almagest/admin/users` — create a new account.
pub async fn create_user(
    State(state): State<AppState>,
    Extension(actor): Extension<CurrentUser>,
    Json(body): Json<CreateUser>,
) -> ApiResult<(StatusCode, Json<User>)> {
    actor.require_admin()?;
    state.ensure_writable()?;
    let role = parse_role(&body.role)?;
    validate_password(&body.password)?;
    let hash = hash_password(&body.password)?;
    let user = {
        let file = state.file();
        let user = file.create_user(body.username.trim(), &hash, role, body.email.as_deref())?;
        file.append_history("user_created", Some(&user.id), actor.audit_id(), None)?;
        user
    };
    Ok((StatusCode::CREATED, Json(user)))
}

/// Body to update a user's role.
#[derive(Deserialize)]
pub struct UpdateUser {
    /// New role string.
    pub role: String,
}

/// `PUT /api/almagest/admin/users/:id` — change a user's role.
pub async fn update_user(
    State(state): State<AppState>,
    Extension(actor): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<UpdateUser>,
) -> ApiResult<StatusCode> {
    actor.require_admin()?;
    state.ensure_writable()?;
    let role = parse_role(&body.role)?;
    {
        let file = state.file();
        let target = file
            .user_by_id(&id)?
            .ok_or_else(|| ApiError::not_found(format!("user '{id}' not found")))?;
        // Don't allow demoting the last remaining admin (would lock everyone out
        // of account management).
        if target.role == Role::Admin
            && role != Role::Admin
            && file.count_users_with_role(Role::Admin)? <= 1
        {
            return Err(ApiError::bad_request(
                "cannot demote the last admin; promote another admin first",
            ));
        }
        if !file.set_user_role(&id, role)? {
            return Err(ApiError::not_found(format!("user '{id}' not found")));
        }
        file.append_history(
            "role_changed",
            Some(&id),
            actor.audit_id(),
            Some(&format!("{{\"role\":\"{}\"}}", role.as_str())),
        )?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/almagest/admin/users/:id` — delete an account.
pub async fn delete_user(
    State(state): State<AppState>,
    Extension(actor): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    actor.require_admin()?;
    state.ensure_writable()?;
    let username = {
        let file = state.file();
        let target = file
            .user_by_id(&id)?
            .ok_or_else(|| ApiError::not_found(format!("user '{id}' not found")))?;
        if target.role == Role::Admin && file.count_users_with_role(Role::Admin)? <= 1 {
            return Err(ApiError::bad_request(
                "cannot delete the last admin; create another admin first",
            ));
        }
        if !file.delete_user(&id)? {
            return Err(ApiError::not_found(format!("user '{id}' not found")));
        }
        file.append_history("user_deleted", Some(&id), actor.audit_id(), None)?;
        target.username
    };
    // Invalidate any active throttle entry for the removed user's name.
    state.auth.throttle.unlock(&username);
    Ok(StatusCode::NO_CONTENT)
}

/// The temporary password returned (once) after an admin reset.
#[derive(Serialize)]
pub struct TempPassword {
    /// The generated temporary password — shown once; the user changes it.
    pub temporary_password: String,
}

/// `POST /api/almagest/admin/users/:id/reset-password` — set a random temp
/// password and return it once.
pub async fn reset_password(
    State(state): State<AppState>,
    Extension(actor): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<TempPassword>> {
    actor.require_admin()?;
    state.ensure_writable()?;
    let temp = generate_temp_password();
    let hash = hash_password(&temp)?;
    {
        let file = state.file();
        if !file.set_password_hash(&id, &hash)? {
            return Err(ApiError::not_found(format!("user '{id}' not found")));
        }
        file.append_history("password_reset", Some(&id), actor.audit_id(), None)?;
    }
    Ok(Json(TempPassword {
        temporary_password: temp,
    }))
}

/// `POST /api/almagest/admin/users/:id/unlock` — clear a user's login lockout.
pub async fn unlock_user(
    State(state): State<AppState>,
    Extension(actor): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    actor.require_admin()?;
    let username = {
        let file = state.file();
        file.user_by_id(&id)?
            .ok_or_else(|| ApiError::not_found(format!("user '{id}' not found")))?
            .username
    };
    state.auth.throttle.unlock(&username);
    Ok(StatusCode::NO_CONTENT)
}

/// Query params for the audit log.
#[derive(Deserialize)]
pub struct AuditQuery {
    /// Restrict to one acting user id.
    #[serde(default)]
    pub user_id: Option<String>,
    /// Restrict to one event kind.
    #[serde(default)]
    pub event_kind: Option<String>,
    /// Max rows (newest first).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// `GET /api/almagest/admin/audit` — browse the audit log with filters.
pub async fn audit(
    State(state): State<AppState>,
    Extension(actor): Extension<CurrentUser>,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Vec<HistoryEntry>>> {
    actor.require_admin()?;
    let filter = HistoryFilter {
        user_id: q.user_id,
        event_kind: q.event_kind,
        limit: q.limit,
    };
    Ok(Json(state.file().list_history(&filter)?))
}

/// `POST /api/almagest/admin/auth/disable` — turn auth back off (accounts are
/// kept; the file just stops requiring login). Re-enabling re-uses the existing
/// accounts via the normal login flow.
pub async fn disable_auth(
    State(state): State<AppState>,
    Extension(actor): Extension<CurrentUser>,
) -> ApiResult<StatusCode> {
    actor.require_admin()?;
    state.ensure_writable()?;
    {
        let file = state.file();
        file.set_auth_enabled(false)?;
        file.append_history("auth_disabled", None, actor.audit_id(), None)?;
    }
    state.reload_auth()?;
    state.emit(ServerEvent::AuthChanged);
    Ok(StatusCode::NO_CONTENT)
}

// --- helpers -----------------------------------------------------------------

fn parse_role(s: &str) -> ApiResult<Role> {
    Role::from_str(s.trim()).map_err(|_| {
        ApiError::bad_request(format!(
            "invalid role '{s}'; expected admin, editor, or viewer"
        ))
    })
}

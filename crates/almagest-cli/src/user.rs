// SPDX-License-Identifier: MIT OR Apache-2.0

//! `almagest user list | add | reset-password | remove` — offline account
//! management for auth-enabled files (doc 14).
//!
//! These operate by **direct file access**: if you can run the CLI against the
//! `.alm` on disk you already fully control it, so — unlike the web admin API —
//! they don't require an authenticated admin session (the file is the access
//! boundary). Adding the *first* user also enables auth and generates the
//! session-signing secret, so this is the terminal equivalent of the web setup
//! flow. Password hashing reuses `almagest-server` (Argon2id) to keep crypto out
//! of `almagest-core`.

use crate::output::Out;
use almagest_core::{AlmagestFile, Role, User};
use almagest_server::{generate_secret, generate_temp_password, hash_password, validate_password};
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::str::FromStr;

/// `almagest user list`.
pub fn list(path: &Path, out: &Out) -> Result<()> {
    let file = open(path)?;
    let enabled = file.auth_enabled()?;
    let users = file.list_users()?;

    if out.json {
        return out.emit(&serde_json::json!({
            "auth_enabled": enabled,
            "users": users,
        }));
    }

    if !enabled {
        out.line("Auth: disabled (single-user mode)");
    } else {
        out.line("Auth: enabled");
    }
    if users.is_empty() {
        out.line("No user accounts.");
        return Ok(());
    }
    for u in &users {
        let last = u.last_login_at.as_deref().unwrap_or("never");
        println!("{:<24} {:<8} last login: {}", u.username, u.role, last);
    }
    Ok(())
}

/// `almagest user add --username U --role R [--email E] [--password P]`.
pub fn add(
    path: &Path,
    username: &str,
    role: &str,
    email: Option<&str>,
    password: Option<&str>,
    out: &Out,
) -> Result<()> {
    let file = open(path)?;
    let role = parse_role(role)?;
    let password = resolve_password(password, true)?;
    validate_password(&password).map_err(|e| anyhow::anyhow!(e.message))?;
    let hash = hash_password(&password).map_err(|e| anyhow::anyhow!(e.message))?;

    let first_user = file.count_users()? == 0;
    let user = file
        .create_user(username.trim(), &hash, role, email)
        .context("creating user")?;

    // First account bootstraps auth: generate the session secret + turn auth on,
    // mirroring the web first-admin setup flow.
    let mut enabled_auth = false;
    if first_user {
        file.set_session_secret(&generate_secret())?;
        file.set_auth_enabled(true)?;
        file.append_history("auth_enabled", None, Some(&user.id), None)?;
        enabled_auth = true;
    }
    file.append_history("user_created", Some(&user.id), Some(&user.id), None)?;
    file.close().context("finalizing the file")?;

    if out.json {
        return out.emit(&serde_json::json!({
            "user": user,
            "auth_enabled": enabled_auth || !first_user,
        }));
    }
    out.result(format!("Added user '{}' ({})", user.username, user.role));
    if enabled_auth {
        out.line("Auth is now enabled for this file — opening it will require login.");
    }
    Ok(())
}

/// `almagest user reset-password --username U` — set a random temp password.
pub fn reset_password(path: &Path, username: &str, out: &Out) -> Result<()> {
    let file = open(path)?;
    let user = find_user(&file, username)?;
    let temp = generate_temp_password();
    let hash = hash_password(&temp).map_err(|e| anyhow::anyhow!(e.message))?;
    file.set_password_hash(&user.id, &hash)?;
    file.append_history("password_reset", Some(&user.id), None, None)?;
    file.close().context("finalizing the file")?;

    if out.json {
        return out.emit(&serde_json::json!({
            "username": user.username,
            "temporary_password": temp,
        }));
    }
    out.result(format!(
        "Temporary password for '{}': {temp}",
        user.username
    ));
    out.line("The user should change it after their next login.");
    Ok(())
}

/// `almagest user remove --username U`.
pub fn remove(path: &Path, username: &str, yes: bool, out: &Out) -> Result<()> {
    let file = open(path)?;
    let user = find_user(&file, username)?;
    if user.role == Role::Admin && file.count_users_with_role(Role::Admin)? <= 1 {
        bail!("cannot remove the last admin; add another admin first");
    }
    if !yes && !out.json {
        crate::confirm(&format!("Remove user \"{}\"?", user.username))?;
    }
    file.delete_user(&user.id)?;
    file.append_history("user_deleted", Some(&user.id), None, None)?;
    file.close().context("finalizing the file")?;

    out.result(format!("Removed user '{}'", user.username));
    out.emit(&serde_json::json!({ "removed": user.username }))?;
    Ok(())
}

// --- helpers -----------------------------------------------------------------

fn open(path: &Path) -> Result<AlmagestFile> {
    if !path.exists() {
        bail!("{} does not exist", path.display());
    }
    AlmagestFile::open(path).with_context(|| format!("opening {}", path.display()))
}

fn parse_role(s: &str) -> Result<Role> {
    Role::from_str(s.trim())
        .map_err(|_| anyhow::anyhow!("invalid role '{s}'; expected admin, editor, or viewer"))
}

fn find_user(file: &AlmagestFile, username: &str) -> Result<User> {
    file.user_credentials(username.trim())?
        .map(|(u, _hash)| u)
        .ok_or_else(|| anyhow::anyhow!("no user named '{username}'"))
}

/// Resolve a password from (in priority order) the `--password` flag, the
/// `ALMAGEST_PASSWORD` env var, or an interactive hidden prompt.
fn resolve_password(flag: Option<&str>, confirm_match: bool) -> Result<String> {
    if let Some(p) = flag {
        return Ok(p.to_string());
    }
    if let Ok(p) = std::env::var("ALMAGEST_PASSWORD") {
        return Ok(p);
    }
    let p = rpassword::prompt_password("New password: ").context("reading password")?;
    if confirm_match {
        let again = rpassword::prompt_password("Confirm password: ").context("reading password")?;
        if p != again {
            bail!("passwords do not match");
        }
    }
    Ok(p)
}

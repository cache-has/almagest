// SPDX-License-Identifier: MIT OR Apache-2.0

//! Local user accounts and per-file auth configuration (doc 13).
//!
//! Auth is **optional and de-emphasized**: the headline "email a `.alm` to
//! someone" case has no auth at all — the file is the access boundary. This
//! module is the storage layer for the narrower shared-file case: local accounts
//! in `almagest_users` and a single-row `almagest_auth` config (enabled flag,
//! HMAC session-signing secret, session lifetime).
//!
//! Because Almagest holds **no live connections**, this auth only gates *who can
//! open a shared file* — there are no external database credentials in the file
//! to protect. Password *hashing* (Argon2id) and session *signing* live in
//! `almagest-server`; this layer stores opaque hash strings and reads/writes the
//! config, keeping crypto dependencies out of the format crate.

use crate::AlmagestFile;
use crate::error::{AlmagestError, Result};
use std::fmt;
use std::str::FromStr;

/// A coarse access role. Three roles cover the v1 cases; finer-grained
/// (per-dashboard) permissions are post-v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Full access: manage users, dashboards, data, and export/import.
    Admin,
    /// Create and edit dashboards and data, but cannot manage users.
    Editor,
    /// Read-only: view dashboards, use parameters, export snapshots.
    Viewer,
}

impl Role {
    /// The lowercase wire/storage string for this role.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Editor => "editor",
            Role::Viewer => "viewer",
        }
    }

    /// A monotonic privilege rank (admin highest) for `>=` comparisons.
    pub fn rank(self) -> u8 {
        match self {
            Role::Viewer => 0,
            Role::Editor => 1,
            Role::Admin => 2,
        }
    }

    /// Whether this role meets or exceeds `required`.
    pub fn satisfies(self, required: Role) -> bool {
        self.rank() >= required.rank()
    }

    /// Whether this role may create/modify dashboards and data.
    pub fn can_edit(self) -> bool {
        self.satisfies(Role::Editor)
    }

    /// Whether this role may manage users and file-level config.
    pub fn can_admin(self) -> bool {
        self == Role::Admin
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = AlmagestError;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "admin" => Ok(Role::Admin),
            "editor" => Ok(Role::Editor),
            "viewer" => Ok(Role::Viewer),
            other => Err(AlmagestError::Invalid(format!("unknown role '{other}'"))),
        }
    }
}

/// A local user account, **without** the password hash (which never leaves the
/// auth layer). Returned by the listing / lookup APIs and safe to serialize to
/// clients.
#[derive(Debug, Clone, serde::Serialize)]
pub struct User {
    /// Stable user id (UUID).
    pub id: String,
    /// Unique login name.
    pub username: String,
    /// Access role.
    pub role: Role,
    /// Optional email (informational; Almagest sends no mail).
    pub email: Option<String>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 timestamp of the last successful login, if any.
    pub last_login_at: Option<String>,
}

/// The per-file auth configuration (the single `almagest_auth` row).
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Whether auth is enforced for this file.
    pub enabled: bool,
    /// The HMAC session-signing secret (None until auth is first enabled).
    pub session_secret: Option<Vec<u8>>,
    /// Session lifetime in seconds (absolute cap; default 24h).
    pub session_lifetime_secs: i64,
}

impl AlmagestFile {
    // --- auth configuration ---------------------------------------------

    /// Read the file's auth configuration.
    pub fn auth_config(&self) -> Result<AuthConfig> {
        self.conn()
            .query_row(
                "SELECT enabled, session_secret, session_lifetime_secs
                 FROM almagest_auth WHERE id = 1",
                [],
                |r| {
                    let enabled: i64 = r.get(0)?;
                    let secret: Option<Vec<u8>> = r.get(1)?;
                    let lifetime: i64 = r.get(2)?;
                    Ok(AuthConfig {
                        enabled: enabled != 0,
                        session_secret: secret.filter(|s| !s.is_empty()),
                        session_lifetime_secs: lifetime,
                    })
                },
            )
            .map_err(AlmagestError::Sqlite)
    }

    /// Whether auth is currently enabled for this file.
    pub fn auth_enabled(&self) -> Result<bool> {
        Ok(self.auth_config()?.enabled)
    }

    /// Enable or disable auth enforcement.
    pub fn set_auth_enabled(&self, enabled: bool) -> Result<()> {
        let now = crate::now_rfc3339();
        self.conn().execute(
            "UPDATE almagest_auth SET enabled = ?1, updated_at = ?2 WHERE id = 1",
            rusqlite::params![enabled as i64, now],
        )?;
        Ok(())
    }

    /// Store the session-signing secret. Generated and supplied by the server
    /// (which owns randomness); core only persists it.
    pub fn set_session_secret(&self, secret: &[u8]) -> Result<()> {
        let now = crate::now_rfc3339();
        self.conn().execute(
            "UPDATE almagest_auth SET session_secret = ?1, updated_at = ?2 WHERE id = 1",
            rusqlite::params![secret, now],
        )?;
        Ok(())
    }

    /// Set the session lifetime (seconds).
    pub fn set_session_lifetime(&self, secs: i64) -> Result<()> {
        let now = crate::now_rfc3339();
        self.conn().execute(
            "UPDATE almagest_auth SET session_lifetime_secs = ?1, updated_at = ?2 WHERE id = 1",
            rusqlite::params![secs, now],
        )?;
        Ok(())
    }

    // --- user accounts --------------------------------------------------

    /// How many user accounts exist. Zero means the file is in no-auth /
    /// awaiting-first-admin state.
    pub fn count_users(&self) -> Result<u64> {
        let n: i64 = self
            .conn()
            .query_row("SELECT COUNT(*) FROM almagest_users", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// Create a user from a pre-computed password hash. Returns the stored
    /// [`User`]. Fails with [`AlmagestError::Conflict`] if the username is taken.
    pub fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        role: Role,
        email: Option<&str>,
    ) -> Result<User> {
        let username = username.trim();
        if username.is_empty() {
            return Err(AlmagestError::Invalid("username must not be empty".into()));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let now = crate::now_rfc3339();
        let res = self.conn().execute(
            "INSERT INTO almagest_users
               (id, username, password_hash, role, email, created_at, last_login_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            rusqlite::params![id, username, password_hash, role.as_str(), email, now],
        );
        match res {
            Ok(_) => Ok(User {
                id,
                username: username.to_string(),
                role,
                email: email.map(str::to_string),
                created_at: now,
                last_login_at: None,
            }),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(AlmagestError::Conflict {
                    kind: "user",
                    detail: format!("username '{username}' already exists"),
                })
            }
            Err(e) => Err(AlmagestError::Sqlite(e)),
        }
    }

    /// Look up a user by username, returning the [`User`] and its stored password
    /// hash (for verification). `None` if no such user.
    pub fn user_credentials(&self, username: &str) -> Result<Option<(User, String)>> {
        let row = self.conn().query_row(
            "SELECT id, username, password_hash, role, email, created_at, last_login_at
             FROM almagest_users WHERE username = ?1",
            [username],
            |r| {
                let hash: String = r.get(2)?;
                Ok((row_to_user(r)?, hash))
            },
        );
        match row {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AlmagestError::Sqlite(e)),
        }
    }

    /// Fetch a user by id.
    pub fn user_by_id(&self, id: &str) -> Result<Option<User>> {
        let row = self.conn().query_row(
            "SELECT id, username, password_hash, role, email, created_at, last_login_at
             FROM almagest_users WHERE id = ?1",
            [id],
            row_to_user,
        );
        match row {
            Ok(u) => Ok(Some(u)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AlmagestError::Sqlite(e)),
        }
    }

    /// List all users, ordered by username.
    pub fn list_users(&self) -> Result<Vec<User>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, username, password_hash, role, email, created_at, last_login_at
             FROM almagest_users ORDER BY username",
        )?;
        let rows = stmt.query_map([], row_to_user)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Count users with a given role (used to protect the last admin).
    pub fn count_users_with_role(&self, role: Role) -> Result<u64> {
        let n: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM almagest_users WHERE role = ?1",
            [role.as_str()],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    /// Change a user's role. Returns `true` if the user existed.
    pub fn set_user_role(&self, id: &str, role: Role) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE almagest_users SET role = ?2 WHERE id = ?1",
            rusqlite::params![id, role.as_str()],
        )?;
        Ok(n > 0)
    }

    /// Replace a user's password hash. Returns `true` if the user existed.
    pub fn set_password_hash(&self, id: &str, password_hash: &str) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE almagest_users SET password_hash = ?2 WHERE id = ?1",
            rusqlite::params![id, password_hash],
        )?;
        Ok(n > 0)
    }

    /// Record a successful login timestamp for a user.
    pub fn record_login(&self, id: &str) -> Result<()> {
        let now = crate::now_rfc3339();
        self.conn().execute(
            "UPDATE almagest_users SET last_login_at = ?2 WHERE id = ?1",
            rusqlite::params![id, now],
        )?;
        Ok(())
    }

    /// Delete a user by id. Returns `true` if one was removed.
    pub fn delete_user(&self, id: &str) -> Result<bool> {
        let n = self
            .conn()
            .execute("DELETE FROM almagest_users WHERE id = ?1", [id])?;
        Ok(n > 0)
    }
}

fn row_to_user(r: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    let role_str: String = r.get(3)?;
    let role = Role::from_str(&role_str).unwrap_or(Role::Viewer);
    Ok(User {
        id: r.get(0)?,
        username: r.get(1)?,
        role,
        email: r.get(4)?,
        created_at: r.get(5)?,
        last_login_at: r.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_file() -> (tempfile::TempDir, AlmagestFile) {
        let dir = tempfile::tempdir().unwrap();
        let file = AlmagestFile::create(dir.path().join("auth.alm")).unwrap();
        (dir, file)
    }

    #[test]
    fn fresh_file_is_no_auth_with_no_users() {
        let (_d, file) = new_file();
        let cfg = file.auth_config().unwrap();
        assert!(!cfg.enabled);
        assert!(cfg.session_secret.is_none());
        assert_eq!(cfg.session_lifetime_secs, 86400);
        assert_eq!(file.count_users().unwrap(), 0);
    }

    #[test]
    fn auth_config_round_trips() {
        let (_d, file) = new_file();
        file.set_session_secret(b"0123456789abcdef").unwrap();
        file.set_auth_enabled(true).unwrap();
        file.set_session_lifetime(3600).unwrap();
        let cfg = file.auth_config().unwrap();
        assert!(cfg.enabled);
        assert_eq!(
            cfg.session_secret.as_deref(),
            Some(&b"0123456789abcdef"[..])
        );
        assert_eq!(cfg.session_lifetime_secs, 3600);
    }

    #[test]
    fn user_crud_and_unique_username() {
        let (_d, file) = new_file();
        let u = file
            .create_user("alice", "hash1", Role::Admin, Some("a@x.io"))
            .unwrap();
        assert_eq!(u.role, Role::Admin);
        assert_eq!(file.count_users().unwrap(), 1);

        // Duplicate username conflicts.
        let dup = file.create_user("alice", "hash2", Role::Viewer, None);
        assert!(matches!(dup, Err(AlmagestError::Conflict { .. })));

        // Lookup by username returns the stored hash.
        let (got, hash) = file.user_credentials("alice").unwrap().unwrap();
        assert_eq!(got.id, u.id);
        assert_eq!(hash, "hash1");
        assert!(file.user_credentials("nobody").unwrap().is_none());

        // Role change + password change + login timestamp.
        assert!(file.set_user_role(&u.id, Role::Editor).unwrap());
        assert!(file.set_password_hash(&u.id, "hash3").unwrap());
        file.record_login(&u.id).unwrap();
        let reloaded = file.user_by_id(&u.id).unwrap().unwrap();
        assert_eq!(reloaded.role, Role::Editor);
        assert!(reloaded.last_login_at.is_some());
        let (_, hash) = file.user_credentials("alice").unwrap().unwrap();
        assert_eq!(hash, "hash3");

        // Role counting + delete.
        file.create_user("bob", "h", Role::Admin, None).unwrap();
        assert_eq!(file.count_users_with_role(Role::Admin).unwrap(), 1);
        assert_eq!(file.count_users_with_role(Role::Editor).unwrap(), 1);
        assert!(file.delete_user(&u.id).unwrap());
        assert!(!file.delete_user(&u.id).unwrap());
        assert_eq!(file.list_users().unwrap().len(), 1);
    }

    #[test]
    fn role_parsing_and_privilege() {
        assert_eq!("admin".parse::<Role>().unwrap(), Role::Admin);
        assert!("bogus".parse::<Role>().is_err());
        assert!(Role::Admin.satisfies(Role::Editor));
        assert!(Role::Editor.satisfies(Role::Viewer));
        assert!(!Role::Viewer.satisfies(Role::Editor));
        assert!(Role::Admin.can_admin());
        assert!(!Role::Editor.can_admin());
        assert!(Role::Editor.can_edit());
        assert!(!Role::Viewer.can_edit());
    }

    #[test]
    fn upgrades_a_v1_file_to_v2() {
        // Hand-build a v1 file: apply only migration 1 + its bookkeeping, then
        // open it and confirm the runner upgrades it to v2 (auth available).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.alm");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(crate::schema::MIGRATIONS[0].sql)
                .unwrap();
            conn.execute_batch(
                "CREATE TABLE almagest_migrations (version INTEGER PRIMARY KEY, \
                 applied_at TEXT NOT NULL, description TEXT) STRICT;",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO almagest_migrations (version, applied_at, description) \
                 VALUES (1, '2020-01-01T00:00:00Z', 'v1')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO almagest_metadata (key, value) VALUES ('format_version', '1')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO almagest_metadata (key, value) VALUES ('almagest_id', 'legacy')",
                [],
            )
            .unwrap();
        }

        let file = AlmagestFile::open(&path).unwrap();
        // The runner upgraded the schema and bumped the reported version.
        assert_eq!(file.format_version().unwrap(), crate::FORMAT_VERSION);
        // Auth config exists and defaults to disabled; the new user columns work.
        assert!(!file.auth_enabled().unwrap());
        let u = file.create_user("carol", "h", Role::Admin, None).unwrap();
        assert_eq!(file.user_by_id(&u.id).unwrap().unwrap().username, "carol");
    }

    #[test]
    fn history_append_and_filter() {
        let (_d, file) = new_file();
        file.append_history("login", None, Some("u1"), None)
            .unwrap();
        file.append_history("dashboard_updated", Some("d1"), Some("u1"), None)
            .unwrap();
        file.append_history("login", None, Some("u2"), None)
            .unwrap();

        let all = file.list_history(&Default::default()).unwrap();
        assert_eq!(all.len(), 3);
        // Newest first.
        assert_eq!(all[0].event_kind, "login");
        assert_eq!(all[0].user_id.as_deref(), Some("u2"));

        let filter = crate::HistoryFilter {
            event_kind: Some("login".into()),
            ..Default::default()
        };
        assert_eq!(file.list_history(&filter).unwrap().len(), 2);

        let filter = crate::HistoryFilter {
            user_id: Some("u1".into()),
            ..Default::default()
        };
        assert_eq!(file.list_history(&filter).unwrap().len(), 2);
    }
}

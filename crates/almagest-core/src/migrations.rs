// SPDX-License-Identifier: MIT OR Apache-2.0

//! The migration runner: applies the ordered [`schema::MIGRATIONS`] to a
//! SQLite connection and tracks which versions have run in `almagest_migrations`.
//!
//! On open, the runner reconciles the file's applied version with what this
//! build supports:
//!
//! - file newer than this build → refuse with [`AlmagestError::FormatTooNew`]
//! - file older (or fresh) → apply pending migrations in a transaction
//! - file current → no-op
//!
//! `almagest_migrations` is bookkeeping, created here rather than in a migration
//! so the runner can always read it.

use crate::FORMAT_VERSION;
use crate::error::{AlmagestError, Result};
use crate::schema::{self, Migration};
use rusqlite::Connection;

/// Bookkeeping table recording each applied migration version.
const MIGRATIONS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS almagest_migrations (
    version    INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL,
    description TEXT
) STRICT;
"#;

/// Highest migration version recorded in the file, or 0 if none applied.
fn applied_version(conn: &Connection) -> Result<u32> {
    let v: Option<i64> =
        conn.query_row("SELECT MAX(version) FROM almagest_migrations", [], |r| {
            r.get(0)
        })?;
    Ok(v.unwrap_or(0) as u32)
}

/// Bring `conn` up to [`FORMAT_VERSION`], applying any pending migrations.
///
/// Returns the version the file is at afterwards. Refuses (without modifying
/// the file) if the file is newer than this build supports.
pub fn migrate_to_current(conn: &Connection) -> Result<u32> {
    conn.execute_batch(MIGRATIONS_TABLE)?;
    let current = applied_version(conn)?;

    if current > FORMAT_VERSION {
        return Err(AlmagestError::FormatTooNew {
            found: current,
            supported: FORMAT_VERSION,
        });
    }
    if current == FORMAT_VERSION {
        return Ok(current);
    }

    for m in schema::MIGRATIONS.iter().filter(|m| m.version > current) {
        apply(conn, m)?;
    }
    Ok(FORMAT_VERSION)
}

/// Apply a single migration and record it, atomically.
fn apply(conn: &Connection, m: &Migration) -> Result<()> {
    let now = crate::now_rfc3339();
    conn.execute_batch("BEGIN")?;
    let result = (|| -> Result<()> {
        conn.execute_batch(m.sql)?;
        conn.execute(
            "INSERT INTO almagest_migrations (version, applied_at, description) VALUES (?1, ?2, ?3)",
            rusqlite::params![m.version, now, m.description],
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            tracing::debug!(version = m.version, "applied almagest migration");
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

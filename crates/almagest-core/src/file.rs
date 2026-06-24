// SPDX-License-Identifier: MIT OR Apache-2.0

//! [`AlmagestFile`] — the owned handle to an open `.alm` file.
//!
//! A `.alm` is a SQLite database; this type wraps the connection and enforces
//! the format guarantees: the schema is migrated to [`crate::FORMAT_VERSION`] on
//! every open, an integrity check runs before use, and WAL sidecars are
//! collapsed back into the single file on [`AlmagestFile::close`] so the
//! "it's one file" promise holds on disk.

use crate::error::{AlmagestError, Result};
use crate::{ALMAGEST_VERSION, migrations};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// An open handle to a `.alm` file.
///
/// Obtain one with [`AlmagestFile::create`] (new file) or [`AlmagestFile::open`]
/// (existing file). Always finish with [`AlmagestFile::close`] to checkpoint the
/// WAL and remove sidecar files; dropping without closing still flushes via
/// SQLite but may leave `-wal`/`-shm` files next to the `.alm`.
pub struct AlmagestFile {
    pub(crate) conn: Connection,
    path: PathBuf,
}

impl std::fmt::Debug for AlmagestFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The SQLite connection isn't Debug; surface the path instead.
        f.debug_struct("AlmagestFile")
            .field("path", &self.path)
            .finish()
    }
}

impl AlmagestFile {
    /// Create a new, empty `.alm` file at `path`.
    ///
    /// Fails if the path already exists — creating over a file would silently
    /// destroy a deliverable. Sets up WAL mode, applies the schema, and seeds
    /// `almagest_metadata` (format version, a fresh `almagest_id`, timestamps).
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            return Err(AlmagestError::Invalid(format!(
                "{} already exists; refusing to overwrite",
                path.display()
            )));
        }
        let conn = Connection::open(path)?;
        Self::apply_pragmas(&conn)?;
        migrations::migrate_to_current(&conn)?;

        let mut file = Self {
            conn,
            path: path.to_path_buf(),
        };
        file.seed_metadata()?;
        Ok(file)
    }

    /// Open an existing `.alm` file at `path`.
    ///
    /// Validates that the file is a Almagest file, reconciles its format version
    /// (refusing files newer than this build), applies any pending migrations,
    /// and runs an integrity check before returning.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(AlmagestError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} does not exist", path.display()),
            )));
        }
        let conn = Connection::open(path)?;
        Self::apply_pragmas(&conn)?;

        // Reject files that opened as SQLite but aren't Almagest files. A
        // pre-existing non-Almagest DB (or a brand-new empty file) has no
        // almagest_migrations table once we look — but migrate_to_current creates
        // that table, so we sniff for a almagest table *before* migrating.
        if !Self::looks_like_almagest(&conn, path)? {
            return Err(AlmagestError::NotAAlmagestFile {
                path: path.to_path_buf(),
                reason: "no almagest_metadata table; not a .alm file".to_string(),
            });
        }

        migrations::migrate_to_current(&conn)?;

        let file = Self {
            conn,
            path: path.to_path_buf(),
        };
        file.integrity_check()?;
        Ok(file)
    }

    /// The path this file lives at.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Close the file: checkpoint the WAL, drop the sidecar files, and release
    /// the connection. After this the `.alm` is, again, a single file.
    pub fn close(self) -> Result<()> {
        // Collapse the WAL back into the main file and switch off WAL so the
        // -wal/-shm sidecars are removed.
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        let _ = self.conn.pragma_update(None, "journal_mode", "DELETE");
        self.conn
            .close()
            .map_err(|(_, e)| AlmagestError::Sqlite(e))?;
        Ok(())
    }

    /// Run SQLite's integrity check plus Almagest-level sanity checks. Returns an
    /// [`AlmagestError::Integrity`] describing the first problem found.
    pub fn integrity_check(&self) -> Result<()> {
        let report: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if report != "ok" {
            return Err(AlmagestError::Integrity(report));
        }
        // Format version must be present and within range.
        let version = self.format_version()?;
        if version == 0 || version > crate::FORMAT_VERSION {
            return Err(AlmagestError::Integrity(format!(
                "format_version {version} out of supported range 1..={}",
                crate::FORMAT_VERSION
            )));
        }
        Ok(())
    }

    /// Borrow the underlying connection (internal; sibling modules build on it).
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Run `f` inside a transaction, committing on `Ok` and rolling back on
    /// `Err`. Keeps multi-statement writes consistent.
    pub(crate) fn with_tx<T>(
        &mut self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let tx = self.conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    // --- internals -------------------------------------------------------

    /// Pragmas applied to every connection: WAL for concurrency/crash-safety,
    /// foreign keys on, busy timeout so brief contention waits instead of
    /// erroring.
    fn apply_pragmas(conn: &Connection) -> Result<()> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        Ok(())
    }

    /// Whether the DB at `conn` already has Almagest's signature table. A 0-byte or
    /// foreign SQLite file returns false.
    fn looks_like_almagest(conn: &Connection, _path: &Path) -> Result<bool> {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='almagest_metadata'",
            [],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Seed `almagest_metadata` for a freshly created file.
    fn seed_metadata(&mut self) -> Result<()> {
        let now = crate::now_rfc3339();
        let almagest_id = uuid::Uuid::new_v4().to_string();
        self.with_tx(|tx| {
            let put = |k: &str, v: &str| -> Result<()> {
                tx.execute(
                    "INSERT INTO almagest_metadata (key, value) VALUES (?1, ?2)",
                    rusqlite::params![k, v],
                )?;
                Ok(())
            };
            put("format_version", &crate::FORMAT_VERSION.to_string())?;
            put("almagest_id", &almagest_id)?;
            put("created_at", &now)?;
            put("created_by_version", ALMAGEST_VERSION)?;
            put("title", "")?;
            put("description", "")?;
            Ok(())
        })
    }
}

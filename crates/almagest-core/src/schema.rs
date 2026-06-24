// SPDX-License-Identifier: MIT OR Apache-2.0

//! The `.alm` SQLite schema, expressed as ordered migrations.
//!
//! A `.alm` file *is* a SQLite database with `almagest_*` tables. The schema is
//! defined here as a list of [`Migration`]s applied in order by the migration
//! runner ([`crate::migrations`]). Migration 1 is the full v1 schema; later
//! breaking changes append new migrations and bump [`crate::FORMAT_VERSION`].
//!
//! See `planning/02-almagest-format.md` for the design rationale (SQLite container
//! + Parquet blobs, both open formats — "your files outlive the vendor").

/// One ordered, forward-only schema migration.
///
/// `version` is the [`crate::FORMAT_VERSION`] this migration brings the file up
/// to. `sql` is applied inside a transaction; it must be idempotent-safe only
/// to the extent the runner guarantees (each version runs exactly once, tracked
/// in `almagest_migrations`).
pub struct Migration {
    /// Format version this migration produces. Must equal its 1-based index.
    pub version: u32,
    /// Short human description, recorded in `almagest_migrations.description`.
    pub description: &'static str,
    /// The DDL/DML to apply for this version.
    pub sql: &'static str,
}

/// The full v1 schema. Tables are prefixed `almagest_` (a reserved namespace);
/// bulk row data lives only in `almagest_data` as Parquet blobs, never as
/// user-owned SQLite tables.
const V1_SQL: &str = r#"
-- Format metadata: version, identity, title/description, timestamps.
CREATE TABLE almagest_metadata (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

-- Bulk row data — compressed Parquet blobs, decoded to an Arrow/DataFusion
-- MemTable on open and registered under `name` for querying (doc 03).
CREATE TABLE almagest_data (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL UNIQUE,
    parquet_blob      BLOB NOT NULL,
    arrow_schema_json TEXT NOT NULL,
    row_count         INTEGER NOT NULL,
    byte_size         INTEGER NOT NULL,
    compression       TEXT NOT NULL,
    source_json       TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
) STRICT;

-- Saved queries: DataFusion SQL over registered almagest_data tables.
CREATE TABLE almagest_queries (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    sql             TEXT NOT NULL,
    parameters_json TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
) STRICT;

-- Optional file-level password protection (deliverable lock). Opt-in and
-- de-emphasized: most .alm files are unprotected. Crypto wiring is deferred;
-- the table exists now so enabling it later needs no migration.
CREATE TABLE almagest_secrets (
    id         TEXT PRIMARY KEY,
    ciphertext BLOB NOT NULL,
    nonce      BLOB NOT NULL,
    kdf_params TEXT NOT NULL
) STRICT;

-- Dashboard definitions (panels, layout, parameters) as JSON.
CREATE TABLE almagest_dashboards (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    description     TEXT,
    definition_json TEXT NOT NULL,
    folder          TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
) STRICT;

-- Query result cache: Arrow IPC bytes keyed by hash of query + parameters.
CREATE TABLE almagest_cache (
    cache_key    TEXT PRIMARY KEY,
    result_arrow BLOB NOT NULL,
    created_at   TEXT NOT NULL,
    expires_at   TEXT,
    row_count    INTEGER,
    byte_size    INTEGER
) STRICT;

-- Execution history / audit log.
CREATE TABLE almagest_history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    event_kind  TEXT NOT NULL,
    entity_id   TEXT,
    user_id     TEXT,
    payload_json TEXT,
    occurred_at TEXT NOT NULL
) STRICT;

-- Optional user accounts for multi-user files.
CREATE TABLE almagest_users (
    id            TEXT PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL,
    created_at    TEXT NOT NULL
) STRICT;

-- Embedded presentation assets (images, custom CSS, fonts) — never row data.
CREATE TABLE almagest_assets (
    id           TEXT PRIMARY KEY,
    path         TEXT NOT NULL UNIQUE,
    content_type TEXT NOT NULL,
    content      BLOB NOT NULL,
    created_at   TEXT NOT NULL
) STRICT;
"#;

/// All migrations, in apply order. The last entry's `version` must equal
/// [`crate::FORMAT_VERSION`] (asserted by a test).
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    description: "initial almagest v1 schema",
    sql: V1_SQL,
}];

#[cfg(test)]
mod tests {
    use super::MIGRATIONS;
    use crate::FORMAT_VERSION;

    #[test]
    fn migrations_are_dense_and_ordered() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version, i as u32 + 1, "migration {i} has a gap/misorder");
        }
    }

    #[test]
    fn last_migration_matches_format_version() {
        assert_eq!(
            MIGRATIONS.last().unwrap().version,
            FORMAT_VERSION,
            "FORMAT_VERSION must equal the highest migration version"
        );
    }
}

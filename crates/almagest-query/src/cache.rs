// SPDX-License-Identifier: MIT OR Apache-2.0

//! The query result cache, persisted in the `.alm` file's `almagest_cache`
//! table as Arrow IPC bytes.
//!
//! Because the cache lives in the file, sharing a `.alm` shares its warm
//! cache — the recipient sees results instantly. Cache keys fold in a
//! *data-version* fingerprint of `almagest_data`, so re-baking the data changes
//! every key and old entries simply stop matching (and age out via TTL / size
//! eviction). The cache owns its own SQLite connection to the file; `almagest-core`
//! stays focused on format primitives.

use crate::error::Result;
use arrow::datatypes::SchemaRef;
use arrow::ipc::reader::StreamReader;
use arrow::ipc::writer::StreamWriter;
use arrow::record_batch::RecordBatch;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;

/// Default cache size budget before LRU-ish eviction kicks in (50 MB).
pub const DEFAULT_MAX_BYTES: u64 = 50 * 1024 * 1024;
/// Default time-to-live for a cache entry (1 hour).
pub const DEFAULT_TTL_SECONDS: i64 = 3600;

/// File-backed query result cache.
pub struct AlmagestCache {
    conn: Mutex<Connection>,
    data_version: String,
    ttl_seconds: i64,
    max_bytes: u64,
}

impl AlmagestCache {
    /// Open a cache over the `.alm` at `path`, keyed against `data_version`.
    pub fn open(
        path: &Path,
        data_version: String,
        ttl_seconds: i64,
        max_bytes: u64,
    ) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        Ok(Self {
            conn: Mutex::new(conn),
            data_version,
            ttl_seconds,
            max_bytes,
        })
    }

    /// Compute the cache key for an already-substituted SQL string. The
    /// data-version is folded in so a re-bake invalidates every prior entry.
    pub fn key(&self, final_sql: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.data_version.as_bytes());
        hasher.update(b"\n");
        hasher.update(final_sql.as_bytes());
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(64);
        for byte in digest {
            hex.push_str(&format!("{byte:02x}"));
        }
        hex
    }

    /// Look up a fresh (non-expired) cached result. Returns the schema and
    /// batches on a hit.
    pub fn get(&self, key: &str) -> Result<Option<(SchemaRef, Vec<RecordBatch>)>> {
        let now = now_rfc3339();
        let conn = self.conn.lock().expect("cache mutex poisoned");
        let blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT result_arrow FROM almagest_cache
                 WHERE cache_key = ?1 AND (expires_at IS NULL OR expires_at > ?2)",
                rusqlite::params![key, now],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        drop(conn);

        match blob {
            Some(bytes) => Ok(Some(decode_ipc(bytes)?)),
            None => Ok(None),
        }
    }

    /// Store a result under `key`, then enforce the size budget. The TTL is
    /// applied from now.
    pub fn put(&self, key: &str, schema: &SchemaRef, batches: &[RecordBatch]) -> Result<()> {
        let bytes = encode_ipc(schema, batches)?;
        let byte_size = bytes.len() as i64;
        let row_count: i64 = batches.iter().map(|b| b.num_rows() as i64).sum();
        let now = chrono::Utc::now();
        let created_at = now.to_rfc3339();
        let expires_at = (now + chrono::Duration::seconds(self.ttl_seconds)).to_rfc3339();

        let conn = self.conn.lock().expect("cache mutex poisoned");
        conn.execute(
            "INSERT INTO almagest_cache
               (cache_key, result_arrow, created_at, expires_at, row_count, byte_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(cache_key) DO UPDATE SET
                result_arrow = excluded.result_arrow,
                created_at   = excluded.created_at,
                expires_at   = excluded.expires_at,
                row_count    = excluded.row_count,
                byte_size    = excluded.byte_size",
            rusqlite::params![key, bytes, created_at, expires_at, row_count, byte_size],
        )?;
        evict_to_budget(&conn, self.max_bytes)?;
        Ok(())
    }

    /// Remove all cache entries (e.g. after the caller knows the data changed).
    pub fn invalidate_all(&self) -> Result<()> {
        let conn = self.conn.lock().expect("cache mutex poisoned");
        conn.execute("DELETE FROM almagest_cache", [])?;
        Ok(())
    }

    /// Delete entries past their TTL.
    pub fn purge_expired(&self) -> Result<usize> {
        let now = now_rfc3339();
        let conn = self.conn.lock().expect("cache mutex poisoned");
        let n = conn.execute(
            "DELETE FROM almagest_cache WHERE expires_at IS NOT NULL AND expires_at <= ?1",
            [now],
        )?;
        Ok(n)
    }
}

/// Evict oldest-first until the total cached bytes fit the budget. RFC3339
/// timestamps sort chronologically, so `created_at` ascending is oldest-first.
fn evict_to_budget(conn: &Connection, max_bytes: u64) -> Result<()> {
    loop {
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(byte_size), 0) FROM almagest_cache",
            [],
            |r| r.get(0),
        )?;
        if (total as u64) <= max_bytes {
            break;
        }
        let victim: Option<String> = conn
            .query_row(
                "SELECT cache_key FROM almagest_cache ORDER BY created_at ASC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        match victim {
            Some(k) => {
                conn.execute("DELETE FROM almagest_cache WHERE cache_key = ?1", [k])?;
            }
            None => break,
        }
    }
    Ok(())
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Encode batches as a self-describing Arrow IPC stream (schema travels with the
/// data, so a zero-batch result round-trips its schema).
fn encode_ipc(schema: &SchemaRef, batches: &[RecordBatch]) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut writer = StreamWriter::try_new(&mut buf, schema)?;
        for batch in batches {
            writer.write(batch)?;
        }
        writer.finish()?;
    }
    Ok(buf)
}

/// Decode an Arrow IPC stream back into schema + batches.
fn decode_ipc(bytes: Vec<u8>) -> Result<(SchemaRef, Vec<RecordBatch>)> {
    let reader = StreamReader::try_new(std::io::Cursor::new(bytes), None)?;
    let schema = reader.schema();
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch?);
    }
    Ok((schema, batches))
}

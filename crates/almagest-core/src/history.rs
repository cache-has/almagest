// SPDX-License-Identifier: MIT OR Apache-2.0

//! Audit / history logging in `almagest_history` (doc 13).
//!
//! Significant actions — logins, dashboard and data mutations, user management —
//! are appended here with optional user attribution. Admins browse the log via
//! the API with filtering by user, event kind, and a row limit. Because Almagest
//! has no live connections, there are no per-connection events to audit; the
//! data events are author-time ingest.

use crate::AlmagestFile;
use crate::error::Result;

/// One audit-log entry.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryEntry {
    /// Auto-increment row id.
    pub id: i64,
    /// Event kind (e.g. `login`, `dashboard_updated`, `user_created`).
    pub event_kind: String,
    /// Optional id of the affected entity (dashboard id, user id, dataset name).
    pub entity_id: Option<String>,
    /// Optional id of the acting user.
    pub user_id: Option<String>,
    /// Optional JSON payload with extra detail.
    pub payload_json: Option<String>,
    /// RFC 3339 timestamp.
    pub occurred_at: String,
}

/// Filter for [`AlmagestFile::list_history`]. Defaults select everything.
#[derive(Debug, Clone, Default)]
pub struct HistoryFilter {
    /// Restrict to a single acting user.
    pub user_id: Option<String>,
    /// Restrict to a single event kind.
    pub event_kind: Option<String>,
    /// Max rows to return (newest first). `None` → a sane default cap.
    pub limit: Option<u32>,
}

impl AlmagestFile {
    /// Append an audit-log entry. Best-effort attribution: any of `entity_id` /
    /// `user_id` / `payload_json` may be absent.
    pub fn append_history(
        &self,
        event_kind: &str,
        entity_id: Option<&str>,
        user_id: Option<&str>,
        payload_json: Option<&str>,
    ) -> Result<()> {
        let now = crate::now_rfc3339();
        self.conn().execute(
            "INSERT INTO almagest_history
               (event_kind, entity_id, user_id, payload_json, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![event_kind, entity_id, user_id, payload_json, now],
        )?;
        Ok(())
    }

    /// List audit-log entries newest-first, applying `filter`.
    pub fn list_history(&self, filter: &HistoryFilter) -> Result<Vec<HistoryEntry>> {
        // Cap the result set; the admin UI paginates by re-querying with a limit.
        let limit = filter.limit.unwrap_or(500).min(5000) as i64;
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, event_kind, entity_id, user_id, payload_json, occurred_at
             FROM almagest_history
             WHERE (?1 IS NULL OR user_id = ?1)
               AND (?2 IS NULL OR event_kind = ?2)
             ORDER BY id DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![filter.user_id, filter.event_kind, limit],
            |r| {
                Ok(HistoryEntry {
                    id: r.get(0)?,
                    event_kind: r.get(1)?,
                    entity_id: r.get(2)?,
                    user_id: r.get(3)?,
                    payload_json: r.get(4)?,
                    occurred_at: r.get(5)?,
                })
            },
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

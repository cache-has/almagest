// SPDX-License-Identifier: MIT OR Apache-2.0

//! Read access to saved queries in `almagest_queries`.
//!
//! A dashboard panel's data source is either inline SQL or a reference to a
//! saved query ([`crate::Query::Reference`]). The server resolves a reference by
//! looking the SQL up here at panel-execution time. Write-side CRUD for saved
//! queries lands with the editor phase (doc 09); this read getter is what the
//! runtime needs.

use crate::AlmagestFile;
use crate::error::{AlmagestError, Result};

/// A saved query: reusable DataFusion SQL over the file's embedded tables,
/// referenced from a panel by `query_id`.
#[derive(Debug, Clone)]
pub struct SavedQuery {
    /// Stable query id, referenced by [`crate::Query::Reference`].
    pub id: String,
    /// Human name.
    pub name: String,
    /// The SQL, possibly with `{{param}}` templating.
    pub sql: String,
    /// Optional JSON-encoded parameter schema for the query.
    pub parameters_json: Option<String>,
}

impl AlmagestFile {
    /// Fetch a saved query by id. Errors with [`AlmagestError::NotFound`] if the
    /// id is unknown.
    pub fn saved_query(&self, id: &str) -> Result<SavedQuery> {
        self.conn()
            .query_row(
                "SELECT id, name, sql, parameters_json
                 FROM almagest_queries WHERE id = ?1",
                [id],
                |r| {
                    Ok(SavedQuery {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        sql: r.get(2)?,
                        parameters_json: r.get(3)?,
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => AlmagestError::NotFound {
                    kind: "query",
                    id: id.to_string(),
                },
                other => AlmagestError::Sqlite(other),
            })
    }

    /// List saved queries (id and name), ordered by name.
    pub fn list_saved_queries(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, name FROM almagest_queries ORDER BY name")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Query-result cache stats: `(entry_count, total_bytes)`. Used by
    /// `almagest info` for a quick health picture.
    pub fn cache_stats(&self) -> Result<(u64, u64)> {
        self.conn()
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(byte_size), 0) FROM almagest_cache",
                [],
                |r| {
                    let count: i64 = r.get(0)?;
                    let bytes: i64 = r.get(1)?;
                    Ok((count as u64, bytes as u64))
                },
            )
            .map_err(AlmagestError::Sqlite)
    }
}

#[cfg(test)]
mod tests {
    use crate::AlmagestFile;

    fn temp_file() -> (tempfile::TempDir, AlmagestFile) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("q.alm");
        let file = AlmagestFile::create(&path).unwrap();
        (dir, file)
    }

    #[test]
    fn saved_query_round_trip() {
        let (_dir, file) = temp_file();
        file.conn()
            .execute(
                "INSERT INTO almagest_queries (id, name, sql, parameters_json, created_at, updated_at)
                 VALUES ('q1', 'Revenue', 'SELECT 1', NULL, '2026-06-24T00:00:00Z', '2026-06-24T00:00:00Z')",
                [],
            )
            .unwrap();

        let q = file.saved_query("q1").unwrap();
        assert_eq!(q.id, "q1");
        assert_eq!(q.name, "Revenue");
        assert_eq!(q.sql, "SELECT 1");
        assert!(q.parameters_json.is_none());

        let all = file.list_saved_queries().unwrap();
        assert_eq!(all, vec![("q1".to_string(), "Revenue".to_string())]);
    }

    #[test]
    fn saved_query_missing_is_not_found() {
        let (_dir, file) = temp_file();
        let err = file.saved_query("nope").unwrap_err();
        assert!(matches!(
            err,
            crate::AlmagestError::NotFound { kind: "query", .. }
        ));
    }
}

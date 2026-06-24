// SPDX-License-Identifier: MIT OR Apache-2.0

//! Persistence for dashboard definitions in `almagest_dashboards`.
//!
//! Two layers sit here: the **raw** CRUD ([`AlmagestFile::create_dashboard`] etc.)
//! stores a dashboard as an opaque-but-valid JSON string, and the **typed**
//! layer ([`AlmagestFile::save_dashboard`] / [`AlmagestFile::load_dashboard`]) goes
//! through the [`Dashboard`] DSL (doc 05) — parsing, validating, and (on import)
//! checking that referenced saved queries exist.

use crate::AlmagestFile;
use crate::dashboard::{Dashboard, Query};
use crate::error::{AlmagestError, Result};
use std::path::Path;

/// A stored dashboard record. `definition_json` is the full dashboard
/// definition; almagest-core treats it as opaque (but valid) JSON.
#[derive(Debug, Clone)]
pub struct DashboardRecord {
    /// Stable dashboard id (UUID).
    pub id: String,
    /// Human name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Optional folder path for organization.
    pub folder: Option<String>,
    /// The dashboard definition as a JSON string.
    pub definition_json: String,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 last-update timestamp.
    pub updated_at: String,
}

impl AlmagestFile {
    /// Create a new dashboard, returning its generated id. `definition_json`
    /// must be valid JSON (parsed-and-discarded as a guard against storing
    /// garbage; the shape is not yet validated).
    pub fn create_dashboard(
        &mut self,
        name: &str,
        description: Option<&str>,
        folder: Option<&str>,
        definition_json: &str,
    ) -> Result<String> {
        validate_json(definition_json)?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = crate::now_rfc3339();
        self.conn().execute(
            "INSERT INTO almagest_dashboards
               (id, name, description, definition_json, folder, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            rusqlite::params![id, name, description, definition_json, folder, now],
        )?;
        Ok(id)
    }

    /// Fetch a dashboard by id.
    pub fn dashboard(&self, id: &str) -> Result<DashboardRecord> {
        self.conn()
            .query_row(
                "SELECT id, name, description, folder, definition_json, created_at, updated_at
                 FROM almagest_dashboards WHERE id = ?1",
                [id],
                row_to_dashboard,
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => AlmagestError::NotFound {
                    kind: "dashboard",
                    id: id.to_string(),
                },
                other => AlmagestError::Sqlite(other),
            })
    }

    /// Replace a dashboard's definition and metadata. Errors if the id is
    /// unknown.
    pub fn update_dashboard(
        &mut self,
        id: &str,
        name: &str,
        description: Option<&str>,
        folder: Option<&str>,
        definition_json: &str,
    ) -> Result<()> {
        validate_json(definition_json)?;
        let now = crate::now_rfc3339();
        let n = self.conn().execute(
            "UPDATE almagest_dashboards
                SET name = ?2, description = ?3, folder = ?4,
                    definition_json = ?5, updated_at = ?6
              WHERE id = ?1",
            rusqlite::params![id, name, description, folder, definition_json, now],
        )?;
        if n == 0 {
            return Err(AlmagestError::NotFound {
                kind: "dashboard",
                id: id.to_string(),
            });
        }
        Ok(())
    }

    /// List dashboards (metadata + definition), ordered by name.
    pub fn list_dashboards(&self) -> Result<Vec<DashboardRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, folder, definition_json, created_at, updated_at
             FROM almagest_dashboards ORDER BY name",
        )?;
        let rows = stmt.query_map([], row_to_dashboard)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Delete a dashboard by id. Returns `true` if one was removed.
    pub fn remove_dashboard(&mut self, id: &str) -> Result<bool> {
        let n = self
            .conn()
            .execute("DELETE FROM almagest_dashboards WHERE id = ?1", [id])?;
        Ok(n > 0)
    }

    // --- typed DSL layer (doc 05) ---------------------------------------

    /// Save a typed [`Dashboard`], validating it first. Name/description come
    /// from the dashboard; `folder` is organizational metadata outside the DSL.
    /// Returns the new dashboard id.
    pub fn save_dashboard(&mut self, dash: &Dashboard, folder: Option<&str>) -> Result<String> {
        let json = dash.to_json()?; // validates
        self.create_dashboard(&dash.name, dash.description.as_deref(), folder, &json)
    }

    /// Load and parse a typed [`Dashboard`] by id (validating on the way out).
    pub fn load_dashboard(&self, id: &str) -> Result<Dashboard> {
        let record = self.dashboard(id)?;
        Dashboard::from_json(&record.definition_json)
    }

    /// Replace an existing dashboard with a typed [`Dashboard`] (validated).
    pub fn update_dashboard_typed(
        &mut self,
        id: &str,
        dash: &Dashboard,
        folder: Option<&str>,
    ) -> Result<()> {
        let json = dash.to_json()?;
        self.update_dashboard(id, &dash.name, dash.description.as_deref(), folder, &json)
    }

    /// Export a dashboard to a standalone, git-diffable JSON file.
    pub fn export_dashboard_json(&self, id: &str, path: impl AsRef<Path>) -> Result<()> {
        let dash = self.load_dashboard(id)?;
        std::fs::write(path, dash.to_json_pretty()?)?;
        Ok(())
    }

    /// Import a dashboard from a standalone JSON file: parse, validate, check
    /// that any referenced saved queries exist in this file, then save it.
    /// Returns the new dashboard id.
    ///
    /// Note: validating that the *tables* a query references are embedded in the
    /// file requires SQL parsing and is deferred; query-id references are checked
    /// here.
    pub fn import_dashboard_json(
        &mut self,
        path: impl AsRef<Path>,
        folder: Option<&str>,
    ) -> Result<String> {
        let json = std::fs::read_to_string(path)?;
        self.import_dashboard(&json, folder)
    }

    /// Import a dashboard from a standalone JSON string (the in-memory form of
    /// [`AlmagestFile::import_dashboard_json`]): parse, validate, check that any
    /// referenced saved queries exist, then save it. Returns the new id. The
    /// HTTP import endpoint uses this so it never touches the filesystem.
    pub fn import_dashboard(&mut self, json: &str, folder: Option<&str>) -> Result<String> {
        let dash = Dashboard::from_json(json)?;
        self.check_query_references(&dash)?;
        self.save_dashboard(&dash, folder)
    }

    /// Verify every `query_id` reference in the dashboard resolves to a saved
    /// query in this file.
    fn check_query_references(&self, dash: &Dashboard) -> Result<()> {
        for row in &dash.layout.rows {
            for panel in &row.panels {
                if let Some(Query::Reference { query_id }) = &panel.query
                    && !self.saved_query_exists(query_id)?
                {
                    return Err(AlmagestError::InvalidDashboard {
                        location: format!("panel '{}'", panel.id),
                        detail: format!("references saved query '{query_id}' which does not exist"),
                    });
                }
            }
        }
        Ok(())
    }

    /// Whether a saved query with `id` exists in `almagest_queries`. (Full
    /// saved-query CRUD lands with the query layer; this is the existence check
    /// the dashboard importer needs today.)
    fn saved_query_exists(&self, id: &str) -> Result<bool> {
        let n: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM almagest_queries WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
}

fn row_to_dashboard(r: &rusqlite::Row<'_>) -> rusqlite::Result<DashboardRecord> {
    Ok(DashboardRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        folder: r.get(3)?,
        definition_json: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

/// Ensure a string is valid JSON before storing it.
fn validate_json(s: &str) -> Result<()> {
    serde_json::from_str::<serde_json::Value>(s)?;
    Ok(())
}

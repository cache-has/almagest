// SPDX-License-Identifier: MIT OR Apache-2.0

//! The JSON-over-HTTP API handlers.
//!
//! These back the single client boundary the frontend talks to (doc 08). The
//! runtime/viewer surface — metadata, dashboard fetch, schema, parameter
//! options, and panel execution returning Arrow IPC — is the load-bearing part;
//! the dashboard CRUD and import/export round it out for the editor. There are
//! deliberately **no connection endpoints**: Almagest is embedded-only, so the
//! doc's connection routes are cut.

use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, ServerEvent};
use almagest_core::{Dashboard, Panel, Query};
use arrow::ipc::writer::StreamWriter;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Content type for an Arrow IPC stream response (panel execution).
const ARROW_STREAM_MIME: &str = "application/vnd.apache.arrow.stream";

// --- metadata ----------------------------------------------------------------

/// File-level metadata returned by `GET /api/almagest`.
#[derive(Serialize)]
pub struct AlmagestMeta {
    /// The file's stable identity (UUID from `almagest_metadata`).
    pub id: String,
    /// Display title.
    pub title: String,
    /// Description (empty string if unset).
    pub description: String,
    /// The `.alm` format version of this file.
    pub format_version: u32,
    /// The running server/binary version.
    pub server_version: &'static str,
    /// Number of dashboards in the file.
    pub dashboard_count: usize,
}

/// `GET /api/almagest` — file identity, title, version, dashboard count.
pub async fn get_meta(State(state): State<AppState>) -> ApiResult<Json<AlmagestMeta>> {
    let file = state.file();
    Ok(Json(AlmagestMeta {
        id: file.almagest_id()?,
        title: file.title()?,
        description: file.description()?,
        format_version: file.format_version()?,
        server_version: almagest_core::ALMAGEST_VERSION,
        dashboard_count: file.list_dashboards()?.len(),
    }))
}

// --- dashboards --------------------------------------------------------------

/// A dashboard list entry (metadata only — no definition body).
#[derive(Serialize)]
pub struct DashboardSummary {
    /// Stable dashboard id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Optional organizational folder.
    pub folder: Option<String>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 last-update timestamp.
    pub updated_at: String,
}

/// `GET /api/almagest/dashboards` — list dashboards (metadata only).
pub async fn list_dashboards(
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<DashboardSummary>>> {
    let file = state.file();
    let out = file
        .list_dashboards()?
        .into_iter()
        .map(|d| DashboardSummary {
            id: d.id,
            name: d.name,
            description: d.description,
            folder: d.folder,
            created_at: d.created_at,
            updated_at: d.updated_at,
        })
        .collect();
    Ok(Json(out))
}

/// `GET /api/almagest/dashboards/:id` — the full typed dashboard definition.
pub async fn get_dashboard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Dashboard>> {
    let dash = state.file().load_dashboard(&id)?;
    Ok(Json(dash))
}

/// Body for create/update: the typed dashboard plus an optional folder.
#[derive(Deserialize)]
pub struct DashboardWrite {
    /// The dashboard definition.
    #[serde(flatten)]
    pub dashboard: Dashboard,
    /// Optional organizational folder (outside the DSL).
    #[serde(default)]
    pub folder: Option<String>,
}

/// Identifier echoed back after a create/import.
#[derive(Serialize)]
pub struct CreatedId {
    /// The new entity's id.
    pub id: String,
}

/// `POST /api/almagest/dashboards` — create a dashboard from a typed body.
pub async fn create_dashboard(
    State(state): State<AppState>,
    Json(body): Json<DashboardWrite>,
) -> ApiResult<(StatusCode, Json<CreatedId>)> {
    let id = {
        let mut file = state.file();
        file.save_dashboard(&body.dashboard, body.folder.as_deref())?
    };
    state.emit(ServerEvent::DashboardUpdated {
        dashboard_id: id.clone(),
    });
    Ok((StatusCode::CREATED, Json(CreatedId { id })))
}

/// `PUT /api/almagest/dashboards/:id` — replace a dashboard definition.
pub async fn update_dashboard(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<DashboardWrite>,
) -> ApiResult<StatusCode> {
    {
        let mut file = state.file();
        file.update_dashboard_typed(&id, &body.dashboard, body.folder.as_deref())?;
    }
    state.emit(ServerEvent::DashboardUpdated { dashboard_id: id });
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/almagest/dashboards/:id` — remove a dashboard.
pub async fn delete_dashboard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let removed = {
        let mut file = state.file();
        file.remove_dashboard(&id)?
    };
    if !removed {
        return Err(ApiError::not_found(format!("dashboard '{id}' not found")));
    }
    state.emit(ServerEvent::DashboardDeleted { dashboard_id: id });
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/almagest/export/dashboard/:id` — standalone, git-diffable JSON.
pub async fn export_dashboard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dash = state.file().load_dashboard(&id)?;
    let json = dash.to_json_pretty()?;
    let filename = format!("{}.dashboard.json", sanitize_filename(&dash.name));
    Ok((
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        json,
    ))
}

/// `POST /api/almagest/import/dashboard` — import a standalone dashboard JSON.
pub async fn import_dashboard(
    State(state): State<AppState>,
    Json(dashboard): Json<Dashboard>,
) -> ApiResult<(StatusCode, Json<CreatedId>)> {
    // Re-serialize to reuse the core import path (parse + query-ref check + save).
    let json = serde_json::to_string(&dashboard)
        .map_err(|e| ApiError::bad_request(format!("invalid dashboard: {e}")))?;
    let id = {
        let mut file = state.file();
        file.import_dashboard(&json, None)?
    };
    state.emit(ServerEvent::DashboardUpdated {
        dashboard_id: id.clone(),
    });
    Ok((StatusCode::CREATED, Json(CreatedId { id })))
}

// --- schema ------------------------------------------------------------------

/// `GET /api/almagest/schema` — the registered tables and their columns.
pub async fn get_schema(State(state): State<AppState>) -> Json<almagest_query::DatabaseSchema> {
    Json(state.query.schema())
}

// --- panel execution ---------------------------------------------------------

/// Body for `POST /api/almagest/panels/execute`.
#[derive(Deserialize)]
pub struct PanelExecuteRequest {
    /// The dashboard owning the panel.
    pub dashboard_id: String,
    /// The panel to run.
    pub panel_id: String,
    /// Raw parameter inputs keyed by parameter id (layered by the caller).
    #[serde(default)]
    pub parameters: BTreeMap<String, serde_json::Value>,
}

/// `POST /api/almagest/panels/execute` — resolve parameters, run the panel's
/// query, and stream the result as Arrow IPC bytes.
pub async fn execute_panel(
    State(state): State<AppState>,
    Json(req): Json<PanelExecuteRequest>,
) -> ApiResult<impl IntoResponse> {
    // Pull everything we need out of the file under the lock, then release it
    // before the async query — the std Mutex must never be held across `.await`.
    let (sql, decls) = {
        let file = state.file();
        let dash = file.load_dashboard(&req.dashboard_id)?;
        let panel = find_panel(&dash, &req.panel_id).ok_or_else(|| {
            ApiError::not_found(format!(
                "panel '{}' not found in dashboard '{}'",
                req.panel_id, req.dashboard_id
            ))
        })?;
        let sql = match &panel.query {
            Some(Query::Inline { sql }) => sql.clone(),
            Some(Query::Reference { query_id }) => file.saved_query(query_id)?.sql,
            None => {
                return Err(ApiError::bad_request(format!(
                    "panel '{}' is a {} panel and has no query to execute",
                    panel.id,
                    panel.kind.name()
                )));
            }
        };
        (sql, dash.parameters.clone())
    };

    let today = chrono::Utc::now().date_naive();
    let params = almagest_query::resolve_parameters(&decls, &req.parameters, today)?;
    let result = state.query.execute(&sql, &params).await?;

    let body = encode_arrow_ipc(&result)?;
    let headers = [
        (header::CONTENT_TYPE, ARROW_STREAM_MIME.to_string()),
        (
            header::HeaderName::from_static("x-almagest-row-count"),
            result.row_count.to_string(),
        ),
        (
            header::HeaderName::from_static("x-almagest-cached"),
            result.cached.to_string(),
        ),
    ];
    Ok((headers, body))
}

/// Encode a query result as a single Arrow IPC stream.
fn encode_arrow_ipc(result: &almagest_query::QueryResult) -> ApiResult<Vec<u8>> {
    let mut buf = Vec::new();
    let mut writer = StreamWriter::try_new(&mut buf, result.schema.as_ref())
        .map_err(|e| ApiError::internal(format!("arrow ipc init failed: {e}")))?;
    for batch in &result.batches {
        writer
            .write(batch)
            .map_err(|e| ApiError::internal(format!("arrow ipc write failed: {e}")))?;
    }
    writer
        .finish()
        .map_err(|e| ApiError::internal(format!("arrow ipc finish failed: {e}")))?;
    drop(writer);
    Ok(buf)
}

// --- parameter options -------------------------------------------------------

/// Body for `POST /api/almagest/options` — resolve one parameter's choices.
#[derive(Deserialize)]
pub struct OptionsRequest {
    /// The dashboard declaring the parameter.
    pub dashboard_id: String,
    /// The parameter whose options to resolve.
    pub parameter: String,
}

/// Resolved option values for a `select` / `multiselect` parameter.
#[derive(Serialize)]
pub struct OptionsResponse {
    /// The option values, in order.
    pub options: Vec<String>,
}

/// `POST /api/almagest/options` — static options, or distinct values from the
/// parameter's `options_query` (doc 07).
pub async fn resolve_options(
    State(state): State<AppState>,
    Json(req): Json<OptionsRequest>,
) -> ApiResult<Json<OptionsResponse>> {
    // Resolve the declaration under the lock; run any options_query after.
    let source = {
        let file = state.file();
        let dash = file.load_dashboard(&req.dashboard_id)?;
        let decl = dash
            .parameters
            .iter()
            .find(|p| p.id == req.parameter)
            .ok_or_else(|| {
                ApiError::not_found(format!(
                    "parameter '{}' not found in dashboard '{}'",
                    req.parameter, req.dashboard_id
                ))
            })?;
        if let Some(opts) = &decl.options {
            OptionSource::Static(opts.clone())
        } else if let Some(sql) = &decl.options_query {
            OptionSource::Query(sql.clone())
        } else {
            OptionSource::Static(Vec::new())
        }
    };

    let options = match source {
        OptionSource::Static(v) => v,
        OptionSource::Query(sql) => state.query.resolve_options(&sql).await?,
    };
    Ok(Json(OptionsResponse { options }))
}

enum OptionSource {
    Static(Vec<String>),
    Query(String),
}

// --- assets ------------------------------------------------------------------

/// An asset list entry.
#[derive(Serialize)]
pub struct AssetEntry {
    /// Logical asset path.
    pub path: String,
    /// MIME content type.
    pub content_type: String,
}

/// `GET /api/almagest/assets` — list embedded presentation assets.
pub async fn list_assets(State(state): State<AppState>) -> ApiResult<Json<Vec<AssetEntry>>> {
    let file = state.file();
    let out = file
        .list_assets()?
        .into_iter()
        .map(|(path, content_type)| AssetEntry { path, content_type })
        .collect();
    Ok(Json(out))
}

/// `GET /api/almagest/assets/*path` — serve an embedded asset's bytes.
pub async fn get_asset(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let asset = state.file().asset(&path)?;
    Ok((
        [
            (header::CONTENT_TYPE, asset.content_type),
            (header::CACHE_CONTROL, "public, max-age=3600".to_string()),
        ],
        asset.content,
    ))
}

// --- lifecycle ---------------------------------------------------------------

/// `POST /api/almagest/shutdown` — request a graceful shutdown (desktop mode,
/// fired by the frontend when the last tab closes).
pub async fn shutdown(State(state): State<AppState>) -> StatusCode {
    tracing::info!("shutdown requested via API");
    state.shutdown.notify_one();
    StatusCode::ACCEPTED
}

// --- helpers -----------------------------------------------------------------

/// Find a panel by id across all rows of a dashboard.
fn find_panel<'a>(dash: &'a Dashboard, panel_id: &str) -> Option<&'a Panel> {
    dash.layout
        .rows
        .iter()
        .flat_map(|r| &r.panels)
        .find(|p| p.id == panel_id)
}

/// Reduce a dashboard name to a safe download filename stem.
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "dashboard".to_string()
    } else {
        trimmed.to_lowercase()
    }
}

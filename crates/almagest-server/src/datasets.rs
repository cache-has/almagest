// SPDX-License-Identifier: MIT OR Apache-2.0

//! Author-time data ingest and dataset management.
//!
//! These endpoints unblock Studio's Data Manager: upload a CSV / Parquet / JSON
//! / SQLite file and bake it into the `.alm` as a compressed Parquet blob (via
//! the `almagest-connectors` importers), then list / rename / delete the
//! resulting datasets. A successful mutation rebuilds the in-memory query context
//! so the new tables are immediately queryable.
//!
//! Ingest is the one place the server writes bytes to a temp file — the
//! importers read from a local path. Nothing dials out; this is still strictly
//! embedded-only. A `dry_run` ingests into a throwaway file and reports the
//! inferred schema without touching the real one (the pre-commit preview).

use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, ServerEvent};
use almagest_connectors::{
    CsvImporter, CsvOptions, ImportedDataset, Importer, JsonFormat, JsonImporter, JsonOptions,
    ParquetImporter, ParquetOptions, SqliteImporter, SqliteOptions,
};
use almagest_core::{AlmagestFile, DatasetMeta};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path as FsPath;

// --- response shapes ---------------------------------------------------------

/// One column of a dataset's schema (mirrors `almagest_data.arrow_schema_json`).
#[derive(Serialize, Deserialize)]
pub struct ColumnInfo {
    /// Column name.
    pub name: String,
    /// Arrow data type, rendered for display.
    pub data_type: String,
    /// Whether the column is nullable.
    pub nullable: bool,
}

/// A dataset's metadata for the Data Manager list/detail views.
#[derive(Serialize)]
pub struct DatasetInfo {
    /// Table name queries reference.
    pub name: String,
    /// Row count.
    pub row_count: u64,
    /// Stored Parquet blob size in bytes.
    pub byte_size: u64,
    /// Compression codec.
    pub compression: String,
    /// Column schema.
    pub columns: Vec<ColumnInfo>,
    /// Import provenance, if any (parsed `source_json`).
    pub source: Option<serde_json::Value>,
}

impl From<DatasetMeta> for DatasetInfo {
    fn from(m: DatasetMeta) -> Self {
        DatasetInfo {
            name: m.name,
            row_count: m.row_count,
            byte_size: m.byte_size,
            compression: m.compression,
            columns: parse_columns(&m.arrow_schema_json),
            source: m.source_json.and_then(|s| serde_json::from_str(&s).ok()),
        }
    }
}

/// One dataset produced by an ingest, with the lenient-mode report.
#[derive(Serialize)]
pub struct IngestedDataset {
    /// Dataset name written.
    pub name: String,
    /// Rows written.
    pub row_count: u64,
    /// Stored blob size in bytes.
    pub byte_size: u64,
    /// Rows skipped in lenient mode.
    pub rows_skipped: u64,
    /// Non-fatal warnings from the import.
    pub warnings: Vec<String>,
    /// Resulting column schema.
    pub columns: Vec<ColumnInfo>,
}

impl From<ImportedDataset> for IngestedDataset {
    fn from(d: ImportedDataset) -> Self {
        IngestedDataset {
            name: d.meta.name,
            row_count: d.meta.row_count,
            byte_size: d.meta.byte_size,
            rows_skipped: d.report.rows_skipped,
            warnings: d.report.warnings,
            columns: parse_columns(&d.meta.arrow_schema_json),
        }
    }
}

/// The ingest result envelope.
#[derive(Serialize)]
pub struct IngestResult {
    /// Whether this was a preview (no write to the real file).
    pub dry_run: bool,
    /// The datasets produced (or that would be produced).
    pub datasets: Vec<IngestedDataset>,
}

// --- ingest ------------------------------------------------------------------

/// Query parameters for `POST /api/almagest/datasets`.
#[derive(Deserialize)]
pub struct IngestQuery {
    /// Source format (`csv` / `parquet` / `json` / `sqlite`). Inferred from
    /// `filename`'s extension when omitted.
    format: Option<String>,
    /// Original filename — used to infer the format and default dataset name.
    filename: Option<String>,
    /// Target dataset name (CSV/Parquet/JSON) or table-name prefix (SQLite).
    name: Option<String>,
    /// Overwrite an existing dataset of the same name.
    #[serde(default)]
    replace: bool,
    /// Infer the schema and report it without writing to the file.
    #[serde(default)]
    dry_run: bool,
    /// CSV delimiter (first byte used). Defaults to `,`.
    delimiter: Option<String>,
    /// CSV: treat the first row as data, not a header.
    #[serde(default)]
    no_header: bool,
    /// JSON layout (`array` or `ndjson`). Auto-detected from the bytes otherwise.
    json_format: Option<String>,
}

/// `POST /api/almagest/datasets` — ingest an uploaded file as a dataset.
pub async fn ingest(
    State(state): State<AppState>,
    Query(q): Query<IngestQuery>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<IngestResult>)> {
    if body.is_empty() {
        return Err(ApiError::bad_request("upload body is empty"));
    }
    let format = resolve_format(&q)?;

    // The importers read from a local path; stage the upload in a temp file.
    let mut tmp = tempfile::Builder::new()
        .prefix("almagest-ingest-")
        .tempfile()
        .map_err(|e| ApiError::internal(format!("staging upload failed: {e}")))?;
    tmp.write_all(&body)
        .and_then(|_| tmp.flush())
        .map_err(|e| ApiError::internal(format!("staging upload failed: {e}")))?;
    let src = tmp.path().to_path_buf();

    if q.dry_run {
        // Ingest into a throwaway file so we can report the inferred schema
        // without mutating the real one.
        let dir = tempfile::tempdir()
            .map_err(|e| ApiError::internal(format!("preview workspace failed: {e}")))?;
        let mut preview = AlmagestFile::create(dir.path().join("preview.alm"))?;
        let imported = run_import(&mut preview, format, &src, &q, &body)?;
        return Ok((
            StatusCode::OK,
            Json(IngestResult {
                dry_run: true,
                datasets: imported.into_iter().map(Into::into).collect(),
            }),
        ));
    }

    let imported = {
        let mut file = state.file();
        run_import(&mut file, format, &src, &q, &body)?
    };

    // Re-register the (now-changed) data so new tables are queryable immediately.
    rebuild_query(&state)?;
    state.emit(ServerEvent::DataChanged);

    Ok((
        StatusCode::CREATED,
        Json(IngestResult {
            dry_run: false,
            datasets: imported.into_iter().map(Into::into).collect(),
        }),
    ))
}

// --- management --------------------------------------------------------------

/// `GET /api/almagest/datasets` — list embedded datasets.
pub async fn list_datasets(State(state): State<AppState>) -> ApiResult<Json<Vec<DatasetInfo>>> {
    let file = state.file();
    let out = file.list_datasets()?.into_iter().map(Into::into).collect();
    Ok(Json(out))
}

/// `GET /api/almagest/datasets/:name` — one dataset's schema and metadata.
pub async fn get_dataset(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Json<DatasetInfo>> {
    let meta = state.file().dataset_meta(&name)?;
    Ok(Json(meta.into()))
}

/// `DELETE /api/almagest/datasets/:name` — remove a dataset.
pub async fn delete_dataset(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    let removed = {
        let mut file = state.file();
        file.remove_dataset(&name)?
    };
    if !removed {
        return Err(ApiError::not_found(format!("dataset '{name}' not found")));
    }
    rebuild_query(&state)?;
    state.emit(ServerEvent::DataChanged);
    Ok(StatusCode::NO_CONTENT)
}

/// Body for `POST /api/almagest/datasets/:name/rename`.
#[derive(Deserialize)]
pub struct RenameRequest {
    /// The new dataset name.
    pub to: String,
}

/// `POST /api/almagest/datasets/:name/rename` — rename a dataset.
pub async fn rename_dataset(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<RenameRequest>,
) -> ApiResult<StatusCode> {
    {
        let mut file = state.file();
        file.rename_dataset(&name, &req.to)?;
    }
    rebuild_query(&state)?;
    state.emit(ServerEvent::DataChanged);
    Ok(StatusCode::NO_CONTENT)
}

// --- helpers -----------------------------------------------------------------

fn rebuild_query(state: &AppState) -> ApiResult<()> {
    let file = state.file();
    state
        .rebuild_query(&file)
        .map_err(|e| ApiError::internal(format!("rebuilding query engine failed: {e}")))
}

fn resolve_format(q: &IngestQuery) -> ApiResult<&'static str> {
    if let Some(f) = &q.format {
        return normalize_format(f);
    }
    if let Some(name) = &q.filename
        && let Some(ext) = FsPath::new(name).extension().and_then(|e| e.to_str())
    {
        return normalize_format(ext);
    }
    Err(ApiError::bad_request(
        "specify ?format=csv|parquet|json|sqlite or ?filename= with a known extension",
    ))
}

fn normalize_format(s: &str) -> ApiResult<&'static str> {
    match s.to_ascii_lowercase().as_str() {
        "csv" => Ok("csv"),
        "parquet" | "pq" => Ok("parquet"),
        "json" | "ndjson" | "jsonl" => Ok("json"),
        "sqlite" | "sqlite3" | "db" => Ok("sqlite"),
        other => Err(ApiError::bad_request(format!(
            "unsupported format '{other}'"
        ))),
    }
}

fn run_import(
    file: &mut AlmagestFile,
    format: &str,
    src: &FsPath,
    q: &IngestQuery,
    body: &[u8],
) -> ApiResult<Vec<ImportedDataset>> {
    // The importers default a missing name from the *source path* stem, which is
    // our random temp file — so derive it from the explicit name or the original
    // filename instead.
    let name = q.name.clone().or_else(|| {
        q.filename
            .as_ref()
            .and_then(|f| {
                FsPath::new(f)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
            })
            .filter(|s| !s.is_empty())
    });

    let datasets = match format {
        "csv" => {
            let mut o = CsvOptions::new(src);
            o.name = name;
            o.replace = q.replace;
            o.has_header = !q.no_header;
            if let Some(d) = q.delimiter.as_ref().and_then(|d| d.bytes().next()) {
                o.delimiter = d;
            }
            CsvImporter::new(o).import(file)?
        }
        "parquet" => {
            let mut o = ParquetOptions::new(src);
            o.name = name;
            o.replace = q.replace;
            ParquetImporter::new(o).import(file)?
        }
        "json" => {
            let mut o = JsonOptions::new(src);
            o.name = name;
            o.replace = q.replace;
            o.format = detect_json_format(q, body);
            JsonImporter::new(o).import(file)?
        }
        "sqlite" => {
            let mut o = SqliteOptions::new(src);
            o.name_prefix = q.name.clone().unwrap_or_default();
            o.replace = q.replace;
            SqliteImporter::new(o).import(file)?
        }
        _ => unreachable!("format already validated"),
    };
    Ok(datasets)
}

fn detect_json_format(q: &IngestQuery, body: &[u8]) -> JsonFormat {
    match q
        .json_format
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("array") => JsonFormat::Array,
        Some("ndjson") | Some("jsonl") => JsonFormat::Ndjson,
        _ => {
            // Auto-detect: a leading '[' (after whitespace) means a JSON array.
            match body.iter().find(|b| !b.is_ascii_whitespace()) {
                Some(b'[') => JsonFormat::Array,
                _ => JsonFormat::Ndjson,
            }
        }
    }
}

fn parse_columns(arrow_schema_json: &str) -> Vec<ColumnInfo> {
    #[derive(Deserialize)]
    struct Doc {
        fields: Vec<ColumnInfo>,
    }
    serde_json::from_str::<Doc>(arrow_schema_json)
        .map(|d| d.fields)
        .unwrap_or_default()
}

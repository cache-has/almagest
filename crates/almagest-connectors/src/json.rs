// SPDX-License-Identifier: MIT OR Apache-2.0

//! JSON / NDJSON importer.
//!
//! Reads newline-delimited records, a top-level array, or an array reached by a
//! simple record path. Records are normalized to NDJSON, then Arrow infers the
//! schema (heterogeneous records are widened by Arrow's inference) and reads
//! them. Nested objects are kept as Arrow struct columns (flattening to dotted
//! columns is a deferred option). In lenient mode, lines that don't parse as a
//! JSON object are skipped and counted; in strict mode the first bad record
//! aborts the import.

use crate::error::{ImportError, ImportReport, Result};
use crate::framework::{
    ImportedDataset, Importer, check_collision, default_name, ensure_source_exists, provenance,
};
use almagest_core::{AlmagestFile, Compression};
use arrow::datatypes::SchemaRef;
use arrow::json::ReaderBuilder;
use arrow::json::reader::infer_json_schema_from_seekable;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

/// How the source JSON is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonFormat {
    /// One JSON record per line.
    Ndjson,
    /// A single top-level (or record-path-reached) JSON array of records.
    Array,
}

/// How to handle records that don't parse as a JSON object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonMode {
    /// Abort on the first bad record.
    Strict,
    /// Skip and count bad records, importing the rest.
    Lenient,
}

/// Options for importing a JSON / NDJSON file.
#[derive(Debug, Clone)]
pub struct JsonOptions {
    /// Absolute path to the JSON file.
    pub path: PathBuf,
    /// Target dataset name. Defaults to the file stem.
    pub name: Option<String>,
    /// Source layout.
    pub format: JsonFormat,
    /// Optional dotted path to the records array (e.g. `$.data`). Only used for
    /// [`JsonFormat::Array`].
    pub record_path: Option<String>,
    /// Rows to sample for schema inference (`None` = all).
    pub infer_schema_rows: Option<usize>,
    /// Strict vs. lenient bad-record handling.
    pub mode: JsonMode,
    /// Overwrite an existing dataset of the same name instead of erroring.
    pub replace: bool,
}

impl JsonOptions {
    /// Default NDJSON, lenient options for `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            name: None,
            format: JsonFormat::Ndjson,
            record_path: None,
            infer_schema_rows: Some(1000),
            mode: JsonMode::Lenient,
            replace: false,
        }
    }
}

/// JSON importer.
pub struct JsonImporter {
    opts: JsonOptions,
}

impl JsonImporter {
    /// Build the importer from options.
    pub fn new(opts: JsonOptions) -> Self {
        Self { opts }
    }
}

impl Importer for JsonImporter {
    fn import(&self, file: &mut AlmagestFile) -> Result<Vec<ImportedDataset>> {
        let path = &self.opts.path;
        ensure_source_exists(path)?;
        let name = self.opts.name.clone().unwrap_or_else(|| default_name(path));
        check_collision(file, &name, self.opts.replace)?;

        let raw = std::fs::read_to_string(path).map_err(|e| ImportError::SourceUnreadable {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;

        // Collect candidate record values according to the layout.
        let candidates = self.candidate_records(&raw)?;

        // Normalize to NDJSON, applying strict/lenient policy to non-objects.
        let mut ndjson = String::new();
        let mut skipped: u64 = 0;
        let mut warnings = Vec::new();
        for (row, value) in candidates.into_iter().enumerate() {
            if value.is_object() {
                ndjson.push_str(&value.to_string());
                ndjson.push('\n');
            } else {
                match self.opts.mode {
                    JsonMode::Strict => {
                        return Err(ImportError::MalformedRecord {
                            row: row as u64,
                            detail: "record is not a JSON object".to_string(),
                        });
                    }
                    JsonMode::Lenient => skipped += 1,
                }
            }
        }
        if skipped > 0 {
            warnings.push(format!("skipped {skipped} non-object record(s)"));
        }

        let bytes = ndjson.into_bytes();
        let (schema, _) =
            infer_json_schema_from_seekable(Cursor::new(&bytes), self.opts.infer_schema_rows)
                .map_err(|e| ImportError::SchemaInferenceFailed {
                    detail: e.to_string(),
                })?;
        let schema: SchemaRef = Arc::new(schema);

        let reader = ReaderBuilder::new(schema.clone())
            .build(Cursor::new(bytes))
            .map_err(|e| ImportError::SchemaInferenceFailed {
                detail: e.to_string(),
            })?;

        let source_json = provenance(
            "json",
            path,
            serde_json::json!({
                "format": match self.opts.format {
                    JsonFormat::Ndjson => "ndjson",
                    JsonFormat::Array => "array",
                },
                "record_path": self.opts.record_path,
                "mode": match self.opts.mode {
                    JsonMode::Strict => "strict",
                    JsonMode::Lenient => "lenient",
                },
            }),
        );

        let meta = file.put_dataset_streaming(
            &name,
            schema,
            reader,
            Compression::Zstd,
            Some(&source_json),
        )?;

        let report = ImportReport {
            rows_imported: meta.row_count,
            rows_skipped: skipped,
            warnings,
        };
        Ok(vec![ImportedDataset { name, meta, report }])
    }
}

impl JsonImporter {
    /// Extract the list of candidate record values from the raw source text.
    fn candidate_records(&self, raw: &str) -> Result<Vec<serde_json::Value>> {
        match self.opts.format {
            JsonFormat::Ndjson => raw
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|line| {
                    serde_json::from_str::<serde_json::Value>(line).map_err(|e| {
                        ImportError::SchemaInferenceFailed {
                            detail: format!("invalid JSON line: {e}"),
                        }
                    })
                })
                .collect(),
            JsonFormat::Array => {
                let root: serde_json::Value =
                    serde_json::from_str(raw).map_err(|e| ImportError::UnsupportedFormat {
                        detail: format!("not valid JSON: {e}"),
                    })?;
                let array = navigate(&root, self.opts.record_path.as_deref())?;
                match array {
                    serde_json::Value::Array(items) => Ok(items.clone()),
                    _ => Err(ImportError::UnsupportedFormat {
                        detail: "record path did not resolve to a JSON array".to_string(),
                    }),
                }
            }
        }
    }
}

/// Follow a simple dotted record path (e.g. `$.data.rows`) into `value`. A
/// `None` or `"$"` path returns the root. Bracketed indices are not supported.
fn navigate<'a>(value: &'a serde_json::Value, path: Option<&str>) -> Result<&'a serde_json::Value> {
    let path = match path {
        None => return Ok(value),
        Some(p) => p.trim_start_matches('$').trim_start_matches('.'),
    };
    if path.is_empty() {
        return Ok(value);
    }
    let mut cur = value;
    for segment in path.split('.') {
        cur = cur
            .get(segment)
            .ok_or_else(|| ImportError::SchemaInferenceFailed {
                detail: format!("record path segment '{segment}' not found"),
            })?;
    }
    Ok(cur)
}

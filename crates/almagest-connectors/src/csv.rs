// SPDX-License-Identifier: MIT OR Apache-2.0

//! CSV importer — delimited text with header detection, sampled type inference,
//! and per-column type overrides.

use crate::error::{ImportError, ImportReport, Result};
use crate::framework::{
    ImportedDataset, Importer, check_collision, default_name, ensure_source_exists, provenance,
};
use almagest_core::{AlmagestFile, Compression};
use arrow::csv::reader::{Format, ReaderBuilder};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

/// Options for importing a CSV file.
#[derive(Debug, Clone)]
pub struct CsvOptions {
    /// Absolute path to the CSV file.
    pub path: PathBuf,
    /// Target dataset name. Defaults to the file stem.
    pub name: Option<String>,
    /// Field delimiter (default `,`).
    pub delimiter: u8,
    /// Whether the first row is a header (default true).
    pub has_header: bool,
    /// Rows to sample for type inference (default 1000; `None` = whole file).
    pub infer_schema_rows: Option<usize>,
    /// Per-column type overrides, e.g. `{"amount": "float64"}`.
    pub column_overrides: HashMap<String, String>,
    /// Overwrite an existing dataset of the same name instead of erroring.
    pub replace: bool,
}

impl CsvOptions {
    /// Default options for `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            name: None,
            delimiter: b',',
            has_header: true,
            infer_schema_rows: Some(1000),
            column_overrides: HashMap::new(),
            replace: false,
        }
    }
}

/// CSV importer.
pub struct CsvImporter {
    opts: CsvOptions,
}

impl CsvImporter {
    /// Build the importer from options.
    pub fn new(opts: CsvOptions) -> Self {
        Self { opts }
    }
}

impl Importer for CsvImporter {
    fn import(&self, file: &mut AlmagestFile) -> Result<Vec<ImportedDataset>> {
        let path = &self.opts.path;
        ensure_source_exists(path)?;
        let name = self.opts.name.clone().unwrap_or_else(|| default_name(path));
        check_collision(file, &name, self.opts.replace)?;

        let format = Format::default()
            .with_header(self.opts.has_header)
            .with_delimiter(self.opts.delimiter);

        // Infer the schema from a sample, then apply any overrides.
        let infer_handle = open(path)?;
        let (inferred, _) = format
            .infer_schema(&infer_handle, self.opts.infer_schema_rows)
            .map_err(|e| ImportError::SchemaInferenceFailed {
                detail: e.to_string(),
            })?;
        let schema = apply_overrides(inferred, &self.opts.column_overrides)?;
        let schema: SchemaRef = Arc::new(schema);

        // Stream the file through the Parquet writer (bounded memory).
        let read_handle = open(path)?;
        let reader = ReaderBuilder::new(schema.clone())
            .with_format(format)
            .build(read_handle)
            .map_err(|e| ImportError::SchemaInferenceFailed {
                detail: e.to_string(),
            })?;

        let source_json = provenance(
            "csv",
            path,
            serde_json::json!({
                "delimiter": self.opts.delimiter as char,
                "has_header": self.opts.has_header,
                "infer_schema_rows": self.opts.infer_schema_rows,
                "column_overrides": self.opts.column_overrides,
            }),
        );

        let meta = file.put_dataset_streaming(
            &name,
            schema,
            reader,
            Compression::Zstd,
            Some(&source_json),
        )?;

        let mut report = ImportReport::clean(meta.row_count);
        if meta.row_count == 0 {
            report.warn("CSV produced zero data rows");
        }
        Ok(vec![ImportedDataset { name, meta, report }])
    }
}

/// Open a CSV source, mapping I/O failure to a clear unreadable error.
fn open(path: &std::path::Path) -> Result<File> {
    File::open(path).map_err(|e| ImportError::SourceUnreadable {
        path: path.display().to_string(),
        detail: e.to_string(),
    })
}

/// Replace the data type of any field named in `overrides`. An override naming a
/// column that isn't present is an error (likely a typo).
fn apply_overrides(schema: Schema, overrides: &HashMap<String, String>) -> Result<Schema> {
    if overrides.is_empty() {
        return Ok(schema);
    }
    for col in overrides.keys() {
        if schema.field_with_name(col).is_err() {
            return Err(ImportError::SchemaInferenceFailed {
                detail: format!("column override names unknown column '{col}'"),
            });
        }
    }
    let fields: Vec<Field> = schema
        .fields()
        .iter()
        .map(|f| match overrides.get(f.name()) {
            Some(type_str) => {
                parse_type(type_str).map(|dt| Field::new(f.name(), dt, f.is_nullable()))
            }
            None => Ok(f.as_ref().clone()),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Schema::new(fields))
}

/// Map a human type name to an Arrow [`DataType`].
fn parse_type(s: &str) -> Result<DataType> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "int" | "int64" | "integer" | "bigint" => DataType::Int64,
        "int32" => DataType::Int32,
        "float" | "float64" | "double" => DataType::Float64,
        "float32" => DataType::Float32,
        "bool" | "boolean" => DataType::Boolean,
        "string" | "utf8" | "text" => DataType::Utf8,
        "date" => DataType::Date32,
        "timestamp" | "datetime" => DataType::Timestamp(TimeUnit::Microsecond, None),
        other => {
            return Err(ImportError::SchemaInferenceFailed {
                detail: format!("unknown column override type '{other}'"),
            });
        }
    })
}

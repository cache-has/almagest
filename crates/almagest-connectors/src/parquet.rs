// SPDX-License-Identifier: MIT OR Apache-2.0

//! Parquet importer — the cheapest import, since the source is already
//! columnar. Re-encodes into the file's chosen compression codec, with optional
//! column projection.
//!
//! Copy-as-is (store the original blob verbatim, skipping re-encode) is a
//! deferred optimization; v1 always recompresses so the `.alm` has uniform
//! codecs and row-group sizing.

use crate::error::{ImportError, ImportReport, Result};
use crate::framework::{
    ImportedDataset, Importer, check_collision, default_name, ensure_source_exists, provenance,
};
use almagest_core::{AlmagestFile, Compression};
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::fs::File;
use std::path::PathBuf;

/// Options for importing a Parquet file.
#[derive(Debug, Clone)]
pub struct ParquetOptions {
    /// Absolute path to the Parquet file.
    pub path: PathBuf,
    /// Target dataset name. Defaults to the file stem.
    pub name: Option<String>,
    /// Optional column projection; `None` imports all columns.
    pub columns: Option<Vec<String>>,
    /// Overwrite an existing dataset of the same name instead of erroring.
    pub replace: bool,
}

impl ParquetOptions {
    /// Default options for `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            name: None,
            columns: None,
            replace: false,
        }
    }
}

/// Parquet importer.
pub struct ParquetImporter {
    opts: ParquetOptions,
}

impl ParquetImporter {
    /// Build the importer from options.
    pub fn new(opts: ParquetOptions) -> Self {
        Self { opts }
    }
}

impl Importer for ParquetImporter {
    fn import(&self, file: &mut AlmagestFile) -> Result<Vec<ImportedDataset>> {
        let path = &self.opts.path;
        ensure_source_exists(path)?;
        let name = self.opts.name.clone().unwrap_or_else(|| default_name(path));
        check_collision(file, &name, self.opts.replace)?;

        let handle = File::open(path).map_err(|e| ImportError::SourceUnreadable {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
        let mut builder = ParquetRecordBatchReaderBuilder::try_new(handle).map_err(|e| {
            ImportError::UnsupportedFormat {
                detail: format!("not a readable Parquet file: {e}"),
            }
        })?;

        // Optional projection by leaf column name.
        if let Some(cols) = &self.opts.columns {
            let parquet_schema = builder.parquet_schema();
            // Validate every requested column exists.
            let known: Vec<String> = (0..parquet_schema.num_columns())
                .map(|i| parquet_schema.column(i).name().to_string())
                .collect();
            for c in cols {
                if !known.contains(c) {
                    return Err(ImportError::SchemaInferenceFailed {
                        detail: format!("projected column '{c}' is not in the Parquet file"),
                    });
                }
            }
            let mask = ProjectionMask::columns(parquet_schema, cols.iter().map(|s| s.as_str()));
            builder = builder.with_projection(mask);
        }

        let reader = builder.build().map_err(|e| {
            ImportError::Arrow(arrow::error::ArrowError::ExternalError(Box::new(e)))
        })?;
        // The reader (a RecordBatchReader) reports the post-projection schema.
        let schema = arrow::array::RecordBatchReader::schema(&reader);

        let source_json = provenance(
            "parquet",
            path,
            serde_json::json!({
                "columns": self.opts.columns,
                "recompress": true,
            }),
        );

        let meta = file.put_dataset_streaming(
            &name,
            schema,
            reader,
            Compression::Zstd,
            Some(&source_json),
        )?;

        let report = ImportReport::clean(meta.row_count);
        Ok(vec![ImportedDataset { name, meta, report }])
    }
}

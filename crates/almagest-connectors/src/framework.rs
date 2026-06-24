// SPDX-License-Identifier: MIT OR Apache-2.0

//! The shared importer framework: the [`Importer`] trait every source
//! implements, plus helpers all importers reuse (existence checks, name-
//! collision policy, provenance records).

use crate::error::{ImportError, ImportReport, Result};
use almagest_core::AlmagestFile;
use std::path::Path;

/// One dataset produced by an import: its target name, the stored metadata, and
/// the lenient-mode report.
#[derive(Debug, Clone)]
pub struct ImportedDataset {
    /// The `almagest_data` name the dataset was written under.
    pub name: String,
    /// Stored dataset metadata (row count, byte size, …).
    pub meta: almagest_core::DatasetMeta,
    /// What the import did (rows imported/skipped, warnings).
    pub report: ImportReport,
}

/// An author-time importer: reads a local source and bakes one or more datasets
/// into the `.alm`. Importers never open a network connection or resolve a
/// credential — every source is a local file (the locked embedded-only rule).
pub trait Importer {
    /// Run the import, writing datasets into `file` and returning what landed.
    fn import(&self, file: &mut AlmagestFile) -> Result<Vec<ImportedDataset>>;
}

/// Fail with [`ImportError::SourceNotFound`] if `path` is missing.
pub(crate) fn ensure_source_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(ImportError::SourceNotFound {
            path: path.display().to_string(),
        });
    }
    Ok(())
}

/// Enforce the name-collision policy before writing `name`. With `replace`
/// false, an existing dataset of the same name is an error rather than a silent
/// overwrite.
pub(crate) fn check_collision(file: &AlmagestFile, name: &str, replace: bool) -> Result<()> {
    if !replace && file.dataset_meta(name).is_ok() {
        return Err(ImportError::NameCollision {
            name: name.to_string(),
        });
    }
    Ok(())
}

/// Build a `source_json` provenance record for `almagest_data.source_json`.
pub(crate) fn provenance(kind: &str, path: &Path, options: serde_json::Value) -> String {
    let record = serde_json::json!({
        "kind": kind,
        "path": path.display().to_string(),
        "options": options,
        "imported_at": chrono::Utc::now().to_rfc3339(),
    });
    record.to_string()
}

/// Default dataset name derived from a source path's file stem (e.g.
/// `/data/orders.csv` → `orders`). Falls back to `dataset` if there's no stem.
pub(crate) fn default_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("dataset")
        .to_string()
}

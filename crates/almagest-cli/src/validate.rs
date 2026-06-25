// SPDX-License-Identifier: MIT OR Apache-2.0

//! `almagest validate` — integrity + schema validation, suitable for CI.
//!
//! Exit code 0 when valid, non-zero otherwise. `--strict` additionally re-parses
//! every dashboard, decodes every dataset blob, and builds the query engine — a
//! deeper check that the file is fully usable, not merely structurally intact.

use crate::output::Out;
use almagest_core::AlmagestFile;
use almagest_query::AlmagestQueryContext;
use anyhow::Result;
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
struct ValidateReport {
    path: String,
    valid: bool,
    strict: bool,
    errors: Vec<String>,
}

/// Run `almagest validate`. Returns `Ok(false)` when the file is invalid so the
/// caller can set a non-zero exit code without printing a Rust error.
pub fn run(path: &Path, strict: bool, out: &Out) -> Result<bool> {
    if !path.exists() {
        anyhow::bail!("{} does not exist", path.display());
    }

    let mut errors: Vec<String> = Vec::new();

    // Opening runs the on-open integrity check (PRAGMA integrity_check + format
    // version bounds). A failure here means the file isn't structurally sound.
    let file = match AlmagestFile::open(path) {
        Ok(f) => Some(f),
        Err(e) => {
            errors.push(format!("integrity: {e}"));
            None
        }
    };

    if let Some(file) = &file {
        // Every dashboard must parse + validate against the DSL.
        match file.list_dashboards() {
            Ok(records) => {
                for rec in records {
                    if let Err(e) = file.load_dashboard(&rec.id) {
                        errors.push(format!("dashboard '{}': {e}", rec.name));
                    }
                }
            }
            Err(e) => errors.push(format!("listing dashboards: {e}")),
        }

        if strict {
            // Each dataset blob must decode back to Arrow…
            match file.list_datasets() {
                Ok(datasets) => {
                    for d in datasets {
                        if let Err(e) = file.read_dataset(&d.name) {
                            errors.push(format!("dataset '{}': {e}", d.name));
                        }
                    }
                }
                Err(e) => errors.push(format!("listing datasets: {e}")),
            }
            // …and the query engine must build over the file's data.
            if let Err(e) = AlmagestQueryContext::open(file) {
                errors.push(format!("query engine: {e}"));
            }
        }
    }

    let valid = errors.is_empty();
    let report = ValidateReport {
        path: path.display().to_string(),
        valid,
        strict,
        errors,
    };

    if out.json {
        out.emit(&report)?;
    } else if valid {
        out.result(format!("{} is valid.", report.path));
    } else {
        // Errors go to stderr so a piped success/failure stays clean.
        eprintln!("{} is INVALID:", report.path);
        for e in &report.errors {
            eprintln!("  ✗ {e}");
        }
    }
    Ok(valid)
}

// SPDX-License-Identifier: MIT OR Apache-2.0

//! `almagest import` — import a standalone dashboard JSON into a `.alm` file.
//!
//! Reuses the core import path (parse + validate + check that referenced saved
//! queries exist). Data import (CSV/Parquet/…) is an author-time Studio action,
//! not a CLI command — the CLI imports *dashboards*, not datasets.

use crate::output::Out;
use almagest_core::AlmagestFile;
use anyhow::{Context, Result, bail};
use std::path::Path;

/// Run `almagest import <file.alm> <dashboard.json>`.
pub fn run(path: &Path, dashboard_json: &Path, folder: Option<&str>, out: &Out) -> Result<()> {
    if !path.exists() {
        bail!("{} does not exist", path.display());
    }
    if !dashboard_json.exists() {
        bail!("{} does not exist", dashboard_json.display());
    }
    let mut file =
        AlmagestFile::open(path).with_context(|| format!("opening {}", path.display()))?;
    let id = file
        .import_dashboard_json(dashboard_json, folder)
        .with_context(|| format!("importing {}", dashboard_json.display()))?;
    file.close().context("finalizing the file")?;

    out.result(format!("Imported dashboard as {id}"));
    out.emit(&serde_json::json!({ "id": id }))?;
    Ok(())
}

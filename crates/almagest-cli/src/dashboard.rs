// SPDX-License-Identifier: MIT OR Apache-2.0

//! `almagest dashboard list | show | delete` — scripted dashboard management.
//!
//! Authoring happens in the Studio UI; the CLI is for listing, inspection, and
//! batch deletion. `show` / `delete` accept either a dashboard id or its name
//! (names resolve case-sensitively; ambiguous names error).

use crate::output::Out;
use almagest_core::AlmagestFile;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
struct DashListEntry {
    id: String,
    name: String,
    description: Option<String>,
    folder: Option<String>,
    panel_count: usize,
}

/// `almagest dashboard list`.
pub fn list(path: &Path, out: &Out) -> Result<()> {
    let file = open(path)?;
    let mut entries = Vec::new();
    for rec in file.list_dashboards()? {
        let panel_count = file
            .load_dashboard(&rec.id)
            .map(|d| d.layout.rows.iter().map(|r| r.panels.len()).sum())
            .unwrap_or(0);
        entries.push(DashListEntry {
            id: rec.id,
            name: rec.name,
            description: rec.description,
            folder: rec.folder,
            panel_count,
        });
    }

    if out.json {
        return out.emit(&entries);
    }
    if entries.is_empty() {
        out.line("No dashboards.");
        return Ok(());
    }
    for e in &entries {
        println!("{}  {}  ({} panels)", e.id, e.name, e.panel_count);
    }
    Ok(())
}

/// `almagest dashboard show <id-or-name>` — print the dashboard definition JSON.
pub fn show(path: &Path, selector: &str, out: &Out) -> Result<()> {
    let file = open(path)?;
    let id = resolve(&file, selector)?;
    let dash = file.load_dashboard(&id)?;
    // The definition JSON is the natural output for both modes (it's already the
    // git-diffable representation); `--json` just guarantees no extra chatter.
    let json = dash.to_json_pretty().context("serializing dashboard")?;
    if out.json {
        println!("{json}");
    } else {
        out.result(json);
    }
    Ok(())
}

/// `almagest dashboard delete <id-or-name>`.
pub fn delete(path: &Path, selector: &str, yes: bool, out: &Out) -> Result<()> {
    let mut file = open(path)?;
    let id = resolve(&file, selector)?;
    let rec = file.dashboard(&id)?;
    if !yes && !out.json {
        crate::confirm(&format!("Delete dashboard \"{}\"?", rec.name))?;
    }
    if !file.remove_dashboard(&id)? {
        bail!("dashboard '{id}' not found");
    }
    out.result(format!("Deleted dashboard \"{}\".", rec.name));
    out.emit(&serde_json::json!({ "deleted": id }))?;
    Ok(())
}

fn open(path: &Path) -> Result<AlmagestFile> {
    if !path.exists() {
        bail!("{} does not exist", path.display());
    }
    AlmagestFile::open(path).with_context(|| format!("opening {}", path.display()))
}

/// Resolve a dashboard selector to an id: exact id match first, then a unique
/// name match.
fn resolve(file: &AlmagestFile, selector: &str) -> Result<String> {
    let records = file.list_dashboards()?;
    if let Some(r) = records.iter().find(|r| r.id == selector) {
        return Ok(r.id.clone());
    }
    let by_name: Vec<&_> = records.iter().filter(|r| r.name == selector).collect();
    match by_name.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => bail!("no dashboard with id or name '{selector}'"),
        many => bail!(
            "'{selector}' matches {} dashboards by name; use the id instead",
            many.len()
        ),
    }
}

// SPDX-License-Identifier: MIT OR Apache-2.0

//! `almagest info` — a quick health summary of a `.alm` file.

use crate::output::Out;
use almagest_core::AlmagestFile;
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
struct TableInfo {
    name: String,
    row_count: u64,
    byte_size: u64,
}

#[derive(Serialize)]
struct DashInfo {
    id: String,
    name: String,
    panel_count: usize,
}

#[derive(Serialize)]
struct FileInfo {
    path: String,
    format_version: u32,
    created_at: Option<String>,
    created_by: Option<String>,
    title: Option<String>,
    size_bytes: u64,
    tables: Vec<TableInfo>,
    dashboards: Vec<DashInfo>,
    auth_enabled: bool,
    user_count: u64,
    cache_entries: u64,
    cache_bytes: u64,
}

/// Run `almagest info`.
pub fn run(path: &Path, out: &Out) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("{} does not exist", path.display());
    }
    let file = AlmagestFile::open(path).with_context(|| format!("opening {}", path.display()))?;

    let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let tables: Vec<TableInfo> = file
        .list_datasets()?
        .into_iter()
        .map(|d| TableInfo {
            name: d.name,
            row_count: d.row_count,
            byte_size: d.byte_size,
        })
        .collect();

    let mut dashboards = Vec::new();
    for rec in file.list_dashboards()? {
        // Count panels via the typed model; fall back to 0 if a definition is
        // somehow unparseable (info should never hard-fail on one bad dashboard).
        let panel_count = file
            .load_dashboard(&rec.id)
            .map(|d| d.layout.rows.iter().map(|r| r.panels.len()).sum())
            .unwrap_or(0);
        dashboards.push(DashInfo {
            id: rec.id,
            name: rec.name,
            panel_count,
        });
    }

    let auth_enabled = file.auth_enabled()?;
    let user_count = file.count_users()?;
    let (cache_entries, cache_bytes) = file.cache_stats()?;

    let title = file.title().ok().filter(|s| !s.is_empty());
    let info = FileInfo {
        path: path.display().to_string(),
        format_version: file.format_version()?,
        created_at: file.metadata("created_at")?,
        created_by: file.metadata("created_by_version")?,
        title,
        size_bytes,
        tables,
        dashboards,
        auth_enabled,
        user_count,
        cache_entries,
        cache_bytes,
    };

    if out.json {
        return out.emit(&info);
    }

    println!("File: {}", info.path);
    println!("Almagest format version: {}", info.format_version);
    if let Some(c) = &info.created_at {
        println!("Created: {c}");
    }
    if let Some(c) = &info.created_by {
        println!("Created by: almagest {c}");
    }
    if let Some(t) = &info.title {
        println!("Title: {t}");
    }
    println!("Size: {}", human_bytes(info.size_bytes));
    println!();

    println!(
        "Embedded data: {} table{}",
        info.tables.len(),
        plural(info.tables.len())
    );
    for t in &info.tables {
        println!(
            "  - {:<20} ({} rows, {})",
            t.name,
            t.row_count,
            human_bytes(t.byte_size)
        );
    }
    println!();

    println!("Dashboards: {}", info.dashboards.len());
    for d in &info.dashboards {
        println!(
            "  - {} ({} panel{})",
            d.name,
            d.panel_count,
            plural(d.panel_count)
        );
    }
    println!();

    if info.auth_enabled {
        println!(
            "Users: enabled ({} account{})",
            info.user_count,
            plural(info.user_count as usize)
        );
    } else {
        println!("Users: disabled (single-user mode)");
    }
    println!(
        "Cache: {} ({} entr{})",
        human_bytes(info.cache_bytes),
        info.cache_entries,
        if info.cache_entries == 1 { "y" } else { "ies" }
    );
    Ok(())
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

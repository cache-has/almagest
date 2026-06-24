// SPDX-License-Identifier: MIT OR Apache-2.0

//! `almagest export` — bake a self-contained static HTML snapshot (Phase 11).
//!
//! Thin CLI wrapper: parse options, delegate to
//! [`almagest_server::export_snapshot_html`], and write the file.

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Run the export command.
pub async fn run(
    path: &Path,
    output: Option<&Path>,
    dashboard: Option<&str>,
    format: &str,
    parameters: Option<&str>,
) -> Result<()> {
    if !path.exists() {
        bail!("{} does not exist", path.display());
    }
    if !format.eq_ignore_ascii_case("html") {
        bail!("unsupported --format '{format}'; only `html` is available today (pdf is planned)");
    }

    let raw_params: BTreeMap<String, serde_json::Value> = match parameters {
        Some(s) => serde_json::from_str(s)
            .context("parsing --parameters (expected a JSON object like {\"region\":\"EU\"})")?,
        None => BTreeMap::new(),
    };

    let generated_at = chrono::Utc::now().to_rfc3339();
    let (html, name) =
        almagest_server::export_snapshot_html(path, dashboard, &raw_params, &generated_at)
            .await
            .context("baking the snapshot")?;

    let out = output
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{}-snapshot.html", slug(&name))));
    std::fs::write(&out, &html).with_context(|| format!("writing {}", out.display()))?;

    let kib = html.len() / 1024;
    println!("Exported \"{name}\" → {} ({kib} KiB)", out.display());
    if kib > 8 * 1024 {
        println!(
            "  note: this snapshot is large ({kib} KiB). Baked-in data has an email-size \
             ceiling — consider pre-aggregating, or host the file instead of emailing it."
        );
    }
    Ok(())
}

/// Reduce a dashboard name to a safe filename stem.
fn slug(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_lowercase();
    if trimmed.is_empty() {
        "dashboard".to_string()
    } else {
        trimmed
    }
}

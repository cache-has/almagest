// SPDX-License-Identifier: MIT OR Apache-2.0

//! Static HTML snapshot export (doc 11, export-ladder rung "static").
//!
//! `almagest export` bakes a single self-contained `.html` that carries the
//! frontend bundle, the dashboard definition, and every panel's pre-executed
//! result. It runs the queries here (against the same DataFusion engine as the
//! live server), encodes each result as base64 Arrow IPC, inlines the JS/CSS
//! bundle, and drops a `window.__ALMAGEST_SNAPSHOT__` payload the frontend reads
//! instead of calling a server (see `frontend/src/lib/snapshotData.ts`).
//!
//! The result opens in any browser via `file://` with no install and no network
//! — charts still render (ECharts runs client-side over the baked rows), tables
//! sort and paginate — but the data is frozen and parameters are read-only,
//! because there is no engine behind it. That's the static tier on purpose; the
//! interactive DuckDB-WASM tier is a later lift behind the same payload seam.

use crate::ServerError;
use crate::static_assets::inline_bundle;
use almagest_core::{AlmagestFile, Query};
use almagest_query::AlmagestQueryContext;
use arrow::ipc::writer::StreamWriter;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::Path;

/// Bake a self-contained static HTML snapshot of one dashboard.
///
/// `dashboard_id` selects the dashboard (defaults to the first one). `raw_params`
/// are user-supplied parameter values layered over the declared defaults.
/// Returns the HTML and the dashboard's display name (for the default filename).
pub async fn export_snapshot_html(
    path: &Path,
    dashboard_id: Option<&str>,
    raw_params: &BTreeMap<String, Value>,
    generated_at: &str,
) -> Result<(String, String), ServerError> {
    let file = AlmagestFile::open(path)?;
    let query = AlmagestQueryContext::open(&file)?;

    // Pick the dashboard (explicit id, else the first one in the file).
    let chosen_id = match dashboard_id {
        Some(id) => id.to_string(),
        None => file
            .list_dashboards()?
            .first()
            .map(|d| d.id.clone())
            .ok_or_else(|| ServerError::Export("the file has no dashboards to export".into()))?,
    };
    let dash = file.load_dashboard(&chosen_id)?;

    // Resolve parameters once; every panel runs against the same values.
    let today = chrono::Utc::now().date_naive();
    let resolved = almagest_query::resolve_parameters(&dash.parameters, raw_params, today)?;

    // Execute each panel that has a query → base64 Arrow IPC, keyed by panel id.
    let mut panels = Map::new();
    for row in &dash.layout.rows {
        for panel in &row.panels {
            let Some(q) = &panel.query else { continue };
            let sql = match q {
                Query::Inline { sql } => sql.clone(),
                Query::Reference { query_id } => file.saved_query(query_id)?.sql,
            };
            let result = query.execute(&sql, &resolved).await?;
            let ipc = encode_arrow_ipc(&result)?;
            panels.insert(panel.id.clone(), Value::String(STANDARD.encode(&ipc)));
        }
    }

    // Inline every embedded asset as a data URL (image panels reference them).
    let mut assets = Map::new();
    for (apath, _ct) in file.list_assets()? {
        let asset = file.asset(&apath)?;
        let data_url = format!(
            "data:{};base64,{}",
            asset.content_type,
            STANDARD.encode(&asset.content)
        );
        assets.insert(apath, Value::String(data_url));
    }

    // The parameter values to display in the frozen bar: provided ∪ declared.
    let mut display_params = Map::new();
    for p in &dash.parameters {
        if let Some(v) = raw_params.get(&p.id) {
            display_params.insert(p.id.clone(), v.clone());
        } else if let Some(d) = &p.default {
            display_params.insert(p.id.clone(), d.clone());
        }
    }

    let payload = json!({
        "dashboard": serde_json::to_value(&dash)?,
        "dashboardId": chosen_id,
        "panels": Value::Object(panels),
        "assets": Value::Object(assets),
        "params": Value::Object(display_params),
        "generatedAt": generated_at,
    });
    // Neutralize any `</script>` that could appear inside string values (e.g. a
    // dashboard title or text panel) so it can't break out of the inline script.
    let payload_json = serde_json::to_string(&payload)?.replace("</", "<\\/");

    let (js, css) = inline_bundle();
    let title = html_escape(&dash.name);
    let html = format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"/>\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"/>\
<title>{title} — snapshot</title>\n<style>{css}</style></head>\n\
<body><div id=\"app\"></div>\n\
<script>window.__ALMAGEST_SNAPSHOT__ = {payload_json};</script>\n\
<script type=\"module\">{js}</script>\n</body></html>\n"
    );

    Ok((html, dash.name))
}

/// Encode a query result as a single Arrow IPC stream (mirrors the live
/// panel-execute path so the frozen bytes decode identically in the frontend).
fn encode_arrow_ipc(result: &almagest_query::QueryResult) -> Result<Vec<u8>, ServerError> {
    let mut buf = Vec::new();
    let mut writer = StreamWriter::try_new(&mut buf, result.schema.as_ref())?;
    for batch in &result.batches {
        writer.write(batch)?;
    }
    writer.finish()?;
    drop(writer);
    Ok(buf)
}

/// Escape a string for HTML text (the `<title>`).
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

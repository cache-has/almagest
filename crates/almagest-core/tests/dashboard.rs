// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests for the dashboard DSL (Phase 05): parsing, validation, round-trip, and
//! typed persistence / export-import through a `.alm`.

use almagest_core::{AlmagestError, AlmagestFile, Dashboard};
use tempfile::TempDir;

const MINIMAL: &str = r#"{
  "version": 1,
  "name": "Minimal",
  "layout": { "rows": [ { "panels": [
    { "id": "p1", "kind": "metric", "span": 4, "query": { "sql": "SELECT 1 AS value FROM orders" } }
  ] } ] }
}"#;

/// Path to the checked-in example dashboard (workspace `examples/`).
fn example_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/dashboards/sales_overview.json")
}

fn invalid_location(json: &str) -> String {
    match Dashboard::from_json(json).unwrap_err() {
        AlmagestError::InvalidDashboard { location, .. } => location,
        other => panic!("expected InvalidDashboard, got {other:?}"),
    }
}

#[test]
fn example_file_parses_validates_and_keeps_panel_config() {
    let json = std::fs::read_to_string(example_path()).unwrap();
    let dash = Dashboard::from_json(&json).unwrap();
    assert_eq!(dash.name, "Sales Overview");
    assert_eq!(dash.parameters.len(), 2);
    assert_eq!(dash.layout.grid, 12);
    assert_eq!(dash.layout.rows.len(), 3);

    // Kind-specific config survives via the flattened `config` map.
    let chart = &dash.layout.rows[1].panels[0];
    assert_eq!(chart.id, "revenue_by_month");
    assert_eq!(
        chart.config.get("chart_type").and_then(|v| v.as_str()),
        Some("bar")
    );
}

#[test]
fn round_trips_through_json_losslessly() {
    let json = std::fs::read_to_string(example_path()).unwrap();
    let d1 = Dashboard::from_json(&json).unwrap();
    let reserialized = d1.to_json_pretty().unwrap();
    let d2 = Dashboard::from_json(&reserialized).unwrap();
    assert_eq!(
        d1, d2,
        "export → import must preserve the dashboard exactly"
    );
}

#[test]
fn minimal_dashboard_is_valid() {
    assert!(Dashboard::from_json(MINIMAL).is_ok());
}

#[test]
fn rejects_unsupported_version() {
    let json = MINIMAL.replace("\"version\": 1", "\"version\": 99");
    assert_eq!(invalid_location(&json), "version");
}

#[test]
fn rejects_empty_name() {
    let json = MINIMAL.replace("\"Minimal\"", "\"\"");
    assert_eq!(invalid_location(&json), "name");
}

#[test]
fn rejects_duplicate_parameter_ids() {
    let json = r#"{
      "version": 1, "name": "D",
      "parameters": [
        { "id": "x", "kind": "text" },
        { "id": "x", "kind": "number" }
      ],
      "layout": { "rows": [] }
    }"#;
    assert_eq!(invalid_location(json), "parameters[1]");
}

#[test]
fn rejects_select_without_options() {
    let json = r#"{
      "version": 1, "name": "D",
      "parameters": [ { "id": "r", "kind": "select" } ],
      "layout": { "rows": [] }
    }"#;
    assert_eq!(invalid_location(json), "parameters[0]");
}

#[test]
fn rejects_duplicate_panel_ids() {
    let json = r#"{
      "version": 1, "name": "D",
      "layout": { "rows": [ { "panels": [
        { "id": "p", "kind": "text", "span": 6 },
        { "id": "p", "kind": "text", "span": 6 }
      ] } ] }
    }"#;
    assert_eq!(invalid_location(json), "layout.rows[0].panels[1]");
}

#[test]
fn rejects_span_outside_grid() {
    let json = r#"{
      "version": 1, "name": "D",
      "layout": { "grid": 12, "rows": [ { "panels": [
        { "id": "p", "kind": "text", "span": 13 }
      ] } ] }
    }"#;
    assert_eq!(invalid_location(json), "layout.rows[0].panels[0]");
}

#[test]
fn rejects_data_panel_without_query() {
    let json = r#"{
      "version": 1, "name": "D",
      "layout": { "rows": [ { "panels": [
        { "id": "m", "kind": "metric", "span": 4 }
      ] } ] }
    }"#;
    assert_eq!(invalid_location(json), "layout.rows[0].panels[0]");
}

#[test]
fn rejects_visibility_referencing_unknown_param() {
    let json = r#"{
      "version": 1, "name": "D",
      "layout": { "rows": [ { "panels": [
        { "id": "t", "kind": "text", "span": 4,
          "visible": { "equals": { "param": "ghost", "value": true } } }
      ] } ] }
    }"#;
    assert_eq!(invalid_location(json), "layout.rows[0].panels[0]");
}

#[test]
fn rejects_templating_unknown_param() {
    let json = r#"{
      "version": 1, "name": "D",
      "layout": { "rows": [ { "panels": [
        { "id": "m", "kind": "metric", "span": 4,
          "query": { "sql": "SELECT * FROM orders WHERE region = {{ghost}}" } }
      ] } ] }
    }"#;
    assert_eq!(invalid_location(json), "layout.rows[0].panels[0]");
}

#[test]
fn rejects_daterange_subfield_on_non_daterange_param() {
    let json = r#"{
      "version": 1, "name": "D",
      "parameters": [ { "id": "d", "kind": "date" } ],
      "layout": { "rows": [ { "panels": [
        { "id": "m", "kind": "metric", "span": 4,
          "query": { "sql": "SELECT * FROM orders WHERE created_at >= {{d.start}}" } }
      ] } ] }
    }"#;
    assert_eq!(invalid_location(json), "layout.rows[0].panels[0]");
}

#[test]
fn saves_and_loads_typed_through_a_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("d.alm");
    let dash = Dashboard::from_json(MINIMAL).unwrap();

    let mut file = AlmagestFile::create(&path).unwrap();
    let id = file.save_dashboard(&dash, Some("/reports")).unwrap();
    let loaded = file.load_dashboard(&id).unwrap();
    assert_eq!(loaded, dash);
    file.close().unwrap();
}

#[test]
fn exports_then_imports_into_another_file() {
    let dir = TempDir::new().unwrap();
    let dash = Dashboard::from_json(MINIMAL).unwrap();

    // Save in file A, export to JSON.
    let a = dir.path().join("a.alm");
    let export = dir.path().join("dash.json");
    {
        let mut file = AlmagestFile::create(&a).unwrap();
        let id = file.save_dashboard(&dash, None).unwrap();
        file.export_dashboard_json(&id, &export).unwrap();
        file.close().unwrap();
    }

    // Import into a fresh file B.
    let b = dir.path().join("b.alm");
    let mut file = AlmagestFile::create(&b).unwrap();
    let id = file.import_dashboard_json(&export, None).unwrap();
    assert_eq!(file.load_dashboard(&id).unwrap(), dash);
    file.close().unwrap();
}

#[test]
fn import_with_dangling_query_reference_fails() {
    let dir = TempDir::new().unwrap();
    let json = r#"{
      "version": 1, "name": "D",
      "layout": { "rows": [ { "panels": [
        { "id": "m", "kind": "metric", "span": 4, "query": { "query_id": "nope" } }
      ] } ] }
    }"#;
    let export = dir.path().join("d.json");
    std::fs::write(&export, json).unwrap();

    let path = dir.path().join("f.alm");
    let mut file = AlmagestFile::create(&path).unwrap();
    let err = file.import_dashboard_json(&export, None).unwrap_err();
    assert!(
        matches!(err, AlmagestError::InvalidDashboard { .. }),
        "got {err:?}"
    );
}

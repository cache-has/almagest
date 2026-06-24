// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for parameter resolution and interactivity (Phase 07):
//! the DSL `Parameter` declarations resolve to typed values that actually run
//! through the DataFusion engine, `options_query` populates choices, and URL
//! state round-trips into a resolved query.

use std::collections::BTreeMap;
use std::sync::Arc;

use almagest_core::{AlmagestFile, Compression, ParamKind, Parameter};
use almagest_query::{
    AlmagestQueryContext, decode_url_state, layered_state, resolve_parameters, substitute,
};
use arrow::array::{Int64Array, RecordBatch, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use chrono::NaiveDate;
use serde_json::{Value, json};
use tempfile::TempDir;

/// orders(region, status, amount, created_at) — eight rows across regions,
/// statuses, and two months.
fn orders() -> (SchemaRef, RecordBatch) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, false),
        Field::new("created_at", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec![
                "US", "US", "EU", "EU", "US", "EU", "US", "EU",
            ])),
            Arc::new(StringArray::from(vec![
                "paid", "pending", "paid", "failed", "paid", "pending", "failed", "paid",
            ])),
            Arc::new(Int64Array::from(vec![10, 20, 5, 7, 30, 8, 15, 12])),
            Arc::new(StringArray::from(vec![
                "2026-05-03",
                "2026-05-18",
                "2026-05-25",
                "2026-06-01",
                "2026-06-10",
                "2026-06-15",
                "2026-06-20",
                "2026-06-24",
            ])),
        ],
    )
    .unwrap();
    (schema, batch)
}

fn make_file(path: &std::path::Path) {
    let (schema, batch) = orders();
    let mut f = AlmagestFile::create(path).unwrap();
    f.put_dataset("orders", schema, &[batch], Compression::Zstd)
        .unwrap();
    f.close().unwrap();
}

fn context(dir: &TempDir, name: &str) -> AlmagestQueryContext {
    let path = dir.path().join(name);
    make_file(&path);
    let file = AlmagestFile::open(&path).unwrap();
    AlmagestQueryContext::open(&file).unwrap()
}

fn count(result: &almagest_query::QueryResult) -> i64 {
    result.batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}

/// A bare declaration; tests fill the kind-specific fields they need.
fn param(id: &str, kind: ParamKind) -> Parameter {
    Parameter {
        id: id.into(),
        kind,
        label: None,
        description: None,
        default: None,
        options: None,
        options_query: None,
        min: None,
        max: None,
        min_selections: None,
        max_selections: None,
        allow_all: false,
        persist: None,
    }
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 6, 24).unwrap()
}

#[tokio::test]
async fn multiselect_and_select_resolve_and_filter() {
    let dir = TempDir::new().unwrap();
    let qc = context(&dir, "f.alm");

    let decls = vec![
        Parameter {
            options: Some(vec!["paid".into(), "pending".into(), "failed".into()]),
            ..param("status", ParamKind::MultiSelect)
        },
        Parameter {
            options: Some(vec!["All".into(), "US".into(), "EU".into()]),
            allow_all: true,
            ..param("region", ParamKind::Select)
        },
    ];

    let mut provided: BTreeMap<String, Value> = BTreeMap::new();
    provided.insert("status".into(), json!(["paid", "pending"]));
    provided.insert("region".into(), json!("US"));

    let params = resolve_parameters(&decls, &provided, today()).unwrap();
    let sql = "SELECT COUNT(*) AS n FROM orders \
               WHERE status IN ({{status}}) AND ({{region}} = 'All' OR region = {{region}})";

    let res = qc.execute(sql, &params).await.unwrap();
    // US rows: paid(10), pending(20), paid(30), failed(15). Keep paid+pending → 3.
    assert_eq!(count(&res), 3);

    // allow_all short-circuits the region filter.
    provided.insert("region".into(), json!("All"));
    let params = resolve_parameters(&decls, &provided, today()).unwrap();
    let res = qc.execute(sql, &params).await.unwrap();
    // All regions, paid+pending: paid(US,EU,US,EU)=4 + pending(US,EU)=2 → 6.
    assert_eq!(count(&res), 6);
}

#[tokio::test]
async fn daterange_expands_to_start_and_end() {
    let dir = TempDir::new().unwrap();
    let qc = context(&dir, "f.alm");

    let decls = vec![param("dr", ParamKind::DateRange)];
    let mut provided: BTreeMap<String, Value> = BTreeMap::new();
    provided.insert(
        "dr".into(),
        json!({ "start": "2026-06-01", "end": "2026-06-30" }),
    );

    let params = resolve_parameters(&decls, &provided, today()).unwrap();
    let sql = "SELECT COUNT(*) AS n FROM orders \
               WHERE CAST(created_at AS DATE) BETWEEN {{dr.start}} AND {{dr.end}}";
    let res = qc.execute(sql, &params).await.unwrap();
    // June rows: 06-01, 06-10, 06-15, 06-20, 06-24 → 5.
    assert_eq!(count(&res), 5);
}

#[tokio::test]
async fn daterange_preset_resolves_against_today() {
    let dir = TempDir::new().unwrap();
    let qc = context(&dir, "f.alm");

    let decls = vec![Parameter {
        default: Some(json!({ "preset": "this_month" })),
        ..param("dr", ParamKind::DateRange)
    }];

    // No provided value → falls back to the default preset, resolved at `today`.
    let params = resolve_parameters(&decls, &BTreeMap::new(), today()).unwrap();
    let sql = "SELECT COUNT(*) AS n FROM orders \
               WHERE CAST(created_at AS DATE) BETWEEN {{dr.start}} AND {{dr.end}}";
    let res = qc.execute(sql, &params).await.unwrap();
    // this_month = 2026-06-01..2026-06-24 → same 5 June rows.
    assert_eq!(count(&res), 5);
}

#[tokio::test]
async fn options_query_returns_distinct_values() {
    let dir = TempDir::new().unwrap();
    let qc = context(&dir, "f.alm");

    let opts = qc
        .resolve_options("SELECT DISTINCT region FROM orders ORDER BY region")
        .await
        .unwrap();
    assert_eq!(opts, vec!["EU".to_string(), "US".to_string()]);

    // Numeric first columns are cast to strings transparently.
    let amounts = qc
        .resolve_options("SELECT amount FROM orders WHERE region = 'EU' ORDER BY amount")
        .await
        .unwrap();
    assert_eq!(amounts, vec!["5", "7", "8", "12"]);
}

#[tokio::test]
async fn invalid_select_value_is_rejected_before_query() {
    let decls = vec![Parameter {
        options: Some(vec!["US".into(), "EU".into()]),
        ..param("region", ParamKind::Select)
    }];
    let mut provided = BTreeMap::new();
    provided.insert("region".to_string(), json!("ZZ"));

    let err = resolve_parameters(&decls, &provided, today()).unwrap_err();
    assert!(
        matches!(err, almagest_query::QueryError::Param(_)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn url_state_decodes_then_resolves_and_runs() {
    let dir = TempDir::new().unwrap();
    let qc = context(&dir, "f.alm");

    let decls = vec![Parameter {
        options: Some(vec!["All".into(), "US".into(), "EU".into()]),
        allow_all: true,
        ..param("region", ParamKind::Select)
    }];

    // A shared link carrying region=EU.
    let from_url = decode_url_state("region=EU", &decls);
    let merged = layered_state(&from_url, &BTreeMap::new());
    let params = resolve_parameters(&decls, &merged, today()).unwrap();

    let res = qc
        .execute(
            "SELECT COUNT(*) AS n FROM orders WHERE region = {{region}}",
            &params,
        )
        .await
        .unwrap();
    // EU rows: 4.
    assert_eq!(count(&res), 4);
}

#[tokio::test]
async fn multiselect_count_bounds_enforced() {
    let decls = vec![Parameter {
        options: Some(vec!["a".into(), "b".into(), "c".into()]),
        max_selections: Some(2),
        ..param("tags", ParamKind::MultiSelect)
    }];
    let mut provided = BTreeMap::new();
    provided.insert("tags".to_string(), json!(["a", "b", "c"]));

    let err = resolve_parameters(&decls, &provided, today()).unwrap_err();
    assert!(matches!(err, almagest_query::QueryError::Param(_)));

    // Empty multiselect renders a NULL list → matches nothing, but is valid SQL.
    let dir = TempDir::new().unwrap();
    let qc = context(&dir, "f.alm");
    let mut empty = BTreeMap::new();
    empty.insert("tags".to_string(), json!([]));
    let params = resolve_parameters(
        &[Parameter {
            options: Some(vec!["a".into()]),
            ..param("tags", ParamKind::MultiSelect)
        }],
        &empty,
        today(),
    )
    .unwrap();
    let sql = substitute(
        "SELECT COUNT(*) AS n FROM orders WHERE status IN ({{tags}})",
        &params,
    )
    .unwrap();
    assert_eq!(
        sql,
        "SELECT COUNT(*) AS n FROM orders WHERE status IN (NULL)"
    );
    let res = qc
        .execute(
            "SELECT COUNT(*) AS n FROM orders WHERE status IN ({{tags}})",
            &params,
        )
        .await
        .unwrap();
    assert_eq!(count(&res), 0);
}

// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the query engine layer (Phase 03).

use std::collections::HashMap;
use std::sync::Arc;

use almagest_core::{AlmagestFile, Compression};
use almagest_query::{
    AlmagestQueryContext, ContextOptions, ParamDecl, ParamSchema, ParamType, ParamValue,
    QueryParams,
};
use arrow::array::{Int64Array, RecordBatch, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use tempfile::TempDir;

/// An `orders` dataset: region + amount, six rows across two regions.
fn orders() -> (SchemaRef, RecordBatch) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["US", "US", "EU", "EU", "US", "EU"])),
            Arc::new(Int64Array::from(vec![10, 20, 5, 7, 30, 8])),
        ],
    )
    .unwrap();
    (schema, batch)
}

/// Create a `.alm` at `path` with the `orders` dataset baked in.
fn make_file(path: &std::path::Path) {
    let (schema, batch) = orders();
    let mut f = AlmagestFile::create(path).unwrap();
    f.put_dataset("orders", schema, &[batch], Compression::Zstd)
        .unwrap();
    f.close().unwrap();
}

fn single_i64(result: &almagest_query::QueryResult, col: usize) -> i64 {
    result.batches[0]
        .column(col)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}

#[tokio::test]
async fn registers_data_and_runs_a_query() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("q.alm");
    make_file(&path);

    let file = AlmagestFile::open(&path).unwrap();
    let qc = AlmagestQueryContext::open(&file).unwrap();

    let res = qc
        .execute("SELECT COUNT(*) AS n FROM orders", &QueryParams::empty())
        .await
        .unwrap();
    assert_eq!(res.row_count, 1);
    assert_eq!(single_i64(&res, 0), 6);
    assert!(!res.cached);
}

#[tokio::test]
async fn schema_introspection_lists_tables_and_columns() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("s.alm");
    make_file(&path);

    let file = AlmagestFile::open(&path).unwrap();
    let qc = AlmagestQueryContext::open(&file).unwrap();

    let schema = qc.schema();
    assert_eq!(schema.tables.len(), 1);
    let t = &schema.tables[0];
    assert_eq!(t.name, "orders");
    assert_eq!(t.row_count, 6);
    let cols: Vec<&str> = t.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cols, vec!["region", "amount"]);
    assert_eq!(t.columns[1].data_type, "Int64");
}

#[tokio::test]
async fn typed_parameters_filter_safely() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("p.alm");
    make_file(&path);

    let file = AlmagestFile::open(&path).unwrap();
    let qc = AlmagestQueryContext::open(&file).unwrap();

    let schema = ParamSchema::new(vec![
        ParamDecl::required("region", ParamType::String),
        ParamDecl::required("min_amount", ParamType::Integer)
            .with_default(ParamValue::Integer(0))
            .with_bounds(0.0, 1000.0),
    ]);
    let mut provided = HashMap::new();
    provided.insert("region".to_string(), ParamValue::String("US".to_string()));
    provided.insert("min_amount".to_string(), ParamValue::Integer(15));
    let params = schema.resolve(&provided).unwrap();

    let res = qc
        .execute(
            "SELECT SUM(amount) AS total FROM orders \
             WHERE region = {{region}} AND amount >= {{min_amount}}",
            &params,
        )
        .await
        .unwrap();
    // US rows >= 15: 20 + 30 = 50.
    assert_eq!(single_i64(&res, 0), 50);
}

#[tokio::test]
async fn malformed_string_param_cannot_alter_query_structure() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("inj.alm");
    make_file(&path);

    let file = AlmagestFile::open(&path).unwrap();
    let qc = AlmagestQueryContext::open(&file).unwrap();

    // A classic injection payload as the *value* must stay an inert string
    // literal: it matches no region, so the result is 0 rows — and the orders
    // table is obviously still queryable afterward.
    let mut provided = HashMap::new();
    provided.insert(
        "region".to_string(),
        ParamValue::String("US' OR '1'='1".to_string()),
    );
    let params = QueryParams::from_values(provided);

    let res = qc
        .execute(
            "SELECT COUNT(*) AS n FROM orders WHERE region = {{region}}",
            &params,
        )
        .await
        .unwrap();
    assert_eq!(
        single_i64(&res, 0),
        0,
        "injection payload must not match all rows"
    );

    // Table still intact.
    let after = qc
        .execute("SELECT COUNT(*) AS n FROM orders", &QueryParams::empty())
        .await
        .unwrap();
    assert_eq!(single_i64(&after, 0), 6);
}

#[tokio::test]
async fn unbound_parameter_is_an_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("u.alm");
    make_file(&path);

    let file = AlmagestFile::open(&path).unwrap();
    let qc = AlmagestQueryContext::open(&file).unwrap();

    let err = qc
        .execute(
            "SELECT * FROM orders WHERE region = {{missing}}",
            &QueryParams::empty(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, almagest_query::QueryError::UnboundParam(ref n) if n == "missing"),
        "got {err:?}"
    );
}

#[tokio::test]
async fn bad_sql_maps_to_friendly_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad.alm");
    make_file(&path);

    let file = AlmagestFile::open(&path).unwrap();
    let qc = AlmagestQueryContext::open(&file).unwrap();

    let err = qc
        .execute("SELECT * FROM no_such_table", &QueryParams::empty())
        .await
        .unwrap_err();
    assert!(
        matches!(err, almagest_query::QueryError::DataFusion(_)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn second_identical_query_is_served_from_cache() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("c.alm");
    make_file(&path);

    let file = AlmagestFile::open(&path).unwrap();
    let qc = AlmagestQueryContext::open(&file).unwrap();

    let sql = "SELECT SUM(amount) AS total FROM orders";
    let first = qc.execute(sql, &QueryParams::empty()).await.unwrap();
    assert!(!first.cached);
    assert_eq!(single_i64(&first, 0), 80);

    let second = qc.execute(sql, &QueryParams::empty()).await.unwrap();
    assert!(second.cached, "identical query should hit the cache");
    assert_eq!(single_i64(&second, 0), 80, "cached value matches");
}

#[tokio::test]
async fn rebaking_data_invalidates_the_cache() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("rebake.alm");
    make_file(&path);

    let sql = "SELECT SUM(amount) AS total FROM orders";

    // Warm the cache with the original data (sum = 80).
    {
        let file = AlmagestFile::open(&path).unwrap();
        let qc = AlmagestQueryContext::open(&file).unwrap();
        let r = qc.execute(sql, &QueryParams::empty()).await.unwrap();
        assert_eq!(single_i64(&r, 0), 80);
        file.close().unwrap();
    }

    // Re-bake `orders` with different numbers (sum = 3).
    {
        let schema = Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("amount", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["US", "EU"])),
                Arc::new(Int64Array::from(vec![1, 2])),
            ],
        )
        .unwrap();
        let mut file = AlmagestFile::open(&path).unwrap();
        file.put_dataset("orders", schema, &[batch], Compression::Zstd)
            .unwrap();
        file.close().unwrap();
    }

    // A fresh context must reflect the new data, not the stale cached 80.
    let file = AlmagestFile::open(&path).unwrap();
    let qc = AlmagestQueryContext::open(&file).unwrap();
    let r = qc.execute(sql, &QueryParams::empty()).await.unwrap();
    assert!(!r.cached, "data-version changed, so this is a cache miss");
    assert_eq!(single_i64(&r, 0), 3, "must reflect re-baked data");
}

#[tokio::test]
async fn cross_dataset_join_works_in_one_context() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("join.alm");

    // Two datasets in the same file.
    let (oschema, obatch) = orders();
    let rschema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let rbatch = RecordBatch::try_new(
        rschema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["US", "EU"])),
            Arc::new(StringArray::from(vec!["United States", "Europe"])),
        ],
    )
    .unwrap();

    let mut f = AlmagestFile::create(&path).unwrap();
    f.put_dataset("orders", oschema, &[obatch], Compression::Zstd)
        .unwrap();
    f.put_dataset("regions", rschema, &[rbatch], Compression::Zstd)
        .unwrap();
    f.close().unwrap();

    let file = AlmagestFile::open(&path).unwrap();
    let qc = AlmagestQueryContext::open(&file).unwrap();
    let res = qc
        .execute(
            "SELECT r.label, SUM(o.amount) AS total \
             FROM orders o JOIN regions r ON o.region = r.region \
             GROUP BY r.label ORDER BY r.label",
            &QueryParams::empty(),
        )
        .await
        .unwrap();
    assert_eq!(res.row_count, 2);
    // EU = 5+7+8 = 20, US = 10+20+30 = 60. Ordered by label: Europe, United States.
    let labels = res.batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(labels.value(0), "Europe");
    assert_eq!(single_i64(&res, 1), 20);
}

#[tokio::test]
async fn out_of_bounds_parameter_is_rejected_before_execution() {
    let schema = ParamSchema::new(vec![
        ParamDecl::required("limit", ParamType::Integer).with_bounds(1.0, 100.0),
    ]);
    let mut provided = HashMap::new();
    provided.insert("limit".to_string(), ParamValue::Integer(500));
    let err = schema.resolve(&provided).unwrap_err();
    assert!(
        matches!(err, almagest_query::QueryError::Param(_)),
        "got {err:?}"
    );

    // Wrong type is also rejected.
    let mut wrong = HashMap::new();
    wrong.insert("limit".to_string(), ParamValue::String("nope".to_string()));
    assert!(schema.resolve(&wrong).is_err());
}

#[tokio::test]
async fn custom_options_use_a_tiny_cache_budget() {
    // Just exercises the options path: a tiny cache budget still works (entries
    // get evicted, queries still return correct results).
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("opt.alm");
    make_file(&path);

    let file = AlmagestFile::open(&path).unwrap();
    let opts = ContextOptions {
        cache_max_bytes: 1, // forces eviction of anything written
        ..Default::default()
    };
    let qc = AlmagestQueryContext::open_with(&file, &opts).unwrap();

    let r1 = qc
        .execute(
            "SELECT SUM(amount) AS total FROM orders",
            &QueryParams::empty(),
        )
        .await
        .unwrap();
    assert_eq!(single_i64(&r1, 0), 80);
    // With a 1-byte budget the entry is evicted immediately, so the next call
    // is still a miss — but correct.
    let r2 = qc
        .execute(
            "SELECT SUM(amount) AS total FROM orders",
            &QueryParams::empty(),
        )
        .await
        .unwrap();
    assert!(!r2.cached, "tiny budget evicts, so no hit");
    assert_eq!(single_i64(&r2, 0), 80);
}

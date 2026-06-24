// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the author-time importers (Phase 04).
//!
//! Each test writes a representative local source, imports it, and verifies the
//! round-trip by querying the baked `.alm` through the DataFusion engine.

use std::sync::Arc;

use almagest_connectors::{
    CsvImporter, CsvOptions, ImportError, Importer, JsonFormat, JsonImporter, JsonMode,
    JsonOptions, ParquetImporter, ParquetOptions, SqliteImporter, SqliteOptions,
};
use almagest_core::AlmagestFile;
use almagest_query::{AlmagestQueryContext, QueryParams};
use arrow::array::{Int64Array, RecordBatch, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use tempfile::TempDir;

/// Open the file, query `sql`, and return the single Int64 cell at (0, col).
async fn query_i64(path: &std::path::Path, sql: &str, col: usize) -> i64 {
    let file = AlmagestFile::open(path).unwrap();
    let qc = AlmagestQueryContext::open(&file).unwrap();
    let res = qc.execute(sql, &QueryParams::empty()).await.unwrap();
    res.batches[0]
        .column(col)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}

fn new_almagest(dir: &TempDir, file: &str) -> std::path::PathBuf {
    let path = dir.path().join(file);
    AlmagestFile::create(&path).unwrap().close().unwrap();
    path
}

#[tokio::test]
async fn csv_import_round_trips() {
    let dir = TempDir::new().unwrap();
    let csv = dir.path().join("orders.csv");
    std::fs::write(&csv, "region,amount\nUS,10\nEU,5\nUS,20\n").unwrap();
    let almagest = new_almagest(&dir, "f.alm");

    let imported = {
        let mut file = AlmagestFile::open(&almagest).unwrap();
        let out = CsvImporter::new(CsvOptions::new(&csv))
            .import(&mut file)
            .unwrap();
        file.close().unwrap();
        out
    };
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].name, "orders");
    assert_eq!(imported[0].meta.row_count, 3);
    // Provenance recorded.
    assert!(
        imported[0]
            .meta
            .source_json
            .as_ref()
            .unwrap()
            .contains("\"kind\":\"csv\"")
    );

    assert_eq!(
        query_i64(&almagest, "SELECT SUM(amount) AS t FROM orders", 0).await,
        35
    );
}

#[tokio::test]
async fn csv_column_override_changes_type() {
    let dir = TempDir::new().unwrap();
    let csv = dir.path().join("nums.csv");
    // Without an override, amount infers as Int64; force Float64.
    std::fs::write(&csv, "amount\n10\n20\n").unwrap();
    let almagest = new_almagest(&dir, "f.alm");

    let mut opts = CsvOptions::new(&csv);
    opts.column_overrides
        .insert("amount".to_string(), "float64".to_string());
    let mut file = AlmagestFile::open(&almagest).unwrap();
    let out = CsvImporter::new(opts).import(&mut file).unwrap();
    let schema_json = &out[0].meta.arrow_schema_json;
    assert!(schema_json.contains("Float64"), "got {schema_json}");
    file.close().unwrap();
}

#[tokio::test]
async fn parquet_import_with_projection() {
    use parquet::arrow::ArrowWriter;

    let dir = TempDir::new().unwrap();
    let pq = dir.path().join("data.parquet");

    // Write a 3-column Parquet source.
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec!["US", "EU", "US"])),
            Arc::new(Int64Array::from(vec![10, 5, 20])),
        ],
    )
    .unwrap();
    {
        let f = std::fs::File::create(&pq).unwrap();
        let mut w = ArrowWriter::try_new(f, schema, None).unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();
    }

    let almagest = new_almagest(&dir, "f.alm");
    let mut opts = ParquetOptions::new(&pq);
    opts.name = Some("orders".to_string());
    opts.columns = Some(vec!["region".to_string(), "amount".to_string()]);

    let mut file = AlmagestFile::open(&almagest).unwrap();
    let out = ParquetImporter::new(opts).import(&mut file).unwrap();
    assert_eq!(out[0].meta.row_count, 3);
    // Only the projected columns made it in.
    let cols = &out[0].meta.arrow_schema_json;
    assert!(cols.contains("region") && cols.contains("amount"));
    assert!(
        !cols.contains("\"name\":\"id\""),
        "id was projected out: {cols}"
    );
    file.close().unwrap();

    assert_eq!(
        query_i64(&almagest, "SELECT SUM(amount) AS t FROM orders", 0).await,
        35
    );
}

#[tokio::test]
async fn json_ndjson_import_round_trips() {
    let dir = TempDir::new().unwrap();
    let json = dir.path().join("events.json");
    std::fs::write(
        &json,
        "{\"region\":\"US\",\"amount\":10}\n{\"region\":\"EU\",\"amount\":5}\n",
    )
    .unwrap();
    let almagest = new_almagest(&dir, "f.alm");

    let mut file = AlmagestFile::open(&almagest).unwrap();
    let out = JsonImporter::new(JsonOptions::new(&json))
        .import(&mut file)
        .unwrap();
    assert_eq!(out[0].name, "events");
    assert_eq!(out[0].meta.row_count, 2);
    file.close().unwrap();

    assert_eq!(
        query_i64(&almagest, "SELECT SUM(amount) AS t FROM events", 0).await,
        15
    );
}

#[tokio::test]
async fn json_array_with_record_path_and_lenient_skip() {
    let dir = TempDir::new().unwrap();
    let json = dir.path().join("payload.json");
    // Records live under $.data; one element is not an object and must be skipped.
    std::fs::write(
        &json,
        r#"{"data": [{"region":"US","amount":10}, 42, {"region":"EU","amount":5}]}"#,
    )
    .unwrap();
    let almagest = new_almagest(&dir, "f.alm");

    let mut opts = JsonOptions::new(&json);
    opts.format = JsonFormat::Array;
    opts.record_path = Some("$.data".to_string());
    opts.mode = JsonMode::Lenient;
    opts.name = Some("rows".to_string());

    let mut file = AlmagestFile::open(&almagest).unwrap();
    let out = JsonImporter::new(opts).import(&mut file).unwrap();
    assert_eq!(
        out[0].meta.row_count, 2,
        "the non-object element is skipped"
    );
    assert_eq!(out[0].report.rows_skipped, 1);
    assert!(!out[0].report.warnings.is_empty());
    file.close().unwrap();

    assert_eq!(
        query_i64(&almagest, "SELECT SUM(amount) AS t FROM rows", 0).await,
        15
    );
}

#[tokio::test]
async fn json_strict_mode_fails_on_bad_record() {
    let dir = TempDir::new().unwrap();
    let json = dir.path().join("p.json");
    std::fs::write(&json, r#"[{"a":1}, 7]"#).unwrap();
    let almagest = new_almagest(&dir, "f.alm");

    let mut opts = JsonOptions::new(&json);
    opts.format = JsonFormat::Array;
    opts.mode = JsonMode::Strict;

    let mut file = AlmagestFile::open(&almagest).unwrap();
    let err = JsonImporter::new(opts).import(&mut file).unwrap_err();
    assert!(
        matches!(err, ImportError::MalformedRecord { row: 1, .. }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn sqlite_import_multiple_tables_with_prefix() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("source.db");
    {
        let c = rusqlite::Connection::open(&src).unwrap();
        c.execute_batch(
            "CREATE TABLE orders (id INTEGER, amount REAL);
             INSERT INTO orders VALUES (1, 10.5), (2, 4.5);
             CREATE TABLE customers (id INTEGER, name TEXT);
             INSERT INTO customers VALUES (1, 'Acme'), (2, 'Globex');",
        )
        .unwrap();
        c.close().unwrap();
    }
    let almagest = new_almagest(&dir, "f.alm");

    let mut opts = SqliteOptions::new(&src);
    opts.name_prefix = "src_".to_string();

    let mut file = AlmagestFile::open(&almagest).unwrap();
    let out = SqliteImporter::new(opts).import(&mut file).unwrap();
    let names: Vec<&str> = out.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"src_orders") && names.contains(&"src_customers"));
    file.close().unwrap();

    // REAL column summed via the engine.
    let file = AlmagestFile::open(&almagest).unwrap();
    let qc = AlmagestQueryContext::open(&file).unwrap();
    let res = qc
        .execute(
            "SELECT SUM(amount) AS t FROM src_orders",
            &QueryParams::empty(),
        )
        .await
        .unwrap();
    let total = res.batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Float64Array>()
        .unwrap()
        .value(0);
    assert_eq!(total, 15.0);
}

#[tokio::test]
async fn name_collision_is_rejected_unless_replace() {
    let dir = TempDir::new().unwrap();
    let csv = dir.path().join("orders.csv");
    std::fs::write(&csv, "amount\n1\n").unwrap();
    let almagest = new_almagest(&dir, "f.alm");

    let mut file = AlmagestFile::open(&almagest).unwrap();
    CsvImporter::new(CsvOptions::new(&csv))
        .import(&mut file)
        .unwrap();

    // Second import to the same name without replace → collision.
    let err = CsvImporter::new(CsvOptions::new(&csv))
        .import(&mut file)
        .unwrap_err();
    assert!(
        matches!(err, ImportError::NameCollision { .. }),
        "got {err:?}"
    );

    // With replace set, it succeeds.
    let mut opts = CsvOptions::new(&csv);
    opts.replace = true;
    assert!(CsvImporter::new(opts).import(&mut file).is_ok());
    file.close().unwrap();
}

#[tokio::test]
async fn missing_source_is_reported() {
    let dir = TempDir::new().unwrap();
    let almagest = new_almagest(&dir, "f.alm");
    let mut file = AlmagestFile::open(&almagest).unwrap();

    let err = CsvImporter::new(CsvOptions::new(dir.path().join("ghost.csv")))
        .import(&mut file)
        .unwrap_err();
    assert!(
        matches!(err, ImportError::SourceNotFound { .. }),
        "got {err:?}"
    );
}

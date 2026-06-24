// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the `.alm` format layer (Phase 02).

use std::sync::Arc;

use almagest_core::{AlmagestError, AlmagestFile, Compression, FORMAT_VERSION};
use arrow::array::{Int64Array, RecordBatch, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use tempfile::TempDir;

/// A small two-column dataset used across the data tests.
fn sample() -> (SchemaRef, RecordBatch) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec![Some("a"), None, Some("c")])),
        ],
    )
    .unwrap();
    (schema, batch)
}

fn tmp(name: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(name);
    (dir, path)
}

#[test]
fn create_then_reopen_preserves_identity() {
    let (_dir, path) = tmp("a.alm");

    let id = {
        let mut f = AlmagestFile::create(&path).unwrap();
        f.set_title("Quarterly").unwrap();
        f.set_description("Q3 numbers").unwrap();
        let id = f.almagest_id().unwrap();
        assert_eq!(f.format_version().unwrap(), FORMAT_VERSION);
        f.close().unwrap();
        id
    };

    let f = AlmagestFile::open(&path).unwrap();
    assert_eq!(f.almagest_id().unwrap(), id, "almagest_id must be stable");
    assert_eq!(f.title().unwrap(), "Quarterly");
    assert_eq!(f.description().unwrap(), "Q3 numbers");
    assert_eq!(f.format_version().unwrap(), FORMAT_VERSION);
    f.integrity_check().unwrap();
    f.close().unwrap();
}

#[test]
fn create_refuses_to_overwrite() {
    let (_dir, path) = tmp("b.alm");
    AlmagestFile::create(&path).unwrap().close().unwrap();
    let err = AlmagestFile::create(&path).unwrap_err();
    assert!(matches!(err, AlmagestError::Invalid(_)), "got {err:?}");
}

#[test]
fn open_missing_file_errors() {
    let (_dir, path) = tmp("nope.alm");
    let err = AlmagestFile::open(&path).unwrap_err();
    assert!(matches!(err, AlmagestError::Io(_)), "got {err:?}");
}

#[test]
fn open_non_almagest_sqlite_is_rejected() {
    let (_dir, path) = tmp("plain.db");
    // A valid SQLite DB with no almagest tables.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE widgets (x INTEGER);")
        .unwrap();
    conn.close().unwrap();

    let err = AlmagestFile::open(&path).unwrap_err();
    assert!(
        matches!(err, AlmagestError::NotAAlmagestFile { .. }),
        "got {err:?}"
    );
}

#[test]
fn close_collapses_wal_sidecars_into_one_file() {
    let (dir, path) = tmp("single.alm");
    let mut f = AlmagestFile::create(&path).unwrap();
    let (schema, batch) = sample();
    f.put_dataset("t", schema, &[batch], Compression::Zstd)
        .unwrap();
    f.close().unwrap();

    // After close, only the .alm file should remain — no -wal / -shm.
    let entries: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["single.alm".to_string()],
        "stray sidecars: {entries:?}"
    );
}

#[test]
fn refuses_file_from_a_newer_format() {
    let (_dir, path) = tmp("future.alm");
    AlmagestFile::create(&path).unwrap().close().unwrap();

    // Forge a migration row claiming a far-future format version.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO almagest_migrations (version, applied_at, description) VALUES (?1, '2099', 'forged')",
        [FORMAT_VERSION + 99],
    )
    .unwrap();
    conn.close().unwrap();

    let err = AlmagestFile::open(&path).unwrap_err();
    match err {
        AlmagestError::FormatTooNew { found, supported } => {
            assert_eq!(found, FORMAT_VERSION + 99);
            assert_eq!(supported, FORMAT_VERSION);
        }
        other => panic!("expected FormatTooNew, got {other:?}"),
    }
}

#[test]
fn integrity_check_catches_corruption_without_panicking() {
    let (_dir, path) = tmp("corrupt.alm");
    AlmagestFile::create(&path).unwrap().close().unwrap();

    // Truncate the file mid-page to corrupt it, then try to open.
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len() / 2);
    std::fs::write(&path, &bytes).unwrap();

    // Either the open fails or the integrity check does — never a panic, and
    // never silent success that returns a usable handle over corrupt data.
    match AlmagestFile::open(&path) {
        Ok(f) => {
            assert!(
                f.integrity_check().is_err(),
                "corrupt file passed integrity check"
            );
        }
        Err(_) => { /* refused on open — also acceptable */ }
    }
}

#[test]
fn dataset_round_trips_losslessly() {
    let (_dir, path) = tmp("data.alm");
    let (schema, batch) = sample();

    let mut f = AlmagestFile::create(&path).unwrap();
    let meta = f
        .put_dataset(
            "orders",
            schema.clone(),
            std::slice::from_ref(&batch),
            Compression::Zstd,
        )
        .unwrap();
    assert_eq!(meta.row_count, 3);
    assert_eq!(meta.compression, "zstd");
    assert!(meta.byte_size > 0);
    f.close().unwrap();

    let f = AlmagestFile::open(&path).unwrap();
    let back = f.read_dataset("orders").unwrap();
    let total: usize = back.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total, 3);
    assert_eq!(back[0].schema(), schema, "schema must round-trip");
    assert_eq!(&back[0], &batch, "rows must round-trip");

    // Missing dataset surfaces as NotFound, not a panic.
    let err = f.read_dataset("ghost").unwrap_err();
    assert!(matches!(err, AlmagestError::NotFound { .. }), "got {err:?}");
    f.close().unwrap();
}

#[test]
fn put_dataset_replaces_transactionally_keeping_id() {
    let (_dir, path) = tmp("replace.alm");
    let (schema, batch) = sample();

    let mut f = AlmagestFile::create(&path).unwrap();
    let first = f
        .put_dataset(
            "t",
            schema.clone(),
            std::slice::from_ref(&batch),
            Compression::Zstd,
        )
        .unwrap();

    // Replace with a single-row version.
    let small = batch.slice(0, 1);
    let second = f
        .put_dataset("t", schema.clone(), &[small], Compression::Snappy)
        .unwrap();

    assert_eq!(first.id, second.id, "id stable across replace");
    assert_eq!(first.created_at, second.created_at, "created_at preserved");
    assert_eq!(second.row_count, 1);
    assert_eq!(second.compression, "snappy");
    assert_eq!(
        f.list_datasets().unwrap().len(),
        1,
        "no duplicate dataset rows"
    );
    f.close().unwrap();
}

#[test]
fn exported_blob_is_a_standalone_parquet_file() {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let (dir, path) = tmp("export.alm");
    let (schema, batch) = sample();

    let mut f = AlmagestFile::create(&path).unwrap();
    f.put_dataset("orders", schema, &[batch], Compression::Zstd)
        .unwrap();

    let out = dir.path().join("orders.parquet");
    f.export_dataset_parquet("orders", &out).unwrap();
    f.close().unwrap();

    // Read it back with the parquet reader directly — no Almagest involved —
    // proving the blob is portable to DuckDB/Polars/pandas etc.
    let file = std::fs::File::open(&out).unwrap();
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap();
    let rows: usize = reader.map(|b| b.unwrap().num_rows()).sum();
    assert_eq!(rows, 3);
}

#[test]
fn assets_store_retrieve_and_guess_type() {
    let (_dir, path) = tmp("assets.alm");
    let mut f = AlmagestFile::create(&path).unwrap();

    f.put_asset("logo.png", b"\x89PNG fake", None).unwrap();
    f.put_asset("theme.css", b"body{}", Some("text/css"))
        .unwrap();

    let logo = f.asset("logo.png").unwrap();
    assert_eq!(
        logo.content_type, "image/png",
        "type guessed from extension"
    );
    assert_eq!(logo.content, b"\x89PNG fake");

    // Replace updates content in place.
    f.put_asset("logo.png", b"newer", None).unwrap();
    assert_eq!(f.asset("logo.png").unwrap().content, b"newer");

    assert_eq!(f.list_assets().unwrap().len(), 2);
    assert!(f.remove_asset("theme.css").unwrap());
    assert_eq!(f.list_assets().unwrap().len(), 1);

    let err = f.asset("gone").unwrap_err();
    assert!(matches!(err, AlmagestError::NotFound { .. }), "got {err:?}");
    f.close().unwrap();
}

#[test]
fn dashboards_crud_and_json_validation() {
    let (_dir, path) = tmp("dash.alm");
    let mut f = AlmagestFile::create(&path).unwrap();

    let id = f
        .create_dashboard("Sales", Some("desc"), Some("/reports"), r#"{"panels":[]}"#)
        .unwrap();
    let d = f.dashboard(&id).unwrap();
    assert_eq!(d.name, "Sales");
    assert_eq!(d.folder.as_deref(), Some("/reports"));

    f.update_dashboard(&id, "Sales v2", None, None, r#"{"panels":[{"k":"kpi"}]}"#)
        .unwrap();
    assert_eq!(f.dashboard(&id).unwrap().name, "Sales v2");

    // Invalid JSON is rejected.
    let err = f
        .create_dashboard("Bad", None, None, "{not json")
        .unwrap_err();
    assert!(matches!(err, AlmagestError::Serde(_)), "got {err:?}");

    // Unknown id on update → NotFound.
    let err = f
        .update_dashboard("nope", "x", None, None, "{}")
        .unwrap_err();
    assert!(matches!(err, AlmagestError::NotFound { .. }), "got {err:?}");

    assert_eq!(f.list_dashboards().unwrap().len(), 1);
    assert!(f.remove_dashboard(&id).unwrap());
    assert_eq!(f.list_dashboards().unwrap().len(), 0);
    f.close().unwrap();
}

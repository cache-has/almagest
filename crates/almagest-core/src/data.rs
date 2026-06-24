// SPDX-License-Identifier: MIT OR Apache-2.0

//! Embedded datasets: the `almagest_data` Parquet-blob layer.
//!
//! Bulk row data never lives in SQLite tables. Each logical dataset is encoded
//! as a single compressed Parquet file and stored as one BLOB in `almagest_data`,
//! keyed by a query-visible `name`. On open (doc 03) the blob decodes to an
//! Arrow/DataFusion `MemTable`; here in almagest-core we only own the lossless
//! round-trip Arrow ⇄ Parquet-blob and the bookkeeping columns.
//!
//! Because the blob is a standard Parquet file, it can be pulled out and read
//! by DuckDB / Polars / pandas with no Almagest involved — the portability promise.

use crate::AlmagestFile;
use crate::error::{AlmagestError, Result};
use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use bytes::Bytes;
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::{Compression as PqCompression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use std::path::Path;

/// Parquet column compression used for an embedded dataset.
///
/// `Zstd` is the default — it compresses analytic data substantially smaller,
/// which is what keeps a "file you email" emailable. `Snappy` decodes faster;
/// `None` is for already-incompressible data or debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// zstd — best ratio (default).
    Zstd,
    /// snappy — faster decode, larger files.
    Snappy,
    /// uncompressed.
    None,
}

impl Compression {
    fn to_parquet(self) -> PqCompression {
        match self {
            Compression::Zstd => PqCompression::ZSTD(ZstdLevel::default()),
            Compression::Snappy => PqCompression::SNAPPY,
            Compression::None => PqCompression::UNCOMPRESSED,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Compression::Zstd => "zstd",
            Compression::Snappy => "snappy",
            Compression::None => "none",
        }
    }
}

/// Bookkeeping metadata for an embedded dataset (everything except the blob).
#[derive(Debug, Clone)]
pub struct DatasetMeta {
    /// Stable per-dataset id (UUID).
    pub id: String,
    /// Logical table name that queries reference (unique within the file).
    pub name: String,
    /// Number of rows across the dataset.
    pub row_count: u64,
    /// Size of the stored Parquet blob in bytes.
    pub byte_size: u64,
    /// Compression codec used.
    pub compression: String,
    /// Compact JSON description of the Arrow schema (for introspection without
    /// decoding the blob).
    pub arrow_schema_json: String,
    /// Provenance: how the dataset was imported (path, importer kind, options,
    /// timestamp), if it was written by an importer. `None` for datasets put
    /// directly via [`AlmagestFile::put_dataset`].
    pub source_json: Option<String>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 last-update timestamp.
    pub updated_at: String,
}

impl AlmagestFile {
    /// Write (or replace) a dataset under `name`, encoding `batches` as a
    /// compressed Parquet blob. Replacement is transactional: the old blob is
    /// only dropped once the new one is committed.
    ///
    /// `schema` is passed explicitly so a zero-row dataset still records its
    /// shape. Every batch must match `schema` (the Parquet writer enforces it).
    pub fn put_dataset(
        &mut self,
        name: &str,
        schema: SchemaRef,
        batches: &[RecordBatch],
        compression: Compression,
    ) -> Result<DatasetMeta> {
        let blob = encode_parquet(schema.clone(), batches, compression)?;
        let row_count: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();
        self.store_dataset_blob(name, &schema, blob, row_count, compression, None)
    }

    /// Like [`AlmagestFile::put_dataset`] but consumes a (possibly large) stream of
    /// batches, encoding them into the Parquet blob incrementally so the whole
    /// decoded source never needs to be resident at once — the path the
    /// author-time importers use. `source_json` records provenance (original
    /// path, importer kind, options, timestamp) into `almagest_data.source_json`.
    pub fn put_dataset_streaming(
        &mut self,
        name: &str,
        schema: SchemaRef,
        batches: impl IntoIterator<Item = std::result::Result<RecordBatch, arrow::error::ArrowError>>,
        compression: Compression,
        source_json: Option<&str>,
    ) -> Result<DatasetMeta> {
        let (blob, row_count) = encode_parquet_streaming(schema.clone(), batches, compression)?;
        self.store_dataset_blob(name, &schema, blob, row_count, compression, source_json)
    }

    /// Shared transactional upsert behind [`put_dataset`] and
    /// [`put_dataset_streaming`]: stores an already-encoded Parquet `blob` under
    /// `name`, preserving the row's `id`/`created_at` on replacement.
    fn store_dataset_blob(
        &mut self,
        name: &str,
        schema: &SchemaRef,
        blob: Vec<u8>,
        row_count: u64,
        compression: Compression,
        source_json: Option<&str>,
    ) -> Result<DatasetMeta> {
        if name.trim().is_empty() {
            return Err(AlmagestError::Invalid(
                "dataset name must not be empty".into(),
            ));
        }

        let byte_size = blob.len() as u64;
        let arrow_schema_json = schema_to_json(schema)?;
        let now = crate::now_rfc3339();

        let existing_id: Option<String> = self
            .conn()
            .query_row(
                "SELECT id FROM almagest_data WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;

        let (id, created_at) = match &existing_id {
            Some(id) => {
                let created: String = self.conn().query_row(
                    "SELECT created_at FROM almagest_data WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )?;
                (id.clone(), created)
            }
            None => (uuid::Uuid::new_v4().to_string(), now.clone()),
        };

        let comp_str = compression.as_str().to_string();
        let meta = DatasetMeta {
            id: id.clone(),
            name: name.to_string(),
            row_count,
            byte_size,
            compression: comp_str.clone(),
            arrow_schema_json: arrow_schema_json.clone(),
            source_json: source_json.map(str::to_string),
            created_at: created_at.clone(),
            updated_at: now.clone(),
        };

        self.with_tx(|tx| {
            tx.execute(
                "INSERT INTO almagest_data
                   (id, name, parquet_blob, arrow_schema_json, row_count, byte_size,
                    compression, source_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    parquet_blob = excluded.parquet_blob,
                    arrow_schema_json = excluded.arrow_schema_json,
                    row_count = excluded.row_count,
                    byte_size = excluded.byte_size,
                    compression = excluded.compression,
                    source_json = excluded.source_json,
                    updated_at = excluded.updated_at",
                rusqlite::params![
                    id,
                    name,
                    blob,
                    arrow_schema_json,
                    row_count as i64,
                    byte_size as i64,
                    comp_str,
                    source_json,
                    created_at,
                    now,
                ],
            )?;
            Ok(())
        })?;

        Ok(meta)
    }

    /// Read a dataset back into Arrow record batches, decoding the Parquet blob.
    /// Round-trips losslessly with [`AlmagestFile::put_dataset`].
    pub fn read_dataset(&self, name: &str) -> Result<Vec<RecordBatch>> {
        Ok(self.read_dataset_arrow(name)?.1)
    }

    /// Like [`AlmagestFile::read_dataset`] but also returns the Arrow schema. The
    /// schema comes from the Parquet metadata, so a zero-row dataset still
    /// reports its shape (which [`AlmagestFile::read_dataset`] alone can't, having
    /// no batch to read it from). This is what the query layer registers a
    /// `MemTable` with.
    pub fn read_dataset_arrow(&self, name: &str) -> Result<(SchemaRef, Vec<RecordBatch>)> {
        let blob: Vec<u8> = self
            .conn()
            .query_row(
                "SELECT parquet_blob FROM almagest_data WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => AlmagestError::NotFound {
                    kind: "dataset",
                    id: name.to_string(),
                },
                other => AlmagestError::Sqlite(other),
            })?;
        decode_parquet(blob)
    }

    /// Fetch a dataset's bookkeeping metadata without decoding its blob.
    pub fn dataset_meta(&self, name: &str) -> Result<DatasetMeta> {
        self.conn()
            .query_row(
                "SELECT id, name, row_count, byte_size, compression, arrow_schema_json,
                        source_json, created_at, updated_at
                 FROM almagest_data WHERE name = ?1",
                [name],
                row_to_meta,
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => AlmagestError::NotFound {
                    kind: "dataset",
                    id: name.to_string(),
                },
                other => AlmagestError::Sqlite(other),
            })
    }

    /// List all embedded datasets (metadata only), ordered by name.
    pub fn list_datasets(&self) -> Result<Vec<DatasetMeta>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, row_count, byte_size, compression, arrow_schema_json,
                    source_json, created_at, updated_at
             FROM almagest_data ORDER BY name",
        )?;
        let rows = stmt.query_map([], row_to_meta)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Remove a dataset. Returns `true` if a dataset was removed.
    pub fn remove_dataset(&mut self, name: &str) -> Result<bool> {
        let n = self
            .conn()
            .execute("DELETE FROM almagest_data WHERE name = ?1", [name])?;
        Ok(n > 0)
    }

    /// Export a dataset's stored blob to a standalone `.parquet` file on disk.
    ///
    /// The bytes are written verbatim — the blob already *is* a Parquet file —
    /// so the output is readable by any Parquet tool, demonstrating portability.
    pub fn export_dataset_parquet(&self, name: &str, out_path: impl AsRef<Path>) -> Result<()> {
        let blob: Vec<u8> = self
            .conn()
            .query_row(
                "SELECT parquet_blob FROM almagest_data WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => AlmagestError::NotFound {
                    kind: "dataset",
                    id: name.to_string(),
                },
                other => AlmagestError::Sqlite(other),
            })?;
        std::fs::write(out_path, blob)?;
        Ok(())
    }
}

/// Map a `almagest_data` row (sans blob) to [`DatasetMeta`].
fn row_to_meta(r: &rusqlite::Row<'_>) -> rusqlite::Result<DatasetMeta> {
    Ok(DatasetMeta {
        id: r.get(0)?,
        name: r.get(1)?,
        row_count: r.get::<_, i64>(2)? as u64,
        byte_size: r.get::<_, i64>(3)? as u64,
        compression: r.get(4)?,
        arrow_schema_json: r.get(5)?,
        source_json: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

/// Encode Arrow batches into a single compressed Parquet file in memory.
fn encode_parquet(
    schema: SchemaRef,
    batches: &[RecordBatch],
    compression: Compression,
) -> Result<Vec<u8>> {
    let props = WriterProperties::builder()
        .set_compression(compression.to_parquet())
        .build();
    let mut buf: Vec<u8> = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut buf, schema, Some(props))?;
    for batch in batches {
        writer.write(batch)?;
    }
    writer.close()?;
    Ok(buf)
}

/// Encode a stream of batches into a compressed Parquet file, returning the
/// bytes and the total row count. Batches are written one at a time, so the full
/// set is never held in memory at once.
fn encode_parquet_streaming(
    schema: SchemaRef,
    batches: impl IntoIterator<Item = std::result::Result<RecordBatch, arrow::error::ArrowError>>,
    compression: Compression,
) -> Result<(Vec<u8>, u64)> {
    let props = WriterProperties::builder()
        .set_compression(compression.to_parquet())
        .build();
    let mut buf: Vec<u8> = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut buf, schema, Some(props))?;
    let mut row_count: u64 = 0;
    for batch in batches {
        let batch = batch?;
        row_count += batch.num_rows() as u64;
        writer.write(&batch)?;
    }
    writer.close()?;
    Ok((buf, row_count))
}

/// Decode an in-memory Parquet file back into its Arrow schema and batches. The
/// schema is taken from the Parquet metadata, so it is correct even when there
/// are zero batches.
fn decode_parquet(blob: Vec<u8>) -> Result<(SchemaRef, Vec<RecordBatch>)> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(Bytes::from(blob))?;
    let schema = builder.schema().clone();
    let reader = builder.build()?;
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch?);
    }
    Ok((schema, batches))
}

/// Serialize an Arrow schema to a compact JSON description (field name, type,
/// nullability). Used for fast introspection without decoding the blob; the
/// Parquet blob's embedded schema remains the source of truth on read.
fn schema_to_json(schema: &SchemaRef) -> Result<String> {
    let fields: Vec<serde_json::Value> = schema
        .fields()
        .iter()
        .map(|f| {
            serde_json::json!({
                "name": f.name(),
                "data_type": format!("{}", f.data_type()),
                "nullable": f.is_nullable(),
            })
        })
        .collect();
    let doc = serde_json::json!({ "fields": fields });
    Ok(serde_json::to_string(&doc)?)
}

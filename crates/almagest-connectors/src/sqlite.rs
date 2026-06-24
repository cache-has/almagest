// SPDX-License-Identifier: MIT OR Apache-2.0

//! SQLite importer — bake one or more tables from an existing SQLite database
//! file (distinct from the `.alm` container itself) into `almagest_data`.
//!
//! SQLite is dynamically typed, so each column's Arrow type is decided from its
//! declared affinity (the SQLite rules), and any cell that doesn't fit that type
//! is stored as null with a counted warning ("widen-to-string"-style leniency)
//! rather than failing the import.

use crate::error::{ImportError, ImportReport, Result};
use crate::framework::{ImportedDataset, Importer, check_collision, provenance};
use almagest_core::{AlmagestFile, Compression};
use arrow::array::{
    ArrayRef, BinaryBuilder, Float64Builder, Int64Builder, RecordBatch, StringBuilder,
};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;
use std::sync::Arc;

/// Options for importing tables from a SQLite database file.
#[derive(Debug, Clone)]
pub struct SqliteOptions {
    /// Absolute path to the source `.db` / `.sqlite` file.
    pub path: PathBuf,
    /// Tables to import; `None` imports all user tables.
    pub tables: Option<Vec<String>>,
    /// Prefix prepended to each dataset name (to avoid `almagest_data` clashes).
    pub name_prefix: String,
    /// Overwrite existing datasets of the same name instead of erroring.
    pub replace: bool,
}

impl SqliteOptions {
    /// Default options importing all user tables from `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            tables: None,
            name_prefix: String::new(),
            replace: false,
        }
    }
}

/// SQLite importer.
pub struct SqliteImporter {
    opts: SqliteOptions,
}

impl SqliteImporter {
    /// Build the importer from options.
    pub fn new(opts: SqliteOptions) -> Self {
        Self { opts }
    }
}

impl Importer for SqliteImporter {
    fn import(&self, file: &mut AlmagestFile) -> Result<Vec<ImportedDataset>> {
        let path = &self.opts.path;
        if !path.exists() {
            return Err(ImportError::SourceNotFound {
                path: path.display().to_string(),
            });
        }

        let conn =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| {
                ImportError::SourceUnreadable {
                    path: path.display().to_string(),
                    detail: e.to_string(),
                }
            })?;

        let tables = self.resolve_tables(&conn)?;
        if tables.is_empty() {
            return Err(ImportError::EmptySource {
                path: path.display().to_string(),
            });
        }

        let mut out = Vec::with_capacity(tables.len());
        for table in tables {
            let name = format!("{}{}", self.opts.name_prefix, table);
            check_collision(file, &name, self.opts.replace)?;

            let (schema, batch, mut report) = read_table(&conn, &table)?;
            let source_json = provenance(
                "sqlite",
                path,
                serde_json::json!({ "table": table, "name_prefix": self.opts.name_prefix }),
            );
            let meta = file.put_dataset_streaming(
                &name,
                schema,
                std::iter::once(Ok(batch)),
                Compression::Zstd,
                Some(&source_json),
            )?;
            report.rows_imported = meta.row_count;
            out.push(ImportedDataset { name, meta, report });
        }
        Ok(out)
    }
}

impl SqliteImporter {
    /// Resolve the list of tables to import: the requested ones, or every user
    /// table (excluding `sqlite_*` internals) when none are specified.
    fn resolve_tables(&self, conn: &Connection) -> Result<Vec<String>> {
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             ORDER BY name",
        )?;
        let all: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        match &self.opts.tables {
            None => Ok(all),
            Some(requested) => {
                for t in requested {
                    if !all.contains(t) {
                        return Err(ImportError::SourceUnreadable {
                            path: self.opts.path.display().to_string(),
                            detail: format!("table '{t}' not found in source database"),
                        });
                    }
                }
                Ok(requested.clone())
            }
        }
    }
}

/// The Arrow column type chosen for a SQLite column.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColType {
    Int,
    Real,
    Text,
    Binary,
}

impl ColType {
    fn arrow(self) -> DataType {
        match self {
            ColType::Int => DataType::Int64,
            ColType::Real => DataType::Float64,
            ColType::Text => DataType::Utf8,
            ColType::Binary => DataType::Binary,
        }
    }
}

/// Map a declared SQLite type to a column type using SQLite's affinity rules,
/// widening an undeclared type to text.
fn affinity(declared: &str) -> ColType {
    let d = declared.to_ascii_uppercase();
    if d.is_empty() {
        ColType::Text
    } else if d.contains("INT") {
        ColType::Int
    } else if d.contains("CHAR") || d.contains("CLOB") || d.contains("TEXT") {
        ColType::Text
    } else if d.contains("BLOB") {
        ColType::Binary
    } else {
        // REAL/FLOA/DOUB and the NUMERIC fallback both map to a float column.
        ColType::Real
    }
}

/// A growable Arrow column builder for one of the four supported types.
enum ColBuilder {
    Int(Int64Builder),
    Real(Float64Builder),
    Text(StringBuilder),
    Binary(BinaryBuilder),
}

impl ColBuilder {
    fn for_type(t: ColType) -> Self {
        match t {
            ColType::Int => ColBuilder::Int(Int64Builder::new()),
            ColType::Real => ColBuilder::Real(Float64Builder::new()),
            ColType::Text => ColBuilder::Text(StringBuilder::new()),
            ColType::Binary => ColBuilder::Binary(BinaryBuilder::new()),
        }
    }

    /// Append one cell, returning `true` if the value had to be coerced to null
    /// because it didn't match the column type.
    fn append(&mut self, v: ValueRef<'_>) -> bool {
        match self {
            ColBuilder::Int(b) => match v {
                ValueRef::Null => {
                    b.append_null();
                    false
                }
                ValueRef::Integer(i) => {
                    b.append_value(i);
                    false
                }
                _ => {
                    b.append_null();
                    true
                }
            },
            ColBuilder::Real(b) => match v {
                ValueRef::Null => {
                    b.append_null();
                    false
                }
                ValueRef::Real(f) => {
                    b.append_value(f);
                    false
                }
                ValueRef::Integer(i) => {
                    b.append_value(i as f64);
                    false
                }
                _ => {
                    b.append_null();
                    true
                }
            },
            ColBuilder::Text(b) => match v {
                ValueRef::Null => {
                    b.append_null();
                    false
                }
                ValueRef::Text(t) => {
                    b.append_value(String::from_utf8_lossy(t));
                    false
                }
                ValueRef::Integer(i) => {
                    b.append_value(i.to_string());
                    false
                }
                ValueRef::Real(f) => {
                    b.append_value(f.to_string());
                    false
                }
                ValueRef::Blob(_) => {
                    b.append_null();
                    true
                }
            },
            ColBuilder::Binary(b) => match v {
                ValueRef::Null => {
                    b.append_null();
                    false
                }
                ValueRef::Blob(bytes) => {
                    b.append_value(bytes);
                    false
                }
                _ => {
                    b.append_null();
                    true
                }
            },
        }
    }

    fn finish(self) -> ArrayRef {
        match self {
            ColBuilder::Int(mut b) => Arc::new(b.finish()),
            ColBuilder::Real(mut b) => Arc::new(b.finish()),
            ColBuilder::Text(mut b) => Arc::new(b.finish()),
            ColBuilder::Binary(mut b) => Arc::new(b.finish()),
        }
    }
}

/// Read a whole table into a single Arrow batch (v1 materializes per table;
/// chunked streaming is a later optimization).
fn read_table(conn: &Connection, table: &str) -> Result<(SchemaRef, RecordBatch, ImportReport)> {
    // Column names + declared types, in definition order.
    let mut info = conn.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let cols: Vec<(String, ColType)> = info
        .query_map([], |r| {
            let name: String = r.get(1)?;
            let decl: String = r.get(2)?;
            Ok((name, affinity(&decl)))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    if cols.is_empty() {
        return Err(ImportError::SchemaInferenceFailed {
            detail: format!("table '{table}' has no columns"),
        });
    }

    let fields: Vec<Field> = cols
        .iter()
        .map(|(n, t)| Field::new(n, t.arrow(), true))
        .collect();
    let schema: SchemaRef = Arc::new(Schema::new(fields));

    let col_list = cols
        .iter()
        .map(|(n, _)| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut builders: Vec<ColBuilder> =
        cols.iter().map(|(_, t)| ColBuilder::for_type(*t)).collect();

    let mut stmt = conn.prepare(&format!("SELECT {col_list} FROM \"{table}\""))?;
    let mut rows = stmt.query([])?;
    let mut coercions: u64 = 0;
    while let Some(row) = rows.next()? {
        for (i, builder) in builders.iter_mut().enumerate() {
            if builder.append(row.get_ref(i)?) {
                coercions += 1;
            }
        }
    }

    let arrays: Vec<ArrayRef> = builders.into_iter().map(ColBuilder::finish).collect();
    let batch = RecordBatch::try_new(schema.clone(), arrays)?;

    let mut report = ImportReport::default();
    if coercions > 0 {
        report.warn(format!(
            "{coercions} cell(s) did not match their column type and were stored as null"
        ));
    }
    Ok((schema, batch, report))
}

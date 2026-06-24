// SPDX-License-Identifier: MIT OR Apache-2.0

//! Query result and schema-introspection types.

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use std::time::Duration;

/// The result of running a query: Arrow batches plus reporting metadata. The
/// dashboard layer consumes `batches` uniformly regardless of which panel ran
/// the query.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Schema of the result set (present even when there are zero rows).
    pub schema: SchemaRef,
    /// Result rows as Arrow record batches.
    pub batches: Vec<RecordBatch>,
    /// Total number of rows across all batches.
    pub row_count: u64,
    /// Wall-clock time to produce the result (≈0 on a cache hit).
    pub execution_time: Duration,
    /// Whether the result was served from the cache.
    pub cached: bool,
}

impl QueryResult {
    /// Sum of rows across `batches`.
    pub(crate) fn count_rows(batches: &[RecordBatch]) -> u64 {
        batches.iter().map(|b| b.num_rows() as u64).sum()
    }
}

/// One registered table's name and column shape.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TableSchema {
    /// Table name queries reference.
    pub name: String,
    /// Columns as `(name, arrow_type)` pairs.
    pub columns: Vec<ColumnSchema>,
    /// Row count of the registered dataset.
    pub row_count: u64,
}

/// One column's name and Arrow type (as a display string).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ColumnSchema {
    /// Column name.
    pub name: String,
    /// Arrow data type, rendered for display (e.g. `Int64`, `Utf8`).
    pub data_type: String,
    /// Whether the column is nullable.
    pub nullable: bool,
}

/// The full set of registered tables in a query context, for the editor and
/// autocompletion.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DatabaseSchema {
    /// Every registered table, ordered by name.
    pub tables: Vec<TableSchema>,
}

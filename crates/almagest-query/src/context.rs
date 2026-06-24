// SPDX-License-Identifier: MIT OR Apache-2.0

//! [`AlmagestQueryContext`] — one open file's queryable engine.
//!
//! On open, every `almagest_data` blob is decoded to Arrow and registered as a
//! DataFusion `MemTable` under its `name`. Panels then run SQL against that
//! single in-process context — no live connections, no backend multiplexing.
//! Because all datasets share one context, cross-dataset joins just work.

use crate::cache::{AlmagestCache, DEFAULT_MAX_BYTES, DEFAULT_TTL_SECONDS};
use crate::error::Result;
use crate::params::{QueryParams, substitute};
use crate::result::{ColumnSchema, DatabaseSchema, QueryResult, TableSchema};
use almagest_core::AlmagestFile;
use arrow::datatypes::SchemaRef;
use datafusion::common::TableReference;
use datafusion::datasource::MemTable;
use datafusion::prelude::SessionContext;
use std::sync::Arc;
use std::time::Instant;

/// Options controlling how a [`AlmagestQueryContext`] is built.
#[derive(Debug, Clone)]
pub struct ContextOptions {
    /// Per-dataset in-memory budget. A dataset whose stored blob exceeds this
    /// would ideally register as a streaming on-disk Parquet scan; that path is
    /// deferred, so for now an over-budget dataset is still loaded into a
    /// `MemTable` and a warning is logged.
    pub in_memory_budget_bytes: u64,
    /// Cache entry TTL in seconds.
    pub cache_ttl_seconds: i64,
    /// Cache size budget in bytes before eviction.
    pub cache_max_bytes: u64,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            // 1 GiB: comfortably above the 100k–1M-row target, so the MemTable
            // path is the norm until the streaming path lands.
            in_memory_budget_bytes: 1024 * 1024 * 1024,
            cache_ttl_seconds: DEFAULT_TTL_SECONDS,
            cache_max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

/// One open Almagest file's query engine: a DataFusion context plus a result cache.
pub struct AlmagestQueryContext {
    ctx: SessionContext,
    tables: Vec<TableSchema>,
    cache: AlmagestCache,
}

impl AlmagestQueryContext {
    /// Build a context for `file` with default options.
    pub fn open(file: &AlmagestFile) -> Result<Self> {
        Self::open_with(file, &ContextOptions::default())
    }

    /// Build a context for `file` with explicit options.
    pub fn open_with(file: &AlmagestFile, options: &ContextOptions) -> Result<Self> {
        let ctx = SessionContext::new();
        let metas = file.list_datasets()?;
        let data_version = data_version_fingerprint(&metas);
        let mut tables = Vec::with_capacity(metas.len());

        for meta in &metas {
            let (schema, batches) = file.read_dataset_arrow(&meta.name)?;

            if meta.byte_size > options.in_memory_budget_bytes {
                tracing::warn!(
                    dataset = %meta.name,
                    byte_size = meta.byte_size,
                    budget = options.in_memory_budget_bytes,
                    "dataset exceeds in-memory budget; loading into RAM anyway \
                     (streaming Parquet path not yet implemented)"
                );
            }

            let table_schema = TableSchema {
                name: meta.name.clone(),
                columns: schema
                    .fields()
                    .iter()
                    .map(|f| ColumnSchema {
                        name: f.name().clone(),
                        data_type: format!("{}", f.data_type()),
                        nullable: f.is_nullable(),
                    })
                    .collect(),
                row_count: meta.row_count,
            };

            let mem = MemTable::try_new(schema, vec![batches])?;
            ctx.register_table(TableReference::bare(meta.name.clone()), Arc::new(mem))?;
            tables.push(table_schema);
        }

        let cache = AlmagestCache::open(
            file.path(),
            data_version,
            options.cache_ttl_seconds,
            options.cache_max_bytes,
        )?;

        Ok(Self { ctx, tables, cache })
    }

    /// Run a query against the registered tables. Parameters are substituted
    /// safely, the result cache is consulted first, and a miss executes through
    /// DataFusion and populates the cache.
    pub async fn execute(&self, sql: &str, params: &QueryParams) -> Result<QueryResult> {
        let start = Instant::now();
        let final_sql = substitute(sql, params)?;
        let key = self.cache.key(&final_sql);

        if let Some((schema, batches)) = self.cache.get(&key)? {
            let row_count = QueryResult::count_rows(&batches);
            return Ok(QueryResult {
                schema,
                batches,
                row_count,
                execution_time: start.elapsed(),
                cached: true,
            });
        }

        let df = self.ctx.sql(&final_sql).await?;
        let schema: SchemaRef = Arc::new(df.schema().as_arrow().clone());
        let batches = df.collect().await?;

        // Best-effort cache write — a cache failure must not fail the query.
        if let Err(e) = self.cache.put(&key, &schema, &batches) {
            tracing::warn!(error = %e, "failed to write query result to cache");
        }

        let row_count = QueryResult::count_rows(&batches);
        Ok(QueryResult {
            schema,
            batches,
            row_count,
            execution_time: start.elapsed(),
            cached: false,
        })
    }

    /// Resolve a parameter `options_query` (doc 07): run it and return the
    /// distinct values of its **first column** as display strings, in
    /// first-seen order with nulls skipped. This populates `select` /
    /// `multiselect` choices at dashboard load. The first column is cast to
    /// UTF-8 so numeric/date option columns work without the author having to
    /// `CAST` in SQL.
    pub async fn resolve_options(&self, sql: &str) -> Result<Vec<String>> {
        use arrow::array::{Array, StringArray};
        use arrow::datatypes::DataType;

        let res = self.execute(sql, &QueryParams::empty()).await?;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for batch in &res.batches {
            if batch.num_columns() == 0 {
                continue;
            }
            let utf8 = arrow::compute::cast(batch.column(0), &DataType::Utf8)?;
            let arr = utf8
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("cast to Utf8 yields a StringArray");
            for i in 0..arr.len() {
                if arr.is_null(i) {
                    continue;
                }
                let v = arr.value(i).to_string();
                if seen.insert(v.clone()) {
                    out.push(v);
                }
            }
        }
        Ok(out)
    }

    /// Introspect the schema of every registered table (for the editor /
    /// autocompletion).
    pub fn schema(&self) -> DatabaseSchema {
        DatabaseSchema {
            tables: self.tables.clone(),
        }
    }

    /// Drop all cached results (e.g. when the caller knows the data changed).
    pub fn invalidate_cache(&self) -> Result<()> {
        self.cache.invalidate_all()
    }
}

/// A compact fingerprint of the dataset set, folded into cache keys so that
/// re-baking any dataset invalidates stale cache entries. `metas` arrives
/// already ordered by name from `list_datasets`.
fn data_version_fingerprint(metas: &[almagest_core::DatasetMeta]) -> String {
    let mut s = String::new();
    for m in metas {
        s.push_str(&format!(
            "{}:{}:{}:{};",
            m.name, m.updated_at, m.byte_size, m.row_count
        ));
    }
    s
}

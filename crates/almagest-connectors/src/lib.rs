// SPDX-License-Identifier: MIT OR Apache-2.0

//! # almagest-connectors
//!
//! **Author-time importers.** Each importer reads a local file once — CSV,
//! Parquet, JSON/NDJSON, or an existing SQLite database — converts it to Arrow,
//! and bakes it into the `.alm` as a compressed Parquet blob in `almagest_data`
//! (with a `source_json` provenance record). After import the data is fully
//! embedded; it is queried by the in-process DataFusion engine ([`almagest_query`]).
//!
//! This is **import**, not **connect**: there are no live database / REST /
//! remote-file connectors in Almagest. Anything touching a live, large, or
//! scheduled source belongs to Armillary, which bakes a `.alm` via its sink
//! plugin. (The crate keeps its `almagest-connectors` name for historical reasons;
//! its job is author-time *import*.)
//!
//! [`almagest_query`]: https://docs.rs/almagest-query

mod csv;
mod error;
mod framework;
mod json;
mod parquet;
mod sqlite;

pub use csv::{CsvImporter, CsvOptions};
pub use error::{ImportError, ImportReport, Result};
pub use framework::{ImportedDataset, Importer};
pub use json::{JsonFormat, JsonImporter, JsonMode, JsonOptions};
pub use parquet::{ParquetImporter, ParquetOptions};
pub use sqlite::{SqliteImporter, SqliteOptions};

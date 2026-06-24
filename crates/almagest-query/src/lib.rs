// SPDX-License-Identifier: MIT OR Apache-2.0

//! # almagest-query
//!
//! The query engine layer. Almagest is **embedded-only**: on open, each
//! `almagest_data` Parquet blob decodes to an in-memory Arrow/DataFusion
//! `MemTable` and is registered by name, and all queries run locally through
//! **DataFusion** (pure Rust) — no live connections, no external drivers. Adds
//! safe `{{param}}` substitution and a result cache on top.
//!
//! Entry point: [`AlmagestQueryContext::open`] over an open [`almagest_core::AlmagestFile`].

mod cache;
mod context;
mod error;
mod params;
mod resolve;
mod result;
mod urlstate;

pub use cache::{AlmagestCache, DEFAULT_MAX_BYTES, DEFAULT_TTL_SECONDS};
pub use context::{AlmagestQueryContext, ContextOptions};
pub use error::{QueryError, Result};
pub use params::{ParamDecl, ParamSchema, ParamType, ParamValue, QueryParams, substitute};
pub use resolve::{
    ALL_SENTINEL, interpolate_action_value, resolve_daterange_preset, resolve_parameters,
};
pub use result::{ColumnSchema, DatabaseSchema, QueryResult, TableSchema};
pub use urlstate::{decode_url_state, encode_url_state, layered_state};

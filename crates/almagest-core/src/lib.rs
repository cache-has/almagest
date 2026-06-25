// SPDX-License-Identifier: MIT OR Apache-2.0

//! # almagest-core
//!
//! The heart of Almagest: the `.alm` file format. A `.alm` file *is* a SQLite
//! database with a defined schema (`almagest_*` tables). This crate owns the
//! schema, migrations, versioning, and the [`AlmagestFile`] handle that the rest
//! of the system builds on.
//!
//! Almagest's thesis: **dashboards should be files, not services.** Everything a
//! dashboard needs — connections, queries, layout, cached results, assets —
//! lives inside one self-contained, portable file.

mod assets;
mod auth;
mod dashboard;
mod dashboards;
mod data;
mod error;
mod file;
mod history;
mod metadata;
mod migrations;
mod queries;
mod schema;

pub use almagest_format::{DurationUnit, Format, FormatValue};
pub use assets::Asset;
pub use auth::{AuthConfig, Role, User};
pub use dashboard::{
    Action, ChartConfig, ChartSort, ChartType, ColumnConfig, Comparison, DASHBOARD_DSL_VERSION,
    Dashboard, DeltaFormat, DividerConfig, ImageConfig, Layout, MetricConfig, Orientation, Panel,
    PanelKind, ParamKind, Parameter, Persist, Query, Row, SortDirection, SortSpec, TableConfig,
    TextConfig, Theme, TrendDirection, Visibility, VisibilityEquals,
};
pub use dashboards::DashboardRecord;
pub use data::{Compression, DatasetMeta};
pub use error::{AlmagestError, Result};
pub use file::AlmagestFile;
pub use history::{HistoryEntry, HistoryFilter};
pub use queries::SavedQuery;

/// Current UTC timestamp as an RFC 3339 string — the one timestamp format used
/// throughout the `almagest_*` tables.
pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// The `.alm` format version this build reads and writes.
///
/// The format is **unstable** until Almagest 1.0 ships; breaking changes may
/// occur between 0.x minor versions, each bumping this number. After 1.0 the
/// format is stable and breaking changes require a major version bump.
///
/// - v1: initial schema (format / data / queries / dashboards / cache / history
///   / users / assets / secrets).
/// - v2: auth & multi-user (doc 13) — `almagest_users` gains `email` /
///   `last_login_at`, plus an `almagest_auth` single-row config table. A v1 file
///   upgrades transparently on open (no auth until an admin enables it).
pub const FORMAT_VERSION: u32 = 2;

/// The crate (and binary) semantic version, sourced from Cargo at build time.
pub const ALMAGEST_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The bundled SQLite library version (e.g. `"3.45.0"`), for `almagest doctor`.
pub fn sqlite_version() -> &'static str {
    rusqlite::version()
}

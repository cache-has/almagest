// SPDX-License-Identifier: MIT OR Apache-2.0

//! Core error type for Almagest file operations.

use std::path::PathBuf;

/// Result alias used throughout `almagest-core`.
pub type Result<T> = std::result::Result<T, AlmagestError>;

/// Errors that can arise opening, creating, or operating on a `.alm` file.
#[derive(Debug, thiserror::Error)]
pub enum AlmagestError {
    /// The underlying SQLite layer returned an error.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// I/O error touching the file on disk.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The file's format version is newer than this build understands.
    #[error(
        "almagest file format version {found} is newer than supported version {supported}; \
         upgrade Almagest to open this file"
    )]
    FormatTooNew { found: u32, supported: u32 },

    /// The file exists but is not a valid Almagest file.
    #[error("{path} is not a valid almagest file: {reason}")]
    NotAAlmagestFile { path: PathBuf, reason: String },

    /// JSON (de)serialization of a stored definition failed.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// Encoding or decoding a Parquet blob failed.
    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),

    /// An Arrow operation (schema, record batch) failed.
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    /// The file's on-open integrity check found a problem.
    #[error("almagest file failed integrity check: {0}")]
    Integrity(String),

    /// A requested entity (dataset, dashboard, asset, …) was not found.
    #[error("{kind} '{id}' not found")]
    NotFound {
        /// What kind of entity was looked up (e.g. "dataset", "dashboard").
        kind: &'static str,
        /// The identifier or name that was looked up.
        id: String,
    },

    /// A write was rejected because it would violate a format invariant.
    #[error("invalid almagest operation: {0}")]
    Invalid(String),

    /// A dashboard definition was structurally or semantically invalid. The
    /// message points at the offending field.
    #[error("invalid dashboard ({location}): {detail}")]
    InvalidDashboard {
        /// Dotted path to the offending field (e.g. `layout.rows[0].panels[2].span`).
        location: String,
        /// What was wrong.
        detail: String,
    },
}

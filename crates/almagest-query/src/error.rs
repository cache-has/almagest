// SPDX-License-Identifier: MIT OR Apache-2.0

//! Error type for the query engine layer.

/// Result alias for `almagest-query`.
pub type Result<T> = std::result::Result<T, QueryError>;

/// Errors that can arise registering data, validating parameters, or running a
/// query. Variants are shaped to map cleanly onto panel-renderable messages.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    /// A `almagest-core` operation failed (reading a dataset, opening the file).
    #[error(transparent)]
    Core(#[from] almagest_core::AlmagestError),

    /// DataFusion failed to plan or execute the query. Rendered with a friendly
    /// prefix for panels; the underlying message is preserved.
    #[error("query failed: {0}")]
    DataFusion(String),

    /// An Arrow operation (cache encode/decode, schema) failed.
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    /// The cache's SQLite layer returned an error.
    #[error("cache error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A parameter was missing, the wrong type, or out of its declared bounds.
    /// Carries a human message safe to show next to the panel.
    #[error("parameter error: {0}")]
    Param(String),

    /// The SQL referenced a `{{name}}` with no matching parameter value.
    #[error("unbound parameter '{0}' in query")]
    UnboundParam(String),
}

impl From<datafusion::error::DataFusionError> for QueryError {
    fn from(e: datafusion::error::DataFusionError) -> Self {
        // Collapse DataFusion's layered errors to their root message — that's
        // the part a dashboard author can act on (bad column, syntax, etc.).
        QueryError::DataFusion(e.to_string())
    }
}

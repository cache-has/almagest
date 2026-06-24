// SPDX-License-Identifier: MIT OR Apache-2.0

//! Import error taxonomy and the lenient-mode report.

/// Result alias for the importers.
pub type Result<T> = std::result::Result<T, ImportError>;

/// A uniform error taxonomy so Studio and the CLI report import failures
/// consistently. Every variant names the actionable cause.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The source path does not exist.
    #[error("source file not found: {path}")]
    SourceNotFound {
        /// The path that was looked for.
        path: String,
    },

    /// The source exists but could not be opened or read.
    #[error("source file unreadable ({path}): {detail}")]
    SourceUnreadable {
        /// The source path.
        path: String,
        /// Why it could not be read.
        detail: String,
    },

    /// The source isn't in a format this importer understands.
    #[error("unsupported format: {detail}")]
    UnsupportedFormat {
        /// What was wrong.
        detail: String,
    },

    /// Schema inference failed (bad CSV header, unreadable JSON, etc.).
    #[error("schema inference failed: {detail}")]
    SchemaInferenceFailed {
        /// What went wrong inferring the schema.
        detail: String,
    },

    /// A record could not be parsed in strict mode.
    #[error("malformed record at row {row}: {detail}")]
    MalformedRecord {
        /// 0-based row index of the bad record.
        row: u64,
        /// Why it was rejected.
        detail: String,
    },

    /// The source contained no rows / no importable tables.
    #[error("source is empty: {path}")]
    EmptySource {
        /// The source path.
        path: String,
    },

    /// The target dataset name is already present and `replace` was not set.
    #[error("dataset name '{name}' already exists in the almagest (set replace to overwrite)")]
    NameCollision {
        /// The colliding dataset name.
        name: String,
    },

    /// Writing the Parquet blob into `almagest_data` failed.
    #[error("write failed: {0}")]
    WriteFailed(#[from] almagest_core::AlmagestError),

    /// An Arrow-level error while reading the source.
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    /// A SQLite error while reading a source `.db` file.
    #[error("sqlite read error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// What a lenient import did: how many rows landed, how many were skipped, and
/// any non-fatal warnings (type coercions, dropped records).
#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    /// Rows successfully written.
    pub rows_imported: u64,
    /// Rows skipped (lenient mode only).
    pub rows_skipped: u64,
    /// Human-readable non-fatal notes.
    pub warnings: Vec<String>,
}

impl ImportReport {
    /// A clean report for `rows_imported` rows with nothing skipped.
    pub fn clean(rows_imported: u64) -> Self {
        Self {
            rows_imported,
            rows_skipped: 0,
            warnings: Vec::new(),
        }
    }

    /// Record a non-fatal warning.
    pub fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }
}

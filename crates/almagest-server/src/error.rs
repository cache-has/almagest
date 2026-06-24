// SPDX-License-Identifier: MIT OR Apache-2.0

//! API error type and its HTTP/JSON representation.
//!
//! Every handler returns [`ApiResult`]; an error renders as a structured JSON
//! body (`{ "error": { "code", "message" } }`) with a status code that follows
//! the doc-08 REST conventions. Lower-layer errors ([`AlmagestError`],
//! [`QueryError`]) map into this type so handlers can use `?` freely.

use almagest_connectors::ImportError;
use almagest_core::AlmagestError;
use almagest_query::QueryError;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// The result type every API handler returns.
pub type ApiResult<T> = Result<T, ApiError>;

/// A handler error carrying an HTTP status, a stable machine code, and a
/// human-readable message.
#[derive(Debug)]
pub struct ApiError {
    /// HTTP status to return.
    pub status: StatusCode,
    /// Stable, machine-readable error code (e.g. `not_found`, `bad_request`).
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
}

impl ApiError {
    /// Construct an error from its parts.
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    /// 400 Bad Request — malformed input or a validation failure.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    /// 404 Not Found — the addressed entity does not exist.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    /// 500 Internal Server Error — an unexpected Almagest-side failure.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }
}

/// The JSON envelope: `{ "error": { "code", "message" } }`.
#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // Log server-side faults; client faults (4xx) are expected and quiet.
        if self.status.is_server_error() {
            tracing::error!(code = self.code, message = %self.message, "api error");
        } else {
            tracing::debug!(code = self.code, message = %self.message, "api client error");
        }
        let body = ErrorEnvelope {
            error: ErrorBody {
                code: self.code,
                message: self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

impl From<AlmagestError> for ApiError {
    fn from(e: AlmagestError) -> Self {
        match e {
            AlmagestError::NotFound { kind, id } => {
                ApiError::not_found(format!("{kind} '{id}' not found"))
            }
            AlmagestError::InvalidDashboard { .. } | AlmagestError::Invalid(_) => {
                ApiError::bad_request(e.to_string())
            }
            AlmagestError::Serde(_) => ApiError::bad_request(e.to_string()),
            other => ApiError::internal(other.to_string()),
        }
    }
}

impl From<ImportError> for ApiError {
    fn from(e: ImportError) -> Self {
        match e {
            // The caller's upload is at fault: bad/empty/unsupported source, a
            // name clash, or a malformed record.
            ImportError::SourceNotFound { .. }
            | ImportError::SourceUnreadable { .. }
            | ImportError::UnsupportedFormat { .. }
            | ImportError::SchemaInferenceFailed { .. }
            | ImportError::MalformedRecord { .. }
            | ImportError::EmptySource { .. }
            | ImportError::NameCollision { .. } => ApiError::bad_request(e.to_string()),
            ImportError::WriteFailed(inner) => inner.into(),
            other => ApiError::internal(other.to_string()),
        }
    }
}

impl From<QueryError> for ApiError {
    fn from(e: QueryError) -> Self {
        match e {
            QueryError::Core(inner) => inner.into(),
            // A bad parameter value or a malformed query is the caller's fault.
            QueryError::Param(_) | QueryError::UnboundParam(_) | QueryError::DataFusion(_) => {
                ApiError::bad_request(e.to_string())
            }
            other => ApiError::internal(other.to_string()),
        }
    }
}

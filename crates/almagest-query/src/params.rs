// SPDX-License-Identifier: MIT OR Apache-2.0

//! Typed query parameters and safe `{{name}}` substitution.
//!
//! Substitution is **never** naive string replacement. Each value is validated
//! against its declared type and bounds, then rendered as a properly-typed,
//! escaped SQL literal. A malformed parameter value can change a *value* in the
//! query but can never change the query's *structure* — strings are quote-
//! escaped, dates are validated to `YYYY-MM-DD`, numbers/booleans render as bare
//! literals. This is the threat-model guarantee the doc calls for.

use crate::error::{QueryError, Result};
use std::collections::HashMap;

/// The declared type of a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    /// UTF-8 text, rendered as a quote-escaped SQL string literal.
    String,
    /// 64-bit signed integer.
    Integer,
    /// 64-bit float (NaN/infinite values are rejected at validation).
    Float,
    /// Boolean, rendered as `TRUE`/`FALSE`.
    Boolean,
    /// Calendar date in `YYYY-MM-DD`, rendered as `DATE '...'`.
    Date,
}

impl ParamType {
    fn label(self) -> &'static str {
        match self {
            ParamType::String => "string",
            ParamType::Integer => "integer",
            ParamType::Float => "float",
            ParamType::Boolean => "boolean",
            ParamType::Date => "date",
        }
    }
}

/// A concrete, typed parameter value.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    /// Text value.
    String(String),
    /// Integer value.
    Integer(i64),
    /// Float value.
    Float(f64),
    /// Boolean value.
    Boolean(bool),
    /// Date value as `YYYY-MM-DD` (validated on construction paths).
    Date(String),
}

impl ParamValue {
    /// The type this value carries.
    pub fn param_type(&self) -> ParamType {
        match self {
            ParamValue::String(_) => ParamType::String,
            ParamValue::Integer(_) => ParamType::Integer,
            ParamValue::Float(_) => ParamType::Float,
            ParamValue::Boolean(_) => ParamType::Boolean,
            ParamValue::Date(_) => ParamType::Date,
        }
    }

    /// As a numeric value, for bounds checking (integers and floats only).
    fn as_f64(&self) -> Option<f64> {
        match self {
            ParamValue::Integer(i) => Some(*i as f64),
            ParamValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Render to a safe SQL literal. Returns an error only for values that
    /// can't be represented safely (non-finite floats, malformed dates) — these
    /// are normally caught earlier by validation, but rendering re-checks so the
    /// guarantee holds even for hand-built [`QueryParams`].
    pub fn to_sql_literal(&self) -> Result<String> {
        Ok(match self {
            // Double single-quotes — the standard SQL string escape. No other
            // metacharacter can break out of a single-quoted literal.
            ParamValue::String(s) => format!("'{}'", s.replace('\'', "''")),
            ParamValue::Integer(i) => i.to_string(),
            ParamValue::Float(f) => {
                if !f.is_finite() {
                    return Err(QueryError::Param(format!(
                        "non-finite float ({f}) cannot be used as a parameter"
                    )));
                }
                // Always include a decimal point so it reads as a float literal.
                let s = format!("{f}");
                if s.contains('.') || s.contains('e') || s.contains('E') {
                    s
                } else {
                    format!("{s}.0")
                }
            }
            ParamValue::Boolean(b) => {
                if *b {
                    "TRUE".to_string()
                } else {
                    "FALSE".to_string()
                }
            }
            ParamValue::Date(d) => {
                validate_date(d)?;
                format!("DATE '{d}'")
            }
        })
    }
}

/// Validate a `YYYY-MM-DD` date string. Rejects anything else so a date param
/// can never smuggle SQL through the `DATE '...'` rendering.
fn validate_date(d: &str) -> Result<()> {
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|_| QueryError::Param(format!("'{d}' is not a valid YYYY-MM-DD date")))
}

/// A parameter declaration: its type, optional default, and optional numeric
/// bounds (applied to integer/float types).
#[derive(Debug, Clone)]
pub struct ParamDecl {
    /// Parameter name (the `{{name}}` referenced in SQL).
    pub name: String,
    /// Declared type.
    pub param_type: ParamType,
    /// Default used when no value is provided.
    pub default: Option<ParamValue>,
    /// Inclusive lower bound (numeric types only).
    pub min: Option<f64>,
    /// Inclusive upper bound (numeric types only).
    pub max: Option<f64>,
}

impl ParamDecl {
    /// A required parameter of the given type with no default or bounds.
    pub fn required(name: impl Into<String>, param_type: ParamType) -> Self {
        Self {
            name: name.into(),
            param_type,
            default: None,
            min: None,
            max: None,
        }
    }

    /// Attach a default value (makes the parameter optional).
    pub fn with_default(mut self, default: ParamValue) -> Self {
        self.default = Some(default);
        self
    }

    /// Attach inclusive numeric bounds.
    pub fn with_bounds(mut self, min: f64, max: f64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }
}

/// A set of declared parameters. Resolving a schema against caller-provided
/// values validates types and bounds, fills defaults, and rejects unknown or
/// missing parameters — producing the [`QueryParams`] that substitution uses.
#[derive(Debug, Clone, Default)]
pub struct ParamSchema {
    decls: Vec<ParamDecl>,
}

impl ParamSchema {
    /// Build a schema from declarations.
    pub fn new(decls: Vec<ParamDecl>) -> Self {
        Self { decls }
    }

    /// Validate `provided` against the declarations and produce the final
    /// values. Errors on: unknown parameter, type mismatch, out-of-bounds, or a
    /// required parameter with no value and no default.
    pub fn resolve(&self, provided: &HashMap<String, ParamValue>) -> Result<QueryParams> {
        // Reject values that don't correspond to any declaration — keeps the
        // contract tight and catches typos.
        for name in provided.keys() {
            if !self.decls.iter().any(|d| &d.name == name) {
                return Err(QueryError::Param(format!("unknown parameter '{name}'")));
            }
        }

        let mut out = HashMap::new();
        for decl in &self.decls {
            let value = match provided.get(&decl.name) {
                Some(v) => v.clone(),
                None => match &decl.default {
                    Some(d) => d.clone(),
                    None => {
                        return Err(QueryError::Param(format!(
                            "missing required parameter '{}'",
                            decl.name
                        )));
                    }
                },
            };

            if value.param_type() != decl.param_type {
                return Err(QueryError::Param(format!(
                    "parameter '{}' expected {} but got {}",
                    decl.name,
                    decl.param_type.label(),
                    value.param_type().label()
                )));
            }

            if let Some(n) = value.as_f64() {
                if let Some(min) = decl.min
                    && n < min
                {
                    return Err(QueryError::Param(format!(
                        "parameter '{}' = {n} is below minimum {min}",
                        decl.name
                    )));
                }
                if let Some(max) = decl.max
                    && n > max
                {
                    return Err(QueryError::Param(format!(
                        "parameter '{}' = {n} is above maximum {max}",
                        decl.name
                    )));
                }
            }

            // Validate date strings early so resolution is the single gate.
            if let ParamValue::Date(d) = &value {
                validate_date(d)?;
            }

            out.insert(decl.name.clone(), value);
        }

        Ok(QueryParams { values: out })
    }
}

/// Validated parameter values ready for substitution into SQL.
#[derive(Debug, Clone, Default)]
pub struct QueryParams {
    values: HashMap<String, ParamValue>,
}

impl QueryParams {
    /// Empty parameter set (for queries that take none).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build directly from a map of values (no schema validation). Prefer
    /// [`ParamSchema::resolve`] when declarations exist; this is for callers
    /// that have already-typed values or queries with no formal schema.
    pub fn from_values(values: HashMap<String, ParamValue>) -> Self {
        Self { values }
    }

    /// Look up a value by name.
    pub fn get(&self, name: &str) -> Option<&ParamValue> {
        self.values.get(name)
    }
}

/// Replace every `{{name}}` in `sql` with the safe SQL literal of the matching
/// parameter. Errors if a referenced parameter has no value. Text outside
/// `{{...}}` is copied verbatim.
pub fn substitute(sql: &str, params: &QueryParams) -> Result<String> {
    let mut out = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // Find the closing `}}`.
            let rest = &sql[i + 2..];
            let close = rest
                .find("}}")
                .ok_or_else(|| QueryError::Param("unterminated '{{' in query".to_string()))?;
            let name = rest[..close].trim();
            let value = params
                .get(name)
                .ok_or_else(|| QueryError::UnboundParam(name.to_string()))?;
            out.push_str(&value.to_sql_literal()?);
            i = i + 2 + close + 2;
        } else {
            // Push one full UTF-8 char so we never split a multibyte sequence.
            let ch = sql[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Ok(out)
}

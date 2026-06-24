// SPDX-License-Identifier: MIT OR Apache-2.0

//! Dashboard parameter resolution (doc 07).
//!
//! The dashboard DSL declares parameters with a UI-flavored shape (`select`
//! with options, `daterange` with presets, `multiselect` with bounds and a
//! JSON `default`). The query engine, by contrast, substitutes **typed,
//! escaped scalar literals** ([`ParamValue`]). This module is the bridge: given
//! the declared [`Parameter`]s and a map of raw user inputs, it validates each
//! value against its kind, fills defaults, expands a `daterange` into
//! `{{id.start}}` / `{{id.end}}` and a `multiselect` into a safe `IN (...)`
//! list, and produces the [`QueryParams`] that [`crate::substitute`] consumes.
//!
//! It also resolves the `$row` / `$column` / `$selection` tokens that a
//! `set_parameter` interaction carries, turning a clicked row into a concrete
//! parameter value (click-to-filter).

use crate::error::{QueryError, Result};
use crate::params::{ParamValue, QueryParams};
use almagest_core::{ParamKind, Parameter};
use chrono::{Datelike, Duration, NaiveDate};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// The literal a `select` treats as "all" when `allow_all` is set. Such a value
/// is accepted even if it is not among the declared options (the SQL author
/// short-circuits it, as in `({{region}} = 'All' OR region = {{region}})`).
pub const ALL_SENTINEL: &str = "All";

/// Validate and resolve dashboard parameter inputs into engine [`QueryParams`].
///
/// `provided` holds the raw user inputs keyed by parameter id (already layered
/// by the caller — see [`crate::urlstate::layered_state`]). A parameter absent
/// from `provided` falls back to its declared `default`; a parameter with
/// neither is an error. `today` anchors relative `daterange` presets so the
/// result is deterministic and testable.
pub fn resolve_parameters(
    decls: &[Parameter],
    provided: &BTreeMap<String, Value>,
    today: NaiveDate,
) -> Result<QueryParams> {
    let mut out: HashMap<String, ParamValue> = HashMap::new();

    for decl in decls {
        let raw = match provided.get(&decl.id).or(decl.default.as_ref()) {
            Some(v) => v,
            None => {
                return Err(QueryError::Param(format!(
                    "missing required parameter '{}'",
                    decl.id
                )));
            }
        };
        resolve_one(decl, raw, today, &mut out)?;
    }

    Ok(QueryParams::from_values(out))
}

/// Resolve a single declared parameter into one (or, for a daterange, two)
/// entries in `out`.
fn resolve_one(
    decl: &Parameter,
    raw: &Value,
    today: NaiveDate,
    out: &mut HashMap<String, ParamValue>,
) -> Result<()> {
    let id = &decl.id;
    match decl.kind {
        ParamKind::Text => {
            let s = expect_string(raw, id, "text")?;
            out.insert(id.clone(), ParamValue::String(s));
        }
        ParamKind::Number => {
            out.insert(id.clone(), resolve_number(decl, raw)?);
        }
        ParamKind::Boolean => {
            let b = raw
                .as_bool()
                .ok_or_else(|| QueryError::Param(format!("parameter '{id}' expected a boolean")))?;
            out.insert(id.clone(), ParamValue::Boolean(b));
        }
        ParamKind::Date => {
            let d = resolve_date(raw, id, today)?;
            out.insert(id.clone(), ParamValue::Date(d));
        }
        ParamKind::DateRange => {
            let (start, end) = resolve_daterange(raw, id, today)?;
            out.insert(format!("{id}.start"), ParamValue::Date(start));
            out.insert(format!("{id}.end"), ParamValue::Date(end));
        }
        ParamKind::Select => {
            let s = expect_string(raw, id, "select")?;
            check_select_option(decl, &s)?;
            out.insert(id.clone(), ParamValue::String(s));
        }
        ParamKind::MultiSelect => {
            out.insert(id.clone(), resolve_multiselect(decl, raw)?);
        }
    }
    Ok(())
}

fn expect_string(raw: &Value, id: &str, kind: &str) -> Result<String> {
    raw.as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| QueryError::Param(format!("parameter '{id}' expected a {kind} string")))
}

fn resolve_number(decl: &Parameter, raw: &Value) -> Result<ParamValue> {
    let id = &decl.id;
    if !raw.is_number() {
        return Err(QueryError::Param(format!(
            "parameter '{id}' expected a number"
        )));
    }

    // Preserve integer-ness: `LIMIT {{n}}` needs `50`, not `50.0`. `as_i64`
    // returns `Some` only for an integral value that fits i64.
    let value = if let Some(i) = raw.as_i64() {
        ParamValue::Integer(i)
    } else {
        let f = raw
            .as_f64()
            .ok_or_else(|| QueryError::Param(format!("parameter '{id}' is not a finite number")))?;
        ParamValue::Float(f)
    };

    let n = match &value {
        ParamValue::Integer(i) => *i as f64,
        ParamValue::Float(f) => *f,
        _ => unreachable!(),
    };
    if let Some(min) = decl.min
        && n < min
    {
        return Err(QueryError::Param(format!(
            "parameter '{id}' = {n} is below minimum {min}"
        )));
    }
    if let Some(max) = decl.max
        && n > max
    {
        return Err(QueryError::Param(format!(
            "parameter '{id}' = {n} is above maximum {max}"
        )));
    }
    Ok(value)
}

/// Resolve a single date value. Accepts a `YYYY-MM-DD` string or the `today`
/// sentinel.
fn resolve_date(raw: &Value, id: &str, today: NaiveDate) -> Result<String> {
    let s = expect_string(raw, id, "date")?;
    if s == "today" {
        return Ok(today.format("%Y-%m-%d").to_string());
    }
    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
        .map(|_| s)
        .map_err(|_| QueryError::Param(format!("parameter '{id}' = '{}' is not a valid date", raw)))
}

/// Resolve a daterange object into concrete `(start, end)` `YYYY-MM-DD` strings.
/// Accepts `{ "preset": "last_30_days" }` or an explicit
/// `{ "start": "...", "end": "..." }` (also when `preset` is `"custom"`).
fn resolve_daterange(raw: &Value, id: &str, today: NaiveDate) -> Result<(String, String)> {
    let obj = raw.as_object().ok_or_else(|| {
        QueryError::Param(format!("parameter '{id}' expected a daterange object"))
    })?;

    let preset = obj.get("preset").and_then(Value::as_str);
    if let Some(preset) = preset
        && preset != "custom"
    {
        let (s, e) = resolve_daterange_preset(preset, today)
            .map_err(|e| QueryError::Param(format!("parameter '{id}': {e}")))?;
        return Ok((
            s.format("%Y-%m-%d").to_string(),
            e.format("%Y-%m-%d").to_string(),
        ));
    }

    // Explicit / custom range: require both endpoints.
    let start = obj.get("start").and_then(Value::as_str).ok_or_else(|| {
        QueryError::Param(format!("parameter '{id}' daterange needs a 'start' date"))
    })?;
    let end = obj.get("end").and_then(Value::as_str).ok_or_else(|| {
        QueryError::Param(format!("parameter '{id}' daterange needs an 'end' date"))
    })?;
    for d in [start, end] {
        NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| {
            QueryError::Param(format!(
                "parameter '{id}' daterange '{d}' is not a valid date"
            ))
        })?;
    }
    Ok((start.to_string(), end.to_string()))
}

/// Resolve a named relative date-range preset against `today`. The bounds are
/// inclusive; rolling windows count back from today (`last_7_days` =
/// `[today-6, today]`).
pub fn resolve_daterange_preset(
    preset: &str,
    today: NaiveDate,
) -> std::result::Result<(NaiveDate, NaiveDate), String> {
    let back = |n: i64| today - Duration::days(n);
    let pair = match preset {
        "today" => (today, today),
        "yesterday" => (back(1), back(1)),
        "last_7_days" => (back(6), today),
        "last_30_days" => (back(29), today),
        "last_90_days" => (back(89), today),
        "this_month" => (first_of_month(today), today),
        "last_month" => {
            let first_this = first_of_month(today);
            let last_prev = first_this - Duration::days(1);
            (first_of_month(last_prev), last_prev)
        }
        "this_quarter" => (first_of_quarter(today), today),
        "last_quarter" => {
            let first_this = first_of_quarter(today);
            let last_prev = first_this - Duration::days(1);
            (first_of_quarter(last_prev), last_prev)
        }
        "this_year" => (NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap(), today),
        other => return Err(format!("unknown date-range preset '{other}'")),
    };
    Ok(pair)
}

fn first_of_month(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap()
}

fn first_of_quarter(d: NaiveDate) -> NaiveDate {
    let q_first_month = ((d.month() - 1) / 3) * 3 + 1;
    NaiveDate::from_ymd_opt(d.year(), q_first_month, 1).unwrap()
}

fn check_select_option(decl: &Parameter, value: &str) -> Result<()> {
    // With dynamic options we can't validate membership statically; trust the
    // value (the options_query is the source of truth at load time).
    if decl.options_query.is_some() {
        return Ok(());
    }
    if decl.allow_all && value == ALL_SENTINEL {
        return Ok(());
    }
    if let Some(opts) = &decl.options
        && !opts.iter().any(|o| o == value)
    {
        return Err(QueryError::Param(format!(
            "parameter '{}' value '{value}' is not one of its options",
            decl.id
        )));
    }
    Ok(())
}

fn resolve_multiselect(decl: &Parameter, raw: &Value) -> Result<ParamValue> {
    let id = &decl.id;
    let arr = raw.as_array().ok_or_else(|| {
        QueryError::Param(format!("parameter '{id}' expected an array of choices"))
    })?;

    if let Some(min) = decl.min_selections
        && arr.len() < min
    {
        return Err(QueryError::Param(format!(
            "parameter '{id}' needs at least {min} selection(s)"
        )));
    }
    if let Some(max) = decl.max_selections
        && arr.len() > max
    {
        return Err(QueryError::Param(format!(
            "parameter '{id}' allows at most {max} selection(s)"
        )));
    }

    let mut items = Vec::with_capacity(arr.len());
    for v in arr {
        let s = v.as_str().ok_or_else(|| {
            QueryError::Param(format!("parameter '{id}' choices must be strings"))
        })?;
        if decl.options_query.is_none()
            && let Some(opts) = &decl.options
            && !opts.iter().any(|o| o == s)
        {
            return Err(QueryError::Param(format!(
                "parameter '{id}' choice '{s}' is not one of its options"
            )));
        }
        items.push(ParamValue::String(s.to_string()));
    }
    Ok(ParamValue::List(items))
}

/// Resolve the value template of a `set_parameter` interaction against the
/// clicked context. Recognised tokens (whole-string only):
///
/// - `"$row.<col>"` / `"$selection.<col>"` → that column's value from `row`
/// - `"$column"` → the clicked column name
///
/// Anything else is a literal and is returned unchanged. A referenced column
/// that is absent resolves to `null`.
pub fn interpolate_action_value(
    template: &Value,
    row: &serde_json::Map<String, Value>,
    column: Option<&str>,
) -> Value {
    let Some(s) = template.as_str() else {
        return template.clone();
    };
    if s == "$column" {
        return Value::String(column.unwrap_or_default().to_string());
    }
    for prefix in ["$row.", "$selection."] {
        if let Some(col) = s.strip_prefix(prefix) {
            return row.get(col).cloned().unwrap_or(Value::Null);
        }
    }
    template.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn presets_resolve_relative_to_today() {
        let today = day(2026, 6, 24); // a Wednesday in Q2
        assert_eq!(
            resolve_daterange_preset("last_7_days", today).unwrap(),
            (day(2026, 6, 18), day(2026, 6, 24))
        );
        assert_eq!(
            resolve_daterange_preset("this_month", today).unwrap(),
            (day(2026, 6, 1), day(2026, 6, 24))
        );
        assert_eq!(
            resolve_daterange_preset("last_month", today).unwrap(),
            (day(2026, 5, 1), day(2026, 5, 31))
        );
        assert_eq!(
            resolve_daterange_preset("this_quarter", today).unwrap(),
            (day(2026, 4, 1), day(2026, 6, 24))
        );
        assert_eq!(
            resolve_daterange_preset("last_quarter", today).unwrap(),
            (day(2026, 1, 1), day(2026, 3, 31))
        );
        assert!(resolve_daterange_preset("never", today).is_err());
    }

    #[test]
    fn interpolation_pulls_from_clicked_row() {
        let mut row = serde_json::Map::new();
        row.insert("region".into(), json!("EU"));
        assert_eq!(
            interpolate_action_value(&json!("$row.region"), &row, None),
            json!("EU")
        );
        assert_eq!(
            interpolate_action_value(&json!("$selection.region"), &row, None),
            json!("EU")
        );
        assert_eq!(
            interpolate_action_value(&json!("$column"), &row, Some("amount")),
            json!("amount")
        );
        // Literal passthrough and missing column.
        assert_eq!(
            interpolate_action_value(&json!("US"), &row, None),
            json!("US")
        );
        assert_eq!(
            interpolate_action_value(&json!("$row.ghost"), &row, None),
            Value::Null
        );
    }
}

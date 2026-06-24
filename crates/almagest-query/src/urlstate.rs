// SPDX-License-Identifier: MIT OR Apache-2.0

//! URL parameter state (doc 07): encode the current parameter values into a
//! shareable query string and decode them back, typed by the declarations.
//!
//! URL persistence is what makes a filtered view link-shareable ("here's the
//! dashboard with region=EU and the last 90 days"). Encoding is **decl-aware**
//! so the query string stays human-readable (`region=EU`, not a blob of JSON);
//! decoding consults the [`Parameter`] kinds to rebuild the right JSON shape
//! (a `multiselect` becomes an array, a `daterange` an object).
//!
//! A small percent-encoder is included rather than a dependency — the query
//! engine has no other use for a URL crate, and minimizing the dependency
//! surface is a project value.

use almagest_core::{ParamKind, Parameter};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Encode parameter values (keyed by id) into a `key=value&…` query string
/// (without a leading `?`). Scalars encode as themselves; a `multiselect`
/// array repeats its key; a `daterange` object encodes either its `preset` or
/// its `start`/`end` endpoints. `null` values are omitted.
pub fn encode_url_state(values: &BTreeMap<String, Value>) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (id, value) in values {
        match value {
            Value::Null => {}
            Value::Array(items) => {
                for item in items {
                    if let Some(s) = scalar_to_string(item) {
                        parts.push(pair(id, &s));
                    }
                }
            }
            Value::Object(obj) => {
                if let Some(preset) = obj.get("preset").and_then(Value::as_str) {
                    parts.push(pair(&format!("{id}.preset"), preset));
                } else {
                    if let Some(s) = obj.get("start").and_then(Value::as_str) {
                        parts.push(pair(&format!("{id}.start"), s));
                    }
                    if let Some(e) = obj.get("end").and_then(Value::as_str) {
                        parts.push(pair(&format!("{id}.end"), e));
                    }
                }
            }
            other => {
                if let Some(s) = scalar_to_string(other) {
                    parts.push(pair(id, &s));
                }
            }
        }
    }
    parts.join("&")
}

/// Decode a query string into raw parameter values, typed by `decls`. Keys not
/// matching any declaration are ignored; a value that doesn't parse for its
/// kind is dropped (so resolution falls back to the declared default). The
/// result feeds [`crate::resolve::resolve_parameters`].
pub fn decode_url_state(query: &str, decls: &[Parameter]) -> BTreeMap<String, Value> {
    let raw = parse_query(query);
    let mut out = BTreeMap::new();

    for decl in decls {
        let id = &decl.id;
        match decl.kind {
            ParamKind::MultiSelect => {
                if let Some(values) = raw.get(id) {
                    let arr = values.iter().cloned().map(Value::String).collect();
                    out.insert(id.clone(), Value::Array(arr));
                }
            }
            ParamKind::DateRange => {
                if let Some(preset) = raw.get(&format!("{id}.preset")).and_then(|v| v.first()) {
                    let mut obj = Map::new();
                    obj.insert("preset".into(), Value::String(preset.clone()));
                    out.insert(id.clone(), Value::Object(obj));
                } else {
                    let start = raw.get(&format!("{id}.start")).and_then(|v| v.first());
                    let end = raw.get(&format!("{id}.end")).and_then(|v| v.first());
                    if let (Some(start), Some(end)) = (start, end) {
                        let mut obj = Map::new();
                        obj.insert("start".into(), Value::String(start.clone()));
                        obj.insert("end".into(), Value::String(end.clone()));
                        out.insert(id.clone(), Value::Object(obj));
                    }
                }
            }
            ParamKind::Number => {
                if let Some(s) = raw.get(id).and_then(|v| v.first())
                    && let Some(n) = parse_number(s)
                {
                    out.insert(id.clone(), n);
                }
            }
            ParamKind::Boolean => {
                if let Some(s) = raw.get(id).and_then(|v| v.first())
                    && let Ok(b) = s.parse::<bool>()
                {
                    out.insert(id.clone(), Value::Bool(b));
                }
            }
            ParamKind::Text | ParamKind::Date | ParamKind::Select => {
                if let Some(s) = raw.get(id).and_then(|v| v.first()) {
                    out.insert(id.clone(), Value::String(s.clone()));
                }
            }
        }
    }
    out
}

/// Layer parameter sources by priority: URL state overrides file-backed
/// defaults. (Declared defaults are applied later, by resolution, when a
/// parameter is absent from the merged map — completing the URL > file >
/// default precedence from doc 07.)
pub fn layered_state(
    url: &BTreeMap<String, Value>,
    file: &BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    let mut out = file.clone();
    for (k, v) in url {
        out.insert(k.clone(), v.clone());
    }
    out
}

fn pair(key: &str, value: &str) -> String {
    format!("{}={}", pct_encode(key), pct_encode(value))
}

fn scalar_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn parse_number(s: &str) -> Option<Value> {
    if let Ok(i) = s.parse::<i64>() {
        Some(Value::Number(i.into()))
    } else if let Ok(f) = s.parse::<f64>() {
        serde_json::Number::from_f64(f).map(Value::Number)
    } else {
        None
    }
}

/// Parse a `key=value&…` query string into a map of key → ordered values
/// (repeated keys accumulate, supporting multiselect).
fn parse_query(query: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let query = query.strip_prefix('?').unwrap_or(query);
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (pct_decode(k), pct_decode(v)),
            None => (pct_decode(pair), String::new()),
        };
        out.entry(k).or_default().push(v);
    }
    out
}

/// Percent-encode for a query component, keeping the RFC 3986 unreserved set
/// (`A–Z a–z 0–9 - _ . ~`) literal and escaping everything else.
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_digit(b >> 4));
            out.push(hex_digit(b & 0x0f));
        }
    }
    out
}

/// Decode percent-escapes; `+` is treated as a space (form convention).
fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push((h << 4) | l);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + (n - 10)) as char,
    }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use almagest_core::ParamKind;
    use serde_json::json;

    fn decl(id: &str, kind: ParamKind) -> Parameter {
        Parameter {
            id: id.into(),
            kind,
            label: None,
            description: None,
            default: None,
            options: None,
            options_query: None,
            min: None,
            max: None,
            min_selections: None,
            max_selections: None,
            allow_all: false,
            persist: None,
        }
    }

    #[test]
    fn percent_round_trip_handles_specials() {
        let s = "a b&c=d/e";
        assert_eq!(pct_decode(&pct_encode(s)), s);
    }

    #[test]
    fn decode_types_by_declaration() {
        let decls = vec![
            decl("region", ParamKind::Select),
            decl("limit", ParamKind::Number),
            decl("active", ParamKind::Boolean),
            decl("cats", ParamKind::MultiSelect),
            decl("dr", ParamKind::DateRange),
        ];
        let q = "region=EU&limit=50&active=true&cats=a&cats=b&dr.preset=last_30_days";
        let state = decode_url_state(q, &decls);
        assert_eq!(state["region"], json!("EU"));
        assert_eq!(state["limit"], json!(50));
        assert_eq!(state["active"], json!(true));
        assert_eq!(state["cats"], json!(["a", "b"]));
        assert_eq!(state["dr"], json!({ "preset": "last_30_days" }));
    }

    #[test]
    fn encode_decode_round_trip() {
        let decls = vec![
            decl("region", ParamKind::Select),
            decl("cats", ParamKind::MultiSelect),
            decl("dr", ParamKind::DateRange),
        ];
        let mut state = BTreeMap::new();
        state.insert("region".to_string(), json!("North & South"));
        state.insert("cats".to_string(), json!(["x", "y"]));
        state.insert(
            "dr".to_string(),
            json!({ "start": "2026-01-01", "end": "2026-03-31" }),
        );

        let encoded = encode_url_state(&state);
        let decoded = decode_url_state(&encoded, &decls);
        assert_eq!(decoded["region"], json!("North & South"));
        assert_eq!(decoded["cats"], json!(["x", "y"]));
        assert_eq!(
            decoded["dr"],
            json!({ "start": "2026-01-01", "end": "2026-03-31" })
        );
    }

    #[test]
    fn url_overrides_file_in_layering() {
        let mut file = BTreeMap::new();
        file.insert("region".to_string(), json!("US"));
        file.insert("limit".to_string(), json!(10));
        let mut url = BTreeMap::new();
        url.insert("region".to_string(), json!("EU"));

        let merged = layered_state(&url, &file);
        assert_eq!(merged["region"], json!("EU")); // URL wins
        assert_eq!(merged["limit"], json!(10)); // file retained
    }
}

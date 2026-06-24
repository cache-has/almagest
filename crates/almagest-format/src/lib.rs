// SPDX-License-Identifier: MIT OR Apache-2.0

//! # almagest-format
//!
//! The canonical value-formatting spec for Almagest panels. A [`Format`] is a
//! declarative description of how a raw value should be displayed (currency,
//! percent, compact, datetime, …); [`Format::apply`] turns a [`FormatValue`]
//! into the display string.
//!
//! This crate is the single source of truth for formatting so that every
//! surface renders a value identically: the Svelte renderer mirrors this logic
//! client-side, while server-side exporters (static PDF / PNG reports) call it
//! directly. Keeping it as a leaf crate (no dependency on `almagest-core`) lets
//! both the format types and their behaviour be shared without a dependency
//! cycle.
//!
//! The `Format` enum is `serde`-tagged on `kind`, matching the JSON in the
//! dashboard DSL (`planning/06`):
//!
//! ```json
//! { "kind": "currency", "currency": "USD", "prefix": "$", "decimal_places": 2 }
//! ```

use serde::{Deserialize, Serialize};

/// How a value should be displayed. Tagged on `kind`; missing options fall back
/// to sensible defaults so authors can write `{ "kind": "number" }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Format {
    /// A plain number with optional fixed decimals and thousands separators.
    Number {
        /// Decimal places to show. `None` keeps the value's natural precision.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decimal_places: Option<u8>,
        /// Group thousands with commas (`1,234,567`).
        #[serde(default = "default_true")]
        thousands_separator: bool,
    },
    /// A monetary amount: `prefix` + number + `suffix`.
    Currency {
        /// ISO currency code (informational; not used for symbol lookup).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        currency: Option<String>,
        /// Leading symbol (e.g. `$`). Defaults to `$` when omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
        /// Trailing text (e.g. ` USD`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        suffix: Option<String>,
        /// Decimal places (default 2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decimal_places: Option<u8>,
        /// Render compactly (`$1.2M`) instead of fully (`$1,200,000.00`).
        #[serde(default)]
        compact: bool,
    },
    /// A ratio rendered as a percentage. The raw value is treated as a fraction
    /// (`0.123` → `12.3%`).
    Percent {
        /// Decimal places (default 1).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decimal_places: Option<u8>,
    },
    /// Compact magnitude notation: `1.2K`, `3.4M`, `5.6B`, `7.8T`.
    Compact,
    /// A timestamp. With `relative: true` it renders as "3 days ago"; otherwise
    /// it is formatted with `format` (a `strftime` pattern, default
    /// `%Y-%m-%d %H:%M`).
    Datetime {
        /// `strftime` pattern for absolute rendering.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<String>,
        /// Render as a relative phrase against "now" instead of absolutely.
        #[serde(default)]
        relative: bool,
    },
    /// A span of time. The raw value is a count of `unit`s, humanised to
    /// `1h 2m 3s`.
    Duration {
        /// The unit the raw value is expressed in.
        #[serde(default)]
        unit: DurationUnit,
    },
    /// Map a raw value to a label via a lookup table; unmapped values render as
    /// themselves.
    Enum {
        /// The value → label mapping.
        values: std::collections::BTreeMap<String, String>,
    },
    /// A free-form template; `${value}` is replaced with the plainly-rendered
    /// value.
    Custom {
        /// Template string containing `${value}`.
        template: String,
    },
}

/// The unit a [`Format::Duration`] value is expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurationUnit {
    /// Milliseconds.
    Milliseconds,
    /// Seconds (the default).
    #[default]
    Seconds,
    /// Minutes.
    Minutes,
    /// Hours.
    Hours,
}

/// A raw value to be formatted. This is the small, format-agnostic value type
/// that components extract from a query result cell.
#[derive(Debug, Clone, PartialEq)]
pub enum FormatValue {
    /// A SQL `NULL` / missing value.
    Null,
    /// An integer.
    Int(i64),
    /// A floating-point number.
    Float(f64),
    /// A boolean.
    Bool(bool),
    /// Text.
    Text(String),
    /// A UTC timestamp in epoch milliseconds.
    Timestamp(i64),
}

impl FormatValue {
    /// Numeric view of the value, if it has one.
    fn as_f64(&self) -> Option<f64> {
        match self {
            FormatValue::Int(i) => Some(*i as f64),
            FormatValue::Float(f) => Some(*f),
            FormatValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            FormatValue::Text(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        }
    }

    /// Plain, format-free rendering (what `${value}` and the enum-fallthrough
    /// path use).
    fn plain(&self) -> String {
        match self {
            FormatValue::Null => NULL_DISPLAY.to_string(),
            FormatValue::Int(i) => i.to_string(),
            FormatValue::Float(f) => trim_float(*f),
            FormatValue::Bool(b) => b.to_string(),
            FormatValue::Text(s) => s.clone(),
            FormatValue::Timestamp(ms) => ms.to_string(),
        }
    }
}

/// The string shown for a `NULL` value.
pub const NULL_DISPLAY: &str = "—";

impl Format {
    /// Format `value` for display. `NULL` always renders as [`NULL_DISPLAY`]
    /// regardless of kind. Relative datetimes are measured against the current
    /// wall-clock; use [`Format::apply_at`] for a deterministic reference.
    pub fn apply(&self, value: &FormatValue) -> String {
        self.apply_at(value, chrono::Utc::now().timestamp_millis())
    }

    /// Like [`Format::apply`], but `now_ms` is the reference instant used for
    /// relative datetimes (epoch milliseconds). Deterministic — used by tests
    /// and reproducible exports.
    pub fn apply_at(&self, value: &FormatValue, now_ms: i64) -> String {
        if matches!(value, FormatValue::Null) {
            return NULL_DISPLAY.to_string();
        }
        match self {
            Format::Number {
                decimal_places,
                thousands_separator,
            } => match value.as_f64() {
                Some(n) => format_number(n, *decimal_places, *thousands_separator),
                None => value.plain(),
            },
            Format::Currency {
                prefix,
                suffix,
                decimal_places,
                compact,
                ..
            } => match value.as_f64() {
                Some(n) => {
                    let prefix = prefix.as_deref().unwrap_or("$");
                    let body = if *compact {
                        format_compact(n, decimal_places.unwrap_or(1))
                    } else {
                        format_number(n, Some(decimal_places.unwrap_or(2)), true)
                    };
                    // Keep the minus sign ahead of the currency prefix: -$5.00.
                    match body.strip_prefix('-') {
                        Some(rest) => format!("-{prefix}{rest}{}", suffix.as_deref().unwrap_or("")),
                        None => format!("{prefix}{body}{}", suffix.as_deref().unwrap_or("")),
                    }
                }
                None => value.plain(),
            },
            Format::Percent { decimal_places } => match value.as_f64() {
                Some(n) => format!(
                    "{}%",
                    format_number(n * 100.0, Some(decimal_places.unwrap_or(1)), true)
                ),
                None => value.plain(),
            },
            Format::Compact => match value.as_f64() {
                Some(n) => format_compact(n, 1),
                None => value.plain(),
            },
            Format::Datetime { format, relative } => {
                let Some(ms) = timestamp_ms(value) else {
                    return value.plain();
                };
                if *relative {
                    humanize_relative(now_ms - ms)
                } else {
                    let pattern = format.as_deref().unwrap_or("%Y-%m-%d %H:%M");
                    match chrono::DateTime::from_timestamp_millis(ms) {
                        Some(dt) => dt.format(pattern).to_string(),
                        None => value.plain(),
                    }
                }
            }
            Format::Duration { unit } => match value.as_f64() {
                Some(n) => humanize_duration(n, *unit),
                None => value.plain(),
            },
            Format::Enum { values } => {
                let key = value.plain();
                values.get(&key).cloned().unwrap_or(key)
            }
            Format::Custom { template } => template.replace("${value}", &value.plain()),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Interpret a value as an epoch-millisecond timestamp. Accepts an explicit
/// [`FormatValue::Timestamp`], an integer (assumed ms), or an RFC 3339 string.
fn timestamp_ms(value: &FormatValue) -> Option<i64> {
    match value {
        FormatValue::Timestamp(ms) => Some(*ms),
        FormatValue::Int(i) => Some(*i),
        FormatValue::Text(s) => chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.timestamp_millis()),
        _ => None,
    }
}

/// Render `n` with optional fixed decimals and optional thousands grouping.
fn format_number(n: f64, decimal_places: Option<u8>, thousands: bool) -> String {
    if !n.is_finite() {
        return n.to_string();
    }
    let negative = n.is_sign_negative() && n != 0.0;
    let abs = n.abs();

    let rendered = match decimal_places {
        Some(dp) => format!("{abs:.*}", dp as usize),
        None => trim_float(abs),
    };

    let (int_part, frac_part) = match rendered.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (rendered, None),
    };

    let grouped = if thousands {
        group_thousands(&int_part)
    } else {
        int_part
    };

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&grouped);
    if let Some(frac) = frac_part {
        out.push('.');
        out.push_str(&frac);
    }
    out
}

/// Insert comma separators into the integer-part string `digits`.
fn group_thousands(digits: &str) -> String {
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let len = bytes.len();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Render `n` in compact magnitude notation with `decimals` after the point.
fn format_compact(n: f64, decimals: u8) -> String {
    if !n.is_finite() {
        return n.to_string();
    }
    let negative = n.is_sign_negative() && n != 0.0;
    let abs = n.abs();
    let (scaled, suffix) = if abs >= 1e12 {
        (abs / 1e12, "T")
    } else if abs >= 1e9 {
        (abs / 1e9, "B")
    } else if abs >= 1e6 {
        (abs / 1e6, "M")
    } else if abs >= 1e3 {
        (abs / 1e3, "K")
    } else {
        // Below 1000: no suffix, trimmed to the requested precision.
        let body = format_number(abs, Some(decimals), true);
        let body = trim_trailing_zeros(&body);
        return if negative { format!("-{body}") } else { body };
    };
    let body = trim_trailing_zeros(&format!("{scaled:.*}", decimals as usize));
    if negative {
        format!("-{body}{suffix}")
    } else {
        format!("{body}{suffix}")
    }
}

/// Drop a trailing `.000…` and any trailing zeros in the fractional part.
fn trim_trailing_zeros(s: &str) -> String {
    if !s.contains('.') {
        return s.to_string();
    }
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    trimmed.to_string()
}

/// Render a float without a forced decimal point or trailing zeros.
fn trim_float(f: f64) -> String {
    if f == f.trunc() && f.is_finite() && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        let s = format!("{f}");
        s
    }
}

/// Humanise a non-negative count of `unit`s into `1h 2m 3s` form.
fn humanize_duration(amount: f64, unit: DurationUnit) -> String {
    let total_secs = match unit {
        DurationUnit::Milliseconds => amount / 1000.0,
        DurationUnit::Seconds => amount,
        DurationUnit::Minutes => amount * 60.0,
        DurationUnit::Hours => amount * 3600.0,
    };
    let negative = total_secs < 0.0;
    let mut secs = total_secs.abs().round() as i64;

    if secs == 0 {
        return "0s".to_string();
    }

    let days = secs / 86_400;
    secs %= 86_400;
    let hours = secs / 3_600;
    secs %= 3_600;
    let mins = secs / 60;
    secs %= 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if mins > 0 {
        parts.push(format!("{mins}m"));
    }
    if secs > 0 {
        parts.push(format!("{secs}s"));
    }
    let body = parts.join(" ");
    if negative { format!("-{body}") } else { body }
}

/// Turn a signed millisecond delta (now - then) into a relative phrase.
/// Positive deltas are in the past ("3 days ago"), negative in the future
/// ("in 3 days").
fn humanize_relative(delta_ms: i64) -> String {
    let future = delta_ms < 0;
    let secs = delta_ms.unsigned_abs() / 1000;

    let (value, unit) = if secs < 45 {
        return if future {
            "in a few seconds".to_string()
        } else {
            "just now".to_string()
        };
    } else if secs < 90 {
        (1, "minute")
    } else if secs < 3_600 {
        (secs / 60, "minute")
    } else if secs < 7_200 {
        (1, "hour")
    } else if secs < 86_400 {
        (secs / 3_600, "hour")
    } else if secs < 172_800 {
        (1, "day")
    } else if secs < 2_592_000 {
        (secs / 86_400, "day")
    } else if secs < 5_184_000 {
        (1, "month")
    } else if secs < 31_536_000 {
        (secs / 2_592_000, "month")
    } else if secs < 63_072_000 {
        (1, "year")
    } else {
        (secs / 31_536_000, "year")
    };

    let plural = if value == 1 { "" } else { "s" };
    if future {
        format!("in {value} {unit}{plural}")
    } else {
        format!("{value} {unit}{plural} ago")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: f64) -> FormatValue {
        FormatValue::Float(v)
    }

    #[test]
    fn number_with_separators_and_decimals() {
        let f = Format::Number {
            decimal_places: Some(2),
            thousands_separator: true,
        };
        assert_eq!(f.apply(&n(1234567.891)), "1,234,567.89");
        assert_eq!(f.apply(&FormatValue::Int(1000)), "1,000.00");
        assert_eq!(f.apply(&n(0.0)), "0.00");
    }

    #[test]
    fn number_negative_and_no_separator() {
        let f = Format::Number {
            decimal_places: Some(1),
            thousands_separator: false,
        };
        assert_eq!(f.apply(&n(-1234.56)), "-1234.6");
    }

    #[test]
    fn number_natural_precision_when_no_decimal_places() {
        let f = Format::Number {
            decimal_places: None,
            thousands_separator: true,
        };
        assert_eq!(f.apply(&FormatValue::Int(1234)), "1,234");
        assert_eq!(f.apply(&n(1234.5)), "1,234.5");
    }

    #[test]
    fn currency_default_prefix_and_negative() {
        let f = Format::Currency {
            currency: None,
            prefix: None,
            suffix: None,
            decimal_places: None,
            compact: false,
        };
        assert_eq!(f.apply(&n(1200.0)), "$1,200.00");
        assert_eq!(f.apply(&n(-5.0)), "-$5.00");
    }

    #[test]
    fn currency_compact() {
        let f = Format::Currency {
            currency: Some("USD".into()),
            prefix: Some("$".into()),
            suffix: None,
            decimal_places: None,
            compact: true,
        };
        assert_eq!(f.apply(&n(1_200_000.0)), "$1.2M");
    }

    #[test]
    fn percent_multiplies_by_100() {
        let f = Format::Percent {
            decimal_places: Some(1),
        };
        assert_eq!(f.apply(&n(0.1234)), "12.3%");
        assert_eq!(f.apply(&n(-0.5)), "-50.0%");
    }

    #[test]
    fn compact_magnitudes() {
        let f = Format::Compact;
        assert_eq!(f.apply(&n(999.0)), "999");
        assert_eq!(f.apply(&n(1500.0)), "1.5K");
        assert_eq!(f.apply(&n(3_400_000.0)), "3.4M");
        assert_eq!(f.apply(&n(5_600_000_000.0)), "5.6B");
        assert_eq!(f.apply(&n(7_800_000_000_000.0)), "7.8T");
        assert_eq!(f.apply(&n(-2_000.0)), "-2K");
    }

    #[test]
    fn enum_maps_then_falls_through() {
        let mut values = std::collections::BTreeMap::new();
        values.insert("paid".to_string(), "✓".to_string());
        let f = Format::Enum { values };
        assert_eq!(f.apply(&FormatValue::Text("paid".into())), "✓");
        assert_eq!(f.apply(&FormatValue::Text("other".into())), "other");
    }

    #[test]
    fn custom_template() {
        let f = Format::Custom {
            template: "${value} units".into(),
        };
        assert_eq!(f.apply(&FormatValue::Int(42)), "42 units");
    }

    #[test]
    fn duration_humanizes() {
        let f = Format::Duration {
            unit: DurationUnit::Seconds,
        };
        assert_eq!(f.apply(&FormatValue::Int(3723)), "1h 2m 3s");
        assert_eq!(f.apply(&FormatValue::Int(0)), "0s");
        let fm = Format::Duration {
            unit: DurationUnit::Milliseconds,
        };
        assert_eq!(fm.apply(&FormatValue::Int(90_000)), "1m 30s");
    }

    #[test]
    fn datetime_absolute_and_relative() {
        // 2026-01-02 03:04 UTC
        let dt = chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
            .unwrap()
            .timestamp_millis();
        let abs = Format::Datetime {
            format: None,
            relative: false,
        };
        assert_eq!(abs.apply(&FormatValue::Timestamp(dt)), "2026-01-02 03:04");

        let rel = Format::Datetime {
            format: None,
            relative: true,
        };
        let now = dt + 3 * 86_400_000; // three days later
        assert_eq!(rel.apply_at(&FormatValue::Timestamp(dt), now), "3 days ago");
        let earlier = dt - 2 * 3_600_000; // two hours before
        assert_eq!(
            rel.apply_at(&FormatValue::Timestamp(dt), earlier),
            "in 2 hours"
        );
    }

    #[test]
    fn datetime_parses_rfc3339_text() {
        let abs = Format::Datetime {
            format: Some("%Y-%m-%d".into()),
            relative: false,
        };
        assert_eq!(
            abs.apply(&FormatValue::Text("2026-06-24T12:00:00Z".into())),
            "2026-06-24"
        );
    }

    #[test]
    fn null_always_renders_as_dash() {
        let f = Format::Currency {
            currency: None,
            prefix: None,
            suffix: None,
            decimal_places: None,
            compact: false,
        };
        assert_eq!(f.apply(&FormatValue::Null), NULL_DISPLAY);
    }

    #[test]
    fn format_round_trips_through_json() {
        let f = Format::Currency {
            currency: Some("USD".into()),
            prefix: Some("$".into()),
            suffix: None,
            decimal_places: Some(0),
            compact: true,
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: Format = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
        // Minimal author-written form deserializes with defaults.
        let minimal: Format = serde_json::from_str(r#"{ "kind": "number" }"#).unwrap();
        assert_eq!(
            minimal,
            Format::Number {
                decimal_places: None,
                thousands_separator: true
            }
        );
    }
}

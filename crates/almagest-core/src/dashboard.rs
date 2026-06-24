// SPDX-License-Identifier: MIT OR Apache-2.0

//! The dashboard DSL — the typed model behind `almagest_dashboards.definition_json`.
//!
//! A dashboard is authored as JSON (GUI editor or by hand) and stored in the
//! file. These structs are the contract between authoring and rendering: serde
//! handles structural (de)serialization, and [`Dashboard::validate`] enforces
//! the semantic rules (unique ids, span bounds, parameter references, safe
//! templating) with field-pointed errors.
//!
//! The semantic model is lifted from Orrery's `.board` DSL (param kinds, panel
//! kinds, the span-12 grid, `{{param}}` templating, conditional visibility), but
//! the authored surface is JSON, not `.board`. **Almagest is embedded-only**, so
//! there are no connection fields here: a panel query is inline SQL over the
//! file's embedded tables, or a reference to a saved query — never a live
//! connection. (This corrects the pre-embedded-only example in `planning/05`.)

use crate::error::{AlmagestError, Result};
use serde::{Deserialize, Serialize};

/// The DSL version this build authors and validates. Bumped on breaking DSL
/// changes; a file carrying a newer version is refused with a clear error.
pub const DASHBOARD_DSL_VERSION: u32 = 1;

/// A complete dashboard definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dashboard {
    /// DSL version (must equal [`DASHBOARD_DSL_VERSION`]).
    pub version: u32,
    /// Display name.
    pub name: String,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional auto-refresh hint in seconds. Mostly moot for Almagest's static
    /// embedded data (re-running a query yields the same rows), but kept for
    /// model fidelity and forward use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_interval: Option<u32>,
    /// Declared parameters for user input.
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    /// Optional theme overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<Theme>,
    /// The grid layout and panels.
    pub layout: Layout,
}

/// A user-input parameter, referenced in SQL via `{{id}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    /// Unique identifier, referenced in queries.
    pub id: String,
    /// Parameter kind.
    pub kind: ParamKind,
    /// Human label for the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Default value (kind-specific shape; validated at resolution time, doc 07).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// Static option list for `select` / `multiselect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    /// Dynamic options query for `select` / `multiselect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options_query: Option<String>,
    /// Inclusive lower bound for `number`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Inclusive upper bound for `number`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Whether `select` offers an "All" choice.
    #[serde(default)]
    pub allow_all: bool,
}

/// The kinds of parameter input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// Free text.
    Text,
    /// Numeric input.
    Number,
    /// Checkbox / toggle.
    Boolean,
    /// A single date.
    Date,
    /// A start/end date range; exposes `{{id.start}}` and `{{id.end}}`.
    #[serde(rename = "daterange")]
    DateRange,
    /// A single choice from a list.
    Select,
    /// Multiple choices; `{{id}}` expands to a comma-separated list for `IN (...)`.
    #[serde(rename = "multiselect")]
    MultiSelect,
}

/// Theme palette and background overrides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    /// Series color palette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<Vec<String>>,
    /// Background color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
}

/// The grid layout: a column count and a sequence of rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// Number of grid columns (default 12).
    #[serde(default = "default_grid")]
    pub grid: u32,
    /// Rows of panels, top to bottom.
    pub rows: Vec<Row>,
}

fn default_grid() -> u32 {
    12
}

/// One row of panels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// Panels, left to right. A panel whose span exceeds the remaining width
    /// flows to the next line.
    pub panels: Vec<Panel>,
}

/// A single panel. Common fields are typed here; kind-specific configuration
/// (chart type, axes, formatting, sortability, …) is captured in `config` and
/// formalized by the component library (doc 06).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    /// Unique within the dashboard.
    pub id: String,
    /// Panel kind.
    pub kind: PanelKind,
    /// Header title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Grid columns occupied.
    pub span: u32,
    /// Data source (omitted for text panels).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<Query>,
    /// Optional parameter-driven visibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Visibility>,
    /// Kind-specific configuration, formalized in doc 06. Preserved verbatim so
    /// the definition round-trips losslessly even before doc 06 types exist.
    #[serde(flatten)]
    pub config: serde_json::Map<String, serde_json::Value>,
}

/// The kinds of panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    /// A single big-number metric.
    Metric,
    /// A chart (type in `config.chart_type`).
    Chart,
    /// A tabular result.
    Table,
    /// A static markdown/text block (no query).
    Text,
}

/// A panel's data source: inline SQL or a reference to a saved query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Query {
    /// Inline SQL over the file's embedded tables.
    Inline {
        /// The SQL, possibly with `{{param}}` templating.
        sql: String,
    },
    /// A reference to a row in `almagest_queries`.
    Reference {
        /// The saved query's id.
        query_id: String,
    },
}

/// Parameter-driven visibility: show the panel only when a parameter equals a
/// value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Visibility {
    /// The equality condition.
    pub equals: VisibilityEquals,
}

/// The `{ "param": id, "value": v }` condition behind [`Visibility`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisibilityEquals {
    /// The parameter id to test.
    pub param: String,
    /// The value it must equal for the panel to show.
    pub value: serde_json::Value,
}

impl Dashboard {
    /// Parse a dashboard from JSON and validate it. Structural errors come from
    /// serde; semantic errors come from [`Dashboard::validate`].
    pub fn from_json(json: &str) -> Result<Self> {
        let dash: Dashboard = serde_json::from_str(json)?;
        dash.validate()?;
        Ok(dash)
    }

    /// Serialize to compact JSON (validating first).
    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        Ok(serde_json::to_string(self)?)
    }

    /// Serialize to pretty, git-diffable JSON (validating first).
    pub fn to_json_pretty(&self) -> Result<String> {
        self.validate()?;
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Enforce the semantic rules. Returns the first violation with a field path.
    pub fn validate(&self) -> Result<()> {
        if self.version != DASHBOARD_DSL_VERSION {
            return Err(err(
                "version",
                format!(
                    "dashboard DSL version {} is not supported (this build authors version {})",
                    self.version, DASHBOARD_DSL_VERSION
                ),
            ));
        }
        if self.name.trim().is_empty() {
            return Err(err("name", "dashboard name must not be empty"));
        }
        if self.layout.grid == 0 {
            return Err(err("layout.grid", "grid must have at least one column"));
        }

        self.validate_parameters()?;
        self.validate_panels()?;
        Ok(())
    }

    fn validate_parameters(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for (i, p) in self.parameters.iter().enumerate() {
            let loc = format!("parameters[{i}]");
            if p.id.trim().is_empty() {
                return Err(err(&loc, "parameter id must not be empty"));
            }
            if !seen.insert(p.id.as_str()) {
                return Err(err(&loc, format!("duplicate parameter id '{}'", p.id)));
            }
            if matches!(p.kind, ParamKind::Select | ParamKind::MultiSelect)
                && p.options.is_none()
                && p.options_query.is_none()
            {
                return Err(err(
                    &loc,
                    format!(
                        "{} parameter '{}' needs either options or options_query",
                        kind_label(p.kind),
                        p.id
                    ),
                ));
            }
            if let (Some(min), Some(max)) = (p.min, p.max)
                && min > max
            {
                return Err(err(&loc, format!("parameter '{}' has min > max", p.id)));
            }
        }
        Ok(())
    }

    fn validate_panels(&self) -> Result<()> {
        let param_ids: std::collections::HashSet<&str> =
            self.parameters.iter().map(|p| p.id.as_str()).collect();
        let daterange_ids: std::collections::HashSet<&str> = self
            .parameters
            .iter()
            .filter(|p| p.kind == ParamKind::DateRange)
            .map(|p| p.id.as_str())
            .collect();

        let mut seen_panels = std::collections::HashSet::new();
        for (ri, row) in self.layout.rows.iter().enumerate() {
            for (pi, panel) in row.panels.iter().enumerate() {
                let loc = format!("layout.rows[{ri}].panels[{pi}]");
                if panel.id.trim().is_empty() {
                    return Err(err(&loc, "panel id must not be empty"));
                }
                if !seen_panels.insert(panel.id.as_str()) {
                    return Err(err(&loc, format!("duplicate panel id '{}'", panel.id)));
                }
                if panel.span == 0 || panel.span > self.layout.grid {
                    return Err(err(
                        &loc,
                        format!(
                            "panel '{}' span {} must be between 1 and the grid width {}",
                            panel.id, panel.span, self.layout.grid
                        ),
                    ));
                }
                // Data panels need a query; text panels don't.
                if panel.kind != PanelKind::Text && panel.query.is_none() {
                    return Err(err(
                        &loc,
                        format!(
                            "panel '{}' of kind {} requires a query",
                            panel.id,
                            kind_label_panel(panel.kind)
                        ),
                    ));
                }
                // Visibility must reference a declared parameter.
                if let Some(vis) = &panel.visible
                    && !param_ids.contains(vis.equals.param.as_str())
                {
                    return Err(err(
                        &loc,
                        format!(
                            "panel '{}' visibility references unknown parameter '{}'",
                            panel.id, vis.equals.param
                        ),
                    ));
                }
                // Templating: every {{ref}} must resolve to a declared parameter,
                // and a sub-field (id.start/id.end) is only valid for daterange.
                if let Some(Query::Inline { sql }) = &panel.query {
                    for token in referenced_params(sql) {
                        let (base, sub) = match token.split_once('.') {
                            Some((b, s)) => (b, Some(s)),
                            None => (token.as_str(), None),
                        };
                        if !param_ids.contains(base) {
                            return Err(err(
                                &loc,
                                format!(
                                    "panel '{}' query references unknown parameter '{{{{{base}}}}}'",
                                    panel.id
                                ),
                            ));
                        }
                        if let Some(sub) = sub {
                            if !daterange_ids.contains(base) {
                                return Err(err(
                                    &loc,
                                    format!(
                                        "panel '{}' references '{{{{{base}.{sub}}}}}' but '{base}' is not a daterange",
                                        panel.id
                                    ),
                                ));
                            }
                            if sub != "start" && sub != "end" {
                                return Err(err(
                                    &loc,
                                    format!(
                                        "panel '{}' daterange sub-field '{sub}' must be 'start' or 'end'",
                                        panel.id
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Build an `InvalidDashboard` error at `location`.
fn err(location: &str, detail: impl Into<String>) -> AlmagestError {
    AlmagestError::InvalidDashboard {
        location: location.to_string(),
        detail: detail.into(),
    }
}

fn kind_label(k: ParamKind) -> &'static str {
    match k {
        ParamKind::Select => "select",
        ParamKind::MultiSelect => "multiselect",
        _ => "parameter",
    }
}

fn kind_label_panel(k: PanelKind) -> &'static str {
    match k {
        PanelKind::Metric => "metric",
        PanelKind::Chart => "chart",
        PanelKind::Table => "table",
        PanelKind::Text => "text",
    }
}

/// Extract the base names referenced by `{{...}}` tokens in `sql` (the trimmed
/// content of each pair of braces). Mirrors the engine's substitution scanner.
fn referenced_params(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = sql;
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        match after.find("}}") {
            Some(close) => {
                out.push(after[..close].trim().to_string());
                rest = &after[close + 2..];
            }
            None => break,
        }
    }
    out
}

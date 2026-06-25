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
use almagest_format::Format;
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
    /// Optional help text shown beneath the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
    /// Minimum chosen items for `multiselect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_selections: Option<usize>,
    /// Maximum chosen items for `multiselect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_selections: Option<usize>,
    /// Whether `select` offers an "All" choice.
    #[serde(default)]
    pub allow_all: bool,
    /// Where this parameter's value persists across loads (doc 07).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persist: Option<Persist>,
}

/// Where a parameter value is preserved across loads. Drives the resolution
/// priority (URL > file > declared default); see `planning/07`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Persist {
    /// In memory for the current tab/session; reset on reload (default).
    Session,
    /// Encoded in the URL query string; shareable via link.
    Url,
    /// Saved into the file as the new default; persists across sessions/users.
    File,
    /// Always reset to the declared default on each load.
    None,
}

/// The kinds of parameter input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// Free text.
    Text,
    /// Numeric input.
    Number,
    /// Checkbox / toggle. The `toggle` alias deserializes to this kind (the doc
    /// lists "toggle" as an on/off rendering of a boolean).
    #[serde(alias = "toggle")]
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

/// A single panel. Common fields are typed here; the `kind` tag and its
/// kind-specific configuration are flattened in via [`PanelKind`] (formalized
/// in doc 06 — these were previously an opaque `config` map).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    /// Unique within the dashboard.
    pub id: String,
    /// Header title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional subtitle / tooltip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Grid columns occupied.
    pub span: u32,
    /// Data source (omitted for text / image / divider panels).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<Query>,
    /// Optional parameter-driven visibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Visibility>,
    /// Declarative interactions fired on user action (row/point click).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_click: Vec<Action>,
    /// The panel kind and its typed configuration. The `kind` discriminator and
    /// the kind-specific fields are flattened to the panel's top level, matching
    /// the DSL JSON (`{ "kind": "chart", "chart_type": "bar", "x": … }`).
    #[serde(flatten)]
    pub kind: PanelKind,
}

/// The kinds of panel and their typed configuration. Internally tagged on
/// `kind`; the variant's config fields sit alongside the tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PanelKind {
    /// A single big-number KPI.
    Metric(MetricConfig),
    /// A chart (line / bar / area / donut / pie / scatter).
    Chart(ChartConfig),
    /// A tabular result.
    Table(TableConfig),
    /// A static markdown/text block (no query).
    Text(TextConfig),
    /// A static image drawn from `almagest_assets` (no query).
    Image(ImageConfig),
    /// A visual separator (no query).
    Divider(DividerConfig),
}

impl PanelKind {
    /// The kind's snake_case name, for messages.
    pub fn name(&self) -> &'static str {
        match self {
            PanelKind::Metric(_) => "metric",
            PanelKind::Chart(_) => "chart",
            PanelKind::Table(_) => "table",
            PanelKind::Text(_) => "text",
            PanelKind::Image(_) => "image",
            PanelKind::Divider(_) => "divider",
        }
    }

    /// Whether this kind draws its content from a query. Text, image, and
    /// divider panels are static; the rest require a query.
    pub fn needs_query(&self) -> bool {
        matches!(
            self,
            PanelKind::Metric(_) | PanelKind::Chart(_) | PanelKind::Table(_)
        )
    }
}

/// KPI configuration: how to format the single value and, optionally, compare
/// it to a previous-period value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MetricConfig {
    /// Display format for the `value` column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    /// Optional comparison to a previous value for a trend indicator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<Comparison>,
}

/// A metric's comparison to a previous-period value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// The result column holding the previous value.
    pub previous_field: String,
    /// How to render the delta.
    #[serde(default)]
    pub delta_format: DeltaFormat,
    /// Which direction counts as "good" (drives coloring).
    #[serde(default)]
    pub direction: TrendDirection,
}

/// How a metric delta is rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaFormat {
    /// Percentage change (default).
    #[default]
    Percent,
    /// Absolute difference.
    Absolute,
}

/// Which direction of change is "good", for direction-aware coloring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    /// Up is good (default).
    #[default]
    HigherBetter,
    /// Down is good.
    LowerBetter,
    /// Neither — show change without good/bad coloring.
    Neutral,
}

/// Chart configuration. Cartesian charts (line/bar/area/scatter) use `x`/`y`
/// (plus optional `series`); proportional charts (donut/pie) use
/// `category`/`value`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartConfig {
    /// The chart type.
    pub chart_type: ChartType,
    /// X-axis column (cartesian charts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    /// Y-axis column (cartesian charts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
    /// Grouping column for multi-series (cartesian charts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    /// Category column (donut/pie).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Value column (donut/pie).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Bar orientation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<Orientation>,
    /// Category ordering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<ChartSort>,
    /// Stack series (area / bar).
    #[serde(default)]
    pub stacked: bool,
    /// Show percentage labels (donut/pie).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_percent: Option<bool>,
    /// Show the legend (renderer default when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_legend: Option<bool>,
    /// Show the background grid (renderer default when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_grid: Option<bool>,
    /// Format for X-axis / category values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_format: Option<Format>,
    /// Format for Y-axis / value numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_format: Option<Format>,
}

/// The chart types Almagest renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartType {
    /// Time series / continuous line.
    Line,
    /// Categorical bars.
    Bar,
    /// Filled line.
    Area,
    /// Donut (ring) proportion.
    Donut,
    /// Pie proportion.
    Pie,
    /// Two-dimensional scatter.
    Scatter,
}

impl ChartType {
    /// Cartesian charts plot `x`/`y`; proportional charts plot
    /// `category`/`value`.
    fn is_cartesian(self) -> bool {
        matches!(
            self,
            ChartType::Line | ChartType::Bar | ChartType::Area | ChartType::Scatter
        )
    }
}

/// Bar orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// Bars rise along the Y-axis.
    Vertical,
    /// Bars extend along the X-axis.
    Horizontal,
}

/// Category ordering for bar/line charts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartSort {
    /// Ascending by the X value.
    AscByX,
    /// Descending by the X value.
    DescByX,
    /// Ascending by the Y value.
    AscByY,
    /// Descending by the Y value.
    DescByY,
}

/// Table configuration: per-column formatting, sorting, and pagination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TableConfig {
    /// Per-column overrides, keyed by result column name.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub columns: std::collections::BTreeMap<String, ColumnConfig>,
    /// Allow the viewer to sort by clicking column headers.
    #[serde(default)]
    pub sortable: bool,
    /// Initial sort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_default: Option<SortSpec>,
    /// Rows per page (no pagination when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
}

/// Per-column display configuration for a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ColumnConfig {
    /// Header label (defaults to the column name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Fixed column width (CSS length, e.g. `120px`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<String>,
    /// Cell value format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
}

/// A column + direction sort specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SortSpec {
    /// Result column to sort on.
    pub column: String,
    /// Sort direction.
    #[serde(default)]
    pub direction: SortDirection,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    /// Ascending (default).
    #[default]
    Asc,
    /// Descending.
    Desc,
}

/// Markdown text-block configuration. Accepts `content` (canonical) or the
/// `markdown` alias.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TextConfig {
    /// The markdown body.
    #[serde(default, alias = "markdown")]
    pub content: String,
}

/// Static-image configuration; `asset_path` names an entry in `almagest_assets`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageConfig {
    /// The asset name/path inside the file.
    pub asset_path: String,
    /// Alt text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}

/// Divider configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DividerConfig {
    /// Optional section label drawn on the rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A declarative interaction fired on user action. Tagged on `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Update a parameter on the current dashboard, triggering dependent
    /// re-queries. `value` may be a literal or a `$row.<col>` template.
    SetParameter {
        /// The parameter id to update.
        parameter: String,
        /// The value to set (literal or `$row.<column>` token).
        value: serde_json::Value,
    },
    /// Navigate to another dashboard in the same file.
    NavigateTo {
        /// The target dashboard id.
        dashboard: String,
    },
    /// Open an external URL (http/https/mailto only).
    OpenUrl {
        /// The URL to open.
        url: String,
    },
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
                // Data panels need a query; text / image / divider don't.
                if panel.kind.needs_query() && panel.query.is_none() {
                    return Err(err(
                        &loc,
                        format!(
                            "panel '{}' of kind {} requires a query",
                            panel.id,
                            panel.kind.name()
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

                // Kind-specific configuration rules.
                validate_panel_kind(panel, &loc)?;

                // Interactions: set_parameter must target a declared parameter;
                // open_url must use a safe scheme.
                for (ai, action) in panel.on_click.iter().enumerate() {
                    let aloc = format!("{loc}.on_click[{ai}]");
                    match action {
                        Action::SetParameter { parameter, .. } => {
                            if !param_ids.contains(parameter.as_str()) {
                                return Err(err(
                                    &aloc,
                                    format!(
                                        "panel '{}' set_parameter targets unknown parameter '{}'",
                                        panel.id, parameter
                                    ),
                                ));
                            }
                        }
                        Action::NavigateTo { dashboard } => {
                            if dashboard.trim().is_empty() {
                                return Err(err(
                                    &aloc,
                                    format!(
                                        "panel '{}' navigate_to needs a dashboard id",
                                        panel.id
                                    ),
                                ));
                            }
                        }
                        Action::OpenUrl { url } => {
                            if !is_safe_external_url(url) {
                                return Err(err(
                                    &aloc,
                                    format!(
                                        "panel '{}' open_url '{}' must be an http(s) or mailto URL",
                                        panel.id, url
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

/// Validate the typed configuration that is specific to a panel's kind.
fn validate_panel_kind(panel: &Panel, loc: &str) -> Result<()> {
    match &panel.kind {
        PanelKind::Metric(cfg) => {
            if let Some(c) = &cfg.comparison
                && c.previous_field.trim().is_empty()
            {
                return Err(err(
                    loc,
                    format!(
                        "metric panel '{}' comparison.previous_field must not be empty",
                        panel.id
                    ),
                ));
            }
        }
        PanelKind::Chart(cfg) => {
            if cfg.chart_type.is_cartesian() {
                if cfg.x.is_none() || cfg.y.is_none() {
                    return Err(err(
                        loc,
                        format!(
                            "{} chart '{}' requires both 'x' and 'y'",
                            chart_type_name(cfg.chart_type),
                            panel.id
                        ),
                    ));
                }
            } else if cfg.category.is_none() || cfg.value.is_none() {
                return Err(err(
                    loc,
                    format!(
                        "{} chart '{}' requires both 'category' and 'value'",
                        chart_type_name(cfg.chart_type),
                        panel.id
                    ),
                ));
            }
        }
        PanelKind::Image(cfg) => {
            if cfg.asset_path.trim().is_empty() {
                return Err(err(
                    loc,
                    format!("image panel '{}' requires an asset_path", panel.id),
                ));
            }
        }
        PanelKind::Table(_) | PanelKind::Text(_) | PanelKind::Divider(_) => {}
    }
    Ok(())
}

/// A URL is safe to open from a dashboard interaction only if it uses an
/// `http`, `https`, or `mailto` scheme. This blocks `javascript:`, `data:`,
/// `file:`, and other schemes that are injection or exfiltration vectors when an
/// authored dashboard is opened in a viewer.
fn is_safe_external_url(url: &str) -> bool {
    let lowered = url.trim().to_ascii_lowercase();
    lowered.starts_with("http://")
        || lowered.starts_with("https://")
        || lowered.starts_with("mailto:")
}

fn chart_type_name(t: ChartType) -> &'static str {
    match t {
        ChartType::Line => "line",
        ChartType::Bar => "bar",
        ChartType::Area => "area",
        ChartType::Donut => "donut",
        ChartType::Pie => "pie",
        ChartType::Scatter => "scatter",
    }
}

impl Panel {
    /// The result columns this panel needs from its query. Empty for static
    /// panels (text / image / divider) and for tables (which display whatever
    /// columns the query returns). The renderer uses this to detect a query
    /// whose shape doesn't match the panel before drawing — a friendlier error
    /// than a blank chart. The `"value"` literal mirrors the metric contract
    /// (one row, a `value` column) from `planning/06`.
    pub fn required_columns(&self) -> Vec<&str> {
        match &self.kind {
            PanelKind::Metric(cfg) => {
                let mut cols = vec!["value"];
                if let Some(c) = &cfg.comparison {
                    cols.push(c.previous_field.as_str());
                }
                cols
            }
            PanelKind::Chart(cfg) => {
                let mut cols = Vec::new();
                if cfg.chart_type.is_cartesian() {
                    cols.extend(cfg.x.as_deref());
                    cols.extend(cfg.y.as_deref());
                    cols.extend(cfg.series.as_deref());
                } else {
                    cols.extend(cfg.category.as_deref());
                    cols.extend(cfg.value.as_deref());
                }
                cols
            }
            PanelKind::Table(_)
            | PanelKind::Text(_)
            | PanelKind::Image(_)
            | PanelKind::Divider(_) => Vec::new(),
        }
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

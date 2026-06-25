// SPDX-License-Identifier: MIT OR Apache-2.0
//
// TypeScript mirror of the dashboard DSL (almagest-core `dashboard.rs`) and the
// format spec (`almagest-format`). These shapes match the JSON the server emits
// and accepts; keep them in sync with the Rust types.

// --- format ------------------------------------------------------------------

export type DurationUnit = "milliseconds" | "seconds" | "minutes" | "hours";

export type Format =
  | { kind: "number"; decimal_places?: number; thousands_separator?: boolean }
  | {
      kind: "currency";
      currency?: string;
      prefix?: string;
      suffix?: string;
      decimal_places?: number;
      compact?: boolean;
    }
  | { kind: "percent"; decimal_places?: number }
  | { kind: "compact" }
  | { kind: "datetime"; format?: string; relative?: boolean }
  | { kind: "duration"; unit?: DurationUnit }
  | { kind: "enum"; values: Record<string, string> }
  | { kind: "custom"; template: string };

// --- parameters --------------------------------------------------------------

export type ParamKind =
  | "text"
  | "number"
  | "boolean"
  | "date"
  | "daterange"
  | "select"
  | "multiselect";

export type Persist = "session" | "url" | "file" | "none";

export interface Parameter {
  id: string;
  kind: ParamKind;
  label?: string;
  description?: string;
  default?: unknown;
  options?: string[];
  options_query?: string;
  min?: number;
  max?: number;
  min_selections?: number;
  max_selections?: number;
  allow_all?: boolean;
  persist?: Persist;
}

// --- panels ------------------------------------------------------------------

export type Query = { sql: string } | { query_id: string };

export interface Visibility {
  equals: { param: string; value: unknown };
}

export type Action =
  | { kind: "set_parameter"; parameter: string; value: unknown }
  | { kind: "navigate_to"; dashboard: string }
  | { kind: "open_url"; url: string };

export type DeltaFormat = "percent" | "absolute";
export type TrendDirection = "higher_better" | "lower_better" | "neutral";

export interface Comparison {
  previous_field: string;
  delta_format?: DeltaFormat;
  direction?: TrendDirection;
}

export type ChartType = "line" | "bar" | "area" | "donut" | "pie" | "scatter";
export type Orientation = "vertical" | "horizontal";
export type ChartSort = "asc_by_x" | "desc_by_x" | "asc_by_y" | "desc_by_y";
export type SortDirection = "asc" | "desc";

export interface ColumnConfig {
  label?: string;
  width?: string;
  format?: Format;
}

export interface SortSpec {
  column: string;
  direction?: SortDirection;
}

interface PanelCommon {
  id: string;
  title?: string;
  description?: string;
  span: number;
  query?: Query;
  visible?: Visibility;
  on_click?: Action[];
}

export type MetricPanel = PanelCommon & {
  kind: "metric";
  format?: Format;
  comparison?: Comparison;
};

export type ChartPanel = PanelCommon & {
  kind: "chart";
  chart_type: ChartType;
  x?: string;
  y?: string;
  series?: string;
  category?: string;
  value?: string;
  orientation?: Orientation;
  sort?: ChartSort;
  stacked?: boolean;
  show_percent?: boolean;
  show_legend?: boolean;
  show_grid?: boolean;
  x_format?: Format;
  y_format?: Format;
};

export type TablePanel = PanelCommon & {
  kind: "table";
  columns?: Record<string, ColumnConfig>;
  sortable?: boolean;
  sort_default?: SortSpec;
  page_size?: number;
};

export type TextPanel = PanelCommon & { kind: "text"; content: string };
export type ImagePanel = PanelCommon & { kind: "image"; asset_path: string; alt?: string };
export type DividerPanel = PanelCommon & { kind: "divider"; label?: string };

export type Panel =
  | MetricPanel
  | ChartPanel
  | TablePanel
  | TextPanel
  | ImagePanel
  | DividerPanel;

export const PANEL_KINDS: Panel["kind"][] = [
  "metric",
  "chart",
  "table",
  "text",
  "image",
  "divider",
];

/** Panel kinds that draw their content from a query. */
export function panelNeedsQuery(kind: Panel["kind"]): boolean {
  return kind === "metric" || kind === "chart" || kind === "table";
}

export interface Theme {
  palette?: string[];
  background?: string;
}

export interface Row {
  panels: Panel[];
}

export interface Layout {
  grid?: number;
  rows: Row[];
}

export interface Dashboard {
  version: number;
  name: string;
  description?: string;
  refresh_interval?: number;
  parameters?: Parameter[];
  theme?: Theme;
  layout: Layout;
}

export const DASHBOARD_DSL_VERSION = 1;

// --- server DTOs -------------------------------------------------------------

export interface AlmagestMeta {
  id: string;
  title: string;
  description: string;
  format_version: number;
  server_version: string;
  dashboard_count: number;
  read_only: boolean;
  heartbeat_enabled: boolean;
  auth_enabled: boolean;
}

// --- auth & multi-user (doc 13) ----------------------------------------------

export type Role = "admin" | "editor" | "viewer";

export interface User {
  id: string;
  username: string;
  role: Role;
  email: string | null;
  created_at: string;
  last_login_at: string | null;
}

/** Response of `GET /auth/me` — the SPA's auth bootstrap probe. */
export interface AuthMe {
  auth_enabled: boolean;
  needs_setup: boolean;
  user: User | null;
}

/** Response of login / setup — the user plus the CSRF token to echo back. */
export interface AuthSession {
  user: User;
  csrf_token: string;
}

export interface HistoryEntry {
  id: number;
  event_kind: string;
  entity_id: string | null;
  user_id: string | null;
  payload_json: string | null;
  occurred_at: string;
}

export interface DashboardSummary {
  id: string;
  name: string;
  description: string | null;
  folder: string | null;
  created_at: string;
  updated_at: string;
}

export interface ColumnSchema {
  name: string;
  data_type: string;
  nullable: boolean;
}

export interface TableSchema {
  name: string;
  columns: ColumnSchema[];
  row_count: number;
}

export interface DatabaseSchema {
  tables: TableSchema[];
}

export interface AssetEntry {
  path: string;
  content_type: string;
}

export interface DatasetInfo {
  name: string;
  row_count: number;
  byte_size: number;
  compression: string;
  columns: ColumnSchema[];
  source: unknown | null;
}

export interface IngestedDataset {
  name: string;
  row_count: number;
  byte_size: number;
  rows_skipped: number;
  warnings: string[];
  columns: ColumnSchema[];
}

export interface IngestResult {
  dry_run: boolean;
  datasets: IngestedDataset[];
}

export interface IngestOptions {
  filename?: string;
  name?: string;
  format?: string;
  replace?: boolean;
  dryRun?: boolean;
  jsonFormat?: string;
  noHeader?: boolean;
  delimiter?: string;
}

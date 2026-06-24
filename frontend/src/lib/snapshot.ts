// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Static HTML snapshot export (doc 10). Produces a single self-contained `.html`
// file of the current view: the dashboard title, the parameter values in effect,
// and every panel's results frozen in place — charts rasterized to PNG data URLs,
// tables / metrics / text rendered as static HTML, images inlined as data URLs.
// No JavaScript, no live queries, no Almagest install needed to open it. This is
// the *static* tier of the export ladder (the interactive DuckDB-WASM tier is a
// later deployment-mode phase); freezing results keeps the snapshot tiny and
// guaranteed to open identically in any browser, Preview, or mobile viewer.

import type {
  Dashboard,
  Panel,
  Parameter,
  MetricPanel,
  TablePanel,
  TextPanel,
  ImagePanel,
  DividerPanel,
} from "./types";
import { panelNeedsQuery } from "./types";
import { api, assetUrl } from "./api";
import { applyFormat, toNumber, plain, NULL_DISPLAY, DEFAULT_PALETTE } from "./format";
import { buildChartOption } from "./chartOption";
import { echarts, type EChartsOption } from "./echarts";
import { marked } from "marked";
import DOMPurify from "dompurify";

const MAX_TABLE_ROWS = 2000;

/** Escape a string for safe interpolation into HTML text / attributes. */
function esc(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** Render an off-screen ECharts chart to a PNG data URL. */
function chartToDataUrl(option: EChartsOption, width: number, height: number): string {
  const host = document.createElement("div");
  host.style.cssText = `position:absolute;left:-99999px;top:0;width:${width}px;height:${height}px`;
  document.body.appendChild(host);
  const chart = echarts.init(host, undefined, { width, height });
  try {
    chart.setOption({ ...option, animation: false } as EChartsOption);
    return chart.getDataURL({ type: "png", pixelRatio: 2, backgroundColor: "#fff" });
  } finally {
    chart.dispose();
    host.remove();
  }
}

/** Fetch an embedded asset and inline it as a data URL. */
async function assetToDataUrl(path: string): Promise<string | null> {
  try {
    const res = await fetch(assetUrl(path));
    if (!res.ok) return null;
    const blob = await res.blob();
    return await new Promise((resolve) => {
      const reader = new FileReader();
      reader.onloadend = () => resolve(reader.result as string);
      reader.onerror = () => resolve(null);
      reader.readAsDataURL(blob);
    });
  } catch {
    return null;
  }
}

function metricHtml(panel: MetricPanel, rows: Record<string, unknown>[]): string {
  const row = rows[0] ?? null;
  const value = row ? row["value"] : null;
  const display = value === undefined || value === null ? NULL_DISPLAY : applyFormat(panel.format, value);

  let deltaHtml = "";
  if (panel.comparison && row) {
    const cur = toNumber(value);
    const prev = toNumber(row[panel.comparison.previous_field]);
    if (cur !== null && prev !== null) {
      const fmt = panel.comparison.delta_format ?? "percent";
      const raw = fmt === "percent" ? (prev === 0 ? null : (cur - prev) / prev) : cur - prev;
      if (raw !== null) {
        const text =
          fmt === "percent"
            ? `${raw >= 0 ? "+" : ""}${(raw * 100).toFixed(1)}%`
            : `${raw >= 0 ? "+" : ""}${applyFormat(panel.format, cur - prev)}`;
        const dir = panel.comparison.direction ?? "higher_better";
        let tone = "neutral";
        if (dir !== "neutral" && raw !== 0) {
          const good = dir === "higher_better" ? raw > 0 : raw < 0;
          tone = good ? "good" : "bad";
        }
        deltaHtml = `<div class="delta ${tone}">${esc(text)}</div>`;
      }
    }
  }
  return `<div class="metric"><div class="value">${esc(display)}</div>${deltaHtml}</div>`;
}

function tableHtml(panel: TablePanel, fields: string[], rows: Record<string, unknown>[]): string {
  let sorted = rows;
  if (panel.sort_default) {
    const col = panel.sort_default.column;
    const dir = (panel.sort_default.direction ?? "asc") === "asc" ? 1 : -1;
    sorted = [...rows].sort((a, b) => {
      const na = toNumber(a[col]);
      const nb = toNumber(b[col]);
      if (na !== null && nb !== null) return dir * (na - nb);
      return dir * plain(a[col]).localeCompare(plain(b[col]));
    });
  }
  const truncated = sorted.length > MAX_TABLE_ROWS;
  const shown = truncated ? sorted.slice(0, MAX_TABLE_ROWS) : sorted;

  const head = fields
    .map((c) => `<th>${esc(panel.columns?.[c]?.label ?? c)}</th>`)
    .join("");
  const body = shown
    .map(
      (r) =>
        `<tr>${fields
          .map((c) => `<td>${esc(applyFormat(panel.columns?.[c]?.format, r[c]))}</td>`)
          .join("")}</tr>`,
    )
    .join("");
  const note = truncated
    ? `<div class="trunc">Showing first ${MAX_TABLE_ROWS.toLocaleString()} of ${sorted.length.toLocaleString()} rows.</div>`
    : "";
  return `<div class="table-wrap"><table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>${note}`;
}

function textHtml(panel: TextPanel): string {
  return `<div class="text">${DOMPurify.sanitize(
    marked.parse(panel.content ?? "", { async: false }) as string,
  )}</div>`;
}

function dividerHtml(panel: DividerPanel): string {
  return panel.label
    ? `<div class="divider"><span>${esc(panel.label)}</span></div>`
    : `<hr class="divider-line" />`;
}

/** Render one panel to static HTML (executing its query if it has one). */
async function panelHtml(
  dashboard: Dashboard,
  dashboardId: string,
  panel: Panel,
  paramValues: Record<string, unknown>,
  spanWidth: number,
): Promise<string> {
  const palette = dashboard.theme?.palette ?? DEFAULT_PALETTE;
  let inner = "";

  try {
    if (panel.kind === "text") {
      inner = textHtml(panel);
    } else if (panel.kind === "divider") {
      inner = dividerHtml(panel);
    } else if (panel.kind === "image") {
      const ip = panel as ImagePanel;
      const dataUrl = await assetToDataUrl(ip.asset_path);
      inner = dataUrl
        ? `<img class="img" src="${dataUrl}" alt="${esc(ip.alt ?? "")}" />`
        : `<div class="empty">Image unavailable</div>`;
    } else if (panelNeedsQuery(panel.kind) && panel.query) {
      const result = await api.executePanel(dashboardId, panel.id, paramValues);
      if (panel.kind === "metric") {
        inner = metricHtml(panel, result.rows);
      } else if (panel.kind === "table") {
        inner = tableHtml(panel, result.fields.map((f) => f.name), result.rows);
      } else if (panel.kind === "chart") {
        const { option } = buildChartOption(panel, result.rows, palette);
        const url = chartToDataUrl(option, Math.max(320, spanWidth), 300);
        inner = `<img class="chart-img" src="${url}" alt="${esc(panel.title ?? "chart")}" />`;
      }
    }
  } catch (e) {
    inner = `<div class="empty err">${esc(e instanceof Error ? e.message : String(e))}</div>`;
  }

  const header =
    panel.title || panel.description
      ? `<header>${panel.title ? `<h3>${esc(panel.title)}</h3>` : ""}${
          panel.description ? `<p class="desc">${esc(panel.description)}</p>` : ""
        }</header>`
      : "";
  return `<div class="panel">${header}<div class="body">${inner}</div></div>`;
}

function paramSummary(parameters: Parameter[], values: Record<string, unknown>): string {
  const chips = parameters
    .map((p) => {
      const v = values[p.id];
      if (v === undefined || v === null || v === "") return null;
      let text: string;
      if (Array.isArray(v)) text = v.join(", ");
      else if (typeof v === "object") {
        const o = v as Record<string, unknown>;
        text = o.preset ? String(o.preset) : `${o.start ?? ""} → ${o.end ?? ""}`;
      } else text = String(v);
      return `<span class="chip"><b>${esc(p.label ?? p.id)}:</b> ${esc(text)}</span>`;
    })
    .filter(Boolean);
  return chips.length ? `<div class="params">${chips.join("")}</div>` : "";
}

const SNAPSHOT_CSS = `
*{box-sizing:border-box}
body{margin:0;background:#f8f9fa;color:#212529;font-family:system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
.wrap{max-width:1200px;margin:0 auto;padding:1.5rem}
.top{display:flex;align-items:baseline;gap:1rem;flex-wrap:wrap;margin-bottom:.25rem}
h1{margin:0;font-size:1.4rem}
.stamp{color:#868e96;font-size:.8rem;margin-left:auto}
.params{display:flex;flex-wrap:wrap;gap:.5rem;margin:.5rem 0 1rem}
.chip{background:#fff;border:1px solid #e9ecef;border-radius:999px;padding:.2rem .7rem;font-size:.8rem}
.grid-row{display:grid;gap:1rem;align-items:stretch;margin-bottom:1rem}
.cell{min-width:0}
.panel{background:#fff;border:1px solid #e9ecef;border-radius:10px;padding:.85rem 1rem;height:100%;overflow:hidden}
header{margin-bottom:.5rem}
h3{margin:0;font-size:.95rem;font-weight:650}
.desc{margin:.15rem 0 0;font-size:.8rem;color:#868e96}
.metric .value{font-size:2.2rem;font-weight:650;letter-spacing:-.02em;line-height:1.1}
.delta{font-size:.9rem;font-weight:600;margin-top:.35rem}
.delta.good{color:#2f9e44}.delta.bad{color:#e03131}.delta.neutral{color:#868e96}
.chart-img,.img{max-width:100%;height:auto;display:block}
.table-wrap{overflow:auto}
table{border-collapse:collapse;width:100%;font-size:.85rem}
th,td{text-align:left;padding:.35rem .6rem;border-bottom:1px solid #e9ecef;white-space:nowrap}
th{background:#fff;font-weight:650;color:#495057}
.trunc{color:#868e96;font-size:.75rem;margin-top:.4rem}
.text{line-height:1.5;font-size:.95rem}
.divider{display:flex;align-items:center;gap:.6rem;color:#868e96;font-size:.8rem;text-transform:uppercase;letter-spacing:.04em}
.divider::before,.divider::after{content:"";flex:1;height:1px;background:#e9ecef}
.divider-line{border:none;border-top:1px solid #e9ecef}
.empty{color:#868e96;font-size:.85rem}.empty.err{color:#e03131;white-space:pre-wrap}
@media(max-width:600px){.grid-row{grid-template-columns:1fr!important}.cell{grid-column:auto!important}}
`;

/** Build, then download, a static HTML snapshot of the current dashboard view. */
export async function exportSnapshot(
  dashboard: Dashboard,
  dashboardId: string,
  paramValues: Record<string, unknown>,
  now: Date = new Date(),
): Promise<void> {
  const grid = dashboard.layout.grid ?? 12;
  const containerWidth = 1136; // 1200 max-width minus padding
  const gap = 16;

  const rowsHtml: string[] = [];
  for (const row of dashboard.layout.rows) {
    const cells: string[] = [];
    for (const panel of row.panels) {
      const span = Math.min(panel.span, grid);
      const spanWidth = Math.round((containerWidth - gap * (grid - 1)) * (span / grid));
      const html = await panelHtml(dashboard, dashboardId, panel, paramValues, spanWidth);
      cells.push(`<div class="cell" style="grid-column:span ${span}">${html}</div>`);
    }
    rowsHtml.push(
      `<div class="grid-row" style="grid-template-columns:repeat(${grid},1fr)">${cells.join("")}</div>`,
    );
  }

  const doc = `<!doctype html>
<html lang="en"><head><meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>${esc(dashboard.name)} — snapshot</title>
<style>${SNAPSHOT_CSS}</style></head>
<body><div class="wrap">
<div class="top"><h1>${esc(dashboard.name)}</h1><span class="stamp">Snapshot ${esc(
    now.toISOString().replace("T", " ").slice(0, 19),
  )} UTC</span></div>
${paramSummary(dashboard.parameters ?? [], paramValues)}
${rowsHtml.join("\n")}
</div></body></html>`;

  const blob = new Blob([doc], { type: "text/html" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `${slug(dashboard.name)}-snapshot.html`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function slug(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "dashboard";
}

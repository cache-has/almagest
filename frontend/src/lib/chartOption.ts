// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Pure builder for a panel's ECharts option from query rows. Extracted from
// ChartPanel so the live (interactive) renderer and the static snapshot exporter
// produce byte-identical charts. Returns the option plus `seriesRows`, the
// (seriesIndex, dataIndex) → source-row map the interactive panel uses to wire
// click-to-filter; the snapshot ignores it.

import type { ChartPanel } from "./types";
import { applyFormat, toNumber, plain, DEFAULT_PALETTE } from "./format";
import type { EChartsOption } from "./echarts";

export type SeriesRows = (Record<string, unknown> | undefined)[][];

export interface BuiltChart {
  option: EChartsOption;
  seriesRows: SeriesRows;
  /** Result column a click on this chart maps to (x for cartesian, category for pie). */
  clickColumn: string;
}

function isCartesian(panel: ChartPanel): boolean {
  return (
    panel.chart_type === "line" ||
    panel.chart_type === "bar" ||
    panel.chart_type === "area" ||
    panel.chart_type === "scatter"
  );
}

function sortRows(panel: ChartPanel, rows: Record<string, unknown>[]): Record<string, unknown>[] {
  if (!panel.sort || !isCartesian(panel)) return rows;
  const key = panel.sort.endsWith("_x") ? panel.x : panel.y;
  if (!key) return rows;
  const dir = panel.sort.startsWith("asc") ? 1 : -1;
  return [...rows].sort((a, b) => {
    const na = toNumber(a[key]);
    const nb = toNumber(b[key]);
    const c = na !== null && nb !== null ? na - nb : plain(a[key]).localeCompare(plain(b[key]));
    return dir * c;
  });
}

export function buildChartOption(
  panel: ChartPanel,
  resultRows: Record<string, unknown>[],
  palette: string[] = DEFAULT_PALETTE,
): BuiltChart {
  const rows = sortRows(panel, resultRows);
  const clickColumn = isCartesian(panel) ? (panel.x ?? "") : (panel.category ?? "");

  if (!isCartesian(panel)) return { ...buildPie(panel, rows, palette), clickColumn };
  if (panel.chart_type === "scatter") return { ...buildScatter(panel, rows, palette), clickColumn };
  return { ...buildCartesian(panel, rows, palette), clickColumn };
}

function buildPie(
  panel: ChartPanel,
  rows: Record<string, unknown>[],
  palette: string[],
): Omit<BuiltChart, "clickColumn"> {
  const cat = panel.category ?? "";
  const val = panel.value ?? "";
  const data = rows.map((r) => ({
    name: applyFormat(panel.x_format, r[cat]),
    value: toNumber(r[val]) ?? 0,
  }));
  return {
    seriesRows: [rows],
    option: {
      color: palette,
      tooltip: { trigger: "item" },
      legend: panel.show_legend === false ? undefined : { type: "scroll", bottom: 0 },
      series: [
        {
          type: "pie",
          radius: panel.chart_type === "donut" ? ["42%", "70%"] : "70%",
          data,
          label: { formatter: panel.show_percent ? "{b}: {d}%" : "{b}" },
        },
      ],
    },
  };
}

function buildScatter(
  panel: ChartPanel,
  rows: Record<string, unknown>[],
  palette: string[],
): Omit<BuiltChart, "clickColumn"> {
  const xk = panel.x ?? "";
  const yk = panel.y ?? "";
  return {
    seriesRows: [rows],
    option: {
      color: palette,
      tooltip: { trigger: "item" },
      xAxis: { type: "value", axisLabel: { formatter: (v: number) => applyFormat(panel.x_format, v) } },
      yAxis: { type: "value", axisLabel: { formatter: (v: number) => applyFormat(panel.y_format, v) } },
      series: [{ type: "scatter", data: rows.map((r) => [toNumber(r[xk]) ?? 0, toNumber(r[yk]) ?? 0]) }],
    },
  };
}

function buildCartesian(
  panel: ChartPanel,
  rows: Record<string, unknown>[],
  palette: string[],
): Omit<BuiltChart, "clickColumn"> {
  const xk = panel.x ?? "";
  const yk = panel.y ?? "";
  const horizontal = panel.chart_type === "bar" && panel.orientation === "horizontal";
  const baseType = panel.chart_type === "area" ? "line" : panel.chart_type;

  const xs: unknown[] = [];
  for (const r of rows) if (!xs.some((x) => x === r[xk])) xs.push(r[xk]);
  const categories = xs.map((x) => applyFormat(panel.x_format, x));

  type Series = { name: string; data: (number | null)[]; rows: (Record<string, unknown> | undefined)[] };
  const series: Series[] = [];

  if (panel.series) {
    const groups: string[] = [];
    for (const r of rows) {
      const g = plain(r[panel.series]);
      if (!groups.includes(g)) groups.push(g);
    }
    for (const g of groups) {
      const data: (number | null)[] = new Array(xs.length).fill(null);
      const srows: (Record<string, unknown> | undefined)[] = new Array(xs.length).fill(undefined);
      for (const r of rows) {
        if (plain(r[panel.series]) !== g) continue;
        const idx = xs.findIndex((x) => x === r[xk]);
        if (idx >= 0) {
          data[idx] = toNumber(r[yk]);
          srows[idx] = r;
        }
      }
      series.push({ name: g, data, rows: srows });
    }
  } else {
    series.push({ name: yk, data: rows.map((r) => toNumber(r[yk])), rows: [...rows] });
  }

  const seriesRows = series.map((s) => s.rows);

  const catAxis = { type: "category" as const, data: categories };
  const valAxis = {
    type: "value" as const,
    axisLabel: { formatter: (v: number) => applyFormat(panel.y_format, v) },
  };

  return {
    seriesRows,
    option: {
      color: palette,
      tooltip: { trigger: "axis" },
      legend: panel.series && panel.show_legend !== false ? { type: "scroll", bottom: 0 } : undefined,
      grid: { left: 48, right: 16, top: 16, bottom: panel.series ? 40 : 28, containLabel: true },
      xAxis: horizontal ? valAxis : catAxis,
      yAxis: horizontal ? catAxis : valAxis,
      series: series.map((s) => ({
        name: s.name,
        type: baseType,
        data: s.data,
        stack: panel.stacked ? "total" : undefined,
        areaStyle: panel.chart_type === "area" ? {} : undefined,
        smooth: false,
        showSymbol: panel.chart_type !== "area",
      })) as EChartsOption["series"],
    },
  };
}

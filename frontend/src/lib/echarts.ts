// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Tree-shaken ECharts entry point. Importing `echarts/core` and registering only
// the chart types and components Almagest actually renders keeps the bundle far
// smaller than the `import * as echarts from "echarts"` barrel (which pulls in
// every chart, every coordinate system, the SVG renderer, etc.). Both the live
// ChartPanel and the static snapshot exporter import the engine from here so the
// registered feature set is identical.

import * as echarts from "echarts/core";
import { LineChart, BarChart, PieChart, ScatterChart } from "echarts/charts";
import { TooltipComponent, LegendComponent, GridComponent } from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import type { EChartsOption } from "echarts";

echarts.use([
  LineChart,
  BarChart,
  PieChart,
  ScatterChart,
  TooltipComponent,
  LegendComponent,
  GridComponent,
  CanvasRenderer,
]);

export { echarts };
export type { EChartsOption };
export type EChartsInstance = ReturnType<typeof echarts.init>;

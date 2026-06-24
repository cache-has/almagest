<script lang="ts">
  import type { ChartPanel } from "../../lib/types";
  import type { ArrowResult } from "../../lib/arrow";
  import { DEFAULT_PALETTE } from "../../lib/format";
  import { echarts, type EChartsInstance } from "../../lib/echarts";
  import { buildChartOption, type SeriesRows } from "../../lib/chartOption";

  let {
    panel,
    result,
    palette = DEFAULT_PALETTE,
    onPointClick,
  }: {
    panel: ChartPanel;
    result: ArrowResult | null;
    palette?: string[];
    onPointClick?: (row: Record<string, unknown>, column: string) => void;
  } = $props();

  let el: HTMLDivElement;
  let chart: EChartsInstance | null = null;
  // Parallel structure mapping (seriesIndex, dataIndex) → source row, for clicks.
  let seriesRows: SeriesRows = [];
  let clickColumn = "";

  $effect(() => {
    chart = echarts.init(el);
    const ro = new ResizeObserver(() => chart?.resize());
    ro.observe(el);
    chart.on("click", (p) => {
      const row = seriesRows[p.seriesIndex ?? 0]?.[p.dataIndex ?? 0];
      if (row && onPointClick) onPointClick(row, clickColumn);
    });
    return () => {
      ro.disconnect();
      chart?.dispose();
      chart = null;
    };
  });

  // Re-render whenever the result or panel config changes.
  $effect(() => {
    const built = buildChartOption(panel, result?.rows ?? [], palette);
    seriesRows = built.seriesRows;
    clickColumn = built.clickColumn;
    chart?.setOption(built.option, true);
  });
</script>

<div class="chart" bind:this={el}></div>

<style>
  .chart {
    width: 100%;
    height: 100%;
    min-height: 180px;
  }
</style>

<script lang="ts">
  import type { Dashboard } from "../lib/types";
  import { DEFAULT_PALETTE } from "../lib/format";
  import PanelCard from "./PanelCard.svelte";

  let {
    dashboard,
    dashboardId,
    paramValues,
    selectedPanelId = null,
    refreshKey = 0,
    onSetParam,
    onSelectPanel,
  }: {
    dashboard: Dashboard;
    dashboardId: string;
    paramValues: Record<string, unknown>;
    selectedPanelId?: string | null;
    refreshKey?: number;
    onSetParam?: (name: string, value: unknown) => void;
    onSelectPanel?: (panelId: string) => void;
  } = $props();

  const grid = $derived(dashboard.layout.grid ?? 12);
  const palette = $derived(dashboard.theme?.palette ?? DEFAULT_PALETTE);
</script>

<div class="grid-rows" style:background={dashboard.theme?.background}>
  {#each dashboard.layout.rows as row, ri (ri)}
    <div class="grid-row" style:--cols={grid}>
      {#each row.panels as panel (panel.id)}
        <div class="cell" style:--span={Math.min(panel.span, grid)}>
          <PanelCard
            {dashboardId}
            {panel}
            {paramValues}
            {palette}
            {refreshKey}
            selected={selectedPanelId === panel.id}
            {onSetParam}
            onSelect={onSelectPanel ? () => onSelectPanel(panel.id) : undefined}
          />
        </div>
      {/each}
    </div>
  {/each}
</div>

<style>
  .grid-rows {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .grid-row {
    display: grid;
    grid-template-columns: repeat(var(--cols, 12), 1fr);
    gap: 1rem;
    align-items: stretch;
  }
  .cell {
    grid-column: span var(--span, 12);
    min-width: 0;
  }
  /* Tablet: halve the effective width floor so panels aren't hairline-thin. */
  @media (max-width: 900px) {
    .grid-row {
      grid-template-columns: repeat(2, 1fr);
    }
    .cell {
      grid-column: span min(var(--span, 1), 2);
    }
  }
  /* Phone: single column, every panel full width. */
  @media (max-width: 600px) {
    .grid-row {
      grid-template-columns: 1fr;
    }
    .cell {
      grid-column: auto;
    }
  }
  /* Print: never split a row across pages. */
  @media print {
    .grid-row {
      break-inside: avoid;
    }
  }
</style>

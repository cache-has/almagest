<script lang="ts">
  import type { Panel, Action } from "../lib/types";
  import { panelNeedsQuery } from "../lib/types";
  import type { ArrowResult } from "../lib/arrow";
  import { api, ApiError } from "../lib/api";
  import { interpolateActionValue } from "../lib/params";
  import { navigate } from "../lib/router";
  import { DEFAULT_PALETTE } from "../lib/format";

  import MetricPanel from "./panels/MetricPanel.svelte";
  import ChartPanel from "./panels/ChartPanel.svelte";
  import TablePanel from "./panels/TablePanel.svelte";
  import TextPanel from "./panels/TextPanel.svelte";
  import ImagePanel from "./panels/ImagePanel.svelte";
  import DividerPanel from "./panels/DividerPanel.svelte";

  let {
    dashboardId,
    panel,
    paramValues,
    palette = DEFAULT_PALETTE,
    selected = false,
    refreshKey = 0,
    onSetParam,
    onSelect,
  }: {
    dashboardId: string;
    panel: Panel;
    paramValues: Record<string, unknown>;
    palette?: string[];
    selected?: boolean;
    refreshKey?: number;
    onSetParam?: (name: string, value: unknown) => void;
    onSelect?: () => void;
  } = $props();

  let loading = $state(false);
  let error = $state<string | null>(null);
  let result = $state<ArrowResult | null>(null);
  let runId = 0;

  const paramsKey = $derived(JSON.stringify(paramValues));
  const queryKey = $derived(JSON.stringify(panel.query));

  const visible = $derived.by(() => {
    if (!panel.visible) return true;
    const { param, value } = panel.visible.equals;
    return String(paramValues[param]) === String(value);
  });

  $effect(() => {
    // Track dependencies explicitly so the query re-runs on the right changes.
    void paramsKey;
    void queryKey;
    void refreshKey;
    const pid = panel.id;
    const did = dashboardId;

    if (!panelNeedsQuery(panel.kind) || !panel.query || !visible) {
      result = null;
      error = null;
      return;
    }

    const id = ++runId;
    loading = true;
    error = null;
    api
      .executePanel(did, pid, paramValues)
      .then((r) => {
        if (id === runId) {
          result = r;
          loading = false;
        }
      })
      .catch((e: unknown) => {
        if (id === runId) {
          error = e instanceof ApiError ? e.message : String(e);
          loading = false;
        }
      });
  });

  function runActions(row: Record<string, unknown>, column: string) {
    for (const action of panel.on_click ?? []) {
      applyAction(action, row, column);
    }
  }

  function applyAction(action: Action, row: Record<string, unknown>, column: string) {
    if (action.kind === "set_parameter") {
      onSetParam?.(action.parameter, interpolateActionValue(action.value, row, column));
    } else if (action.kind === "navigate_to") {
      // Drilldown: forward the current shareable param state. The target
      // dashboard's decl-aware decoder keeps the parameters it declares and
      // ignores the rest, so shared filters propagate across the drilldown.
      const hash = location.hash;
      const q = hash.includes("?") ? hash.slice(hash.indexOf("?")) : "";
      navigate(`/view/${encodeURIComponent(action.dashboard)}${q}`);
    } else if (action.kind === "open_url") {
      if (/^(https?:|mailto:)/i.test(action.url)) window.open(action.url, "_blank", "noopener");
    }
  }

  const clickable = $derived((panel.on_click?.length ?? 0) > 0);
</script>

{#if visible}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    class="panel"
    class:selected
    class:selectable={!!onSelect}
    onclick={() => onSelect?.()}
    role={onSelect ? "button" : undefined}
    tabindex={onSelect ? 0 : undefined}
    onkeydown={(e) => onSelect && (e.key === "Enter" || e.key === " ") && onSelect()}
  >
    {#if panel.title || panel.description}
      <header>
        {#if panel.title}<h3>{panel.title}</h3>{/if}
        {#if panel.description}<p class="desc">{panel.description}</p>{/if}
      </header>
    {/if}

    <div class="body">
      {#if loading}
        {#if panel.kind === "metric"}
          <div class="metric-skel"><div class="skel sk-num"></div><div class="skel sk-sub"></div></div>
        {:else if panel.kind === "table"}
          <div class="table-skel">
            {#each Array(5) as _, i (i)}<div class="skel sk-row"></div>{/each}
          </div>
        {:else}
          <div class="chart-skel"><div class="skel sk-block"></div></div>
        {/if}
      {:else if error}
        <div class="state error">{error}</div>
      {:else if panel.kind === "metric"}
        <MetricPanel {panel} {result} />
      {:else if panel.kind === "chart"}
        <ChartPanel {panel} {result} {palette} onPointClick={clickable ? runActions : undefined} />
      {:else if panel.kind === "table"}
        <TablePanel {panel} {result} onCellClick={clickable ? runActions : undefined} />
      {:else if panel.kind === "text"}
        <TextPanel {panel} />
      {:else if panel.kind === "image"}
        <ImagePanel {panel} />
      {:else if panel.kind === "divider"}
        <DividerPanel {panel} />
      {/if}
    </div>
  </div>
{/if}

<style>
  .panel {
    background: var(--surface, #fff);
    border: 1px solid var(--border, #e9ecef);
    border-radius: 10px;
    padding: 0.85rem 1rem;
    display: flex;
    flex-direction: column;
    min-height: 90px;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.03);
    overflow: hidden;
  }
  .panel.selectable {
    cursor: pointer;
  }
  .panel.selected {
    border-color: var(--accent, #1c7ed6);
    box-shadow: 0 0 0 2px rgba(28, 126, 214, 0.25);
  }
  header {
    margin-bottom: 0.5rem;
  }
  h3 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 650;
  }
  .desc {
    margin: 0.15rem 0 0;
    font-size: 0.8rem;
    color: var(--muted, #868e96);
  }
  .body {
    flex: 1;
    min-height: 0;
    position: relative;
  }
  .state {
    color: var(--muted, #868e96);
    font-size: 0.85rem;
    padding: 0.5rem 0;
  }
  .state.error {
    color: var(--bad, #e03131);
    white-space: pre-wrap;
  }

  /* Loading skeletons — sized per panel kind to minimize layout shift. */
  .skel {
    background: linear-gradient(
      90deg,
      var(--hover, #f1f3f5) 25%,
      var(--border, #e9ecef) 37%,
      var(--hover, #f1f3f5) 63%
    );
    background-size: 400% 100%;
    border-radius: 6px;
    animation: shimmer 1.4s ease infinite;
  }
  @keyframes shimmer {
    0% {
      background-position: 100% 0;
    }
    100% {
      background-position: 0 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .skel {
      animation: none;
    }
  }
  .metric-skel {
    display: flex;
    flex-direction: column;
    justify-content: center;
    height: 100%;
    gap: 0.6rem;
  }
  .sk-num {
    height: 2rem;
    width: 60%;
  }
  .sk-sub {
    height: 0.85rem;
    width: 35%;
  }
  .table-skel {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding-top: 0.3rem;
  }
  .sk-row {
    height: 0.9rem;
    width: 100%;
  }
  .chart-skel {
    height: 100%;
    min-height: 160px;
  }
  .sk-block {
    height: 100%;
    min-height: 160px;
    width: 100%;
  }
</style>

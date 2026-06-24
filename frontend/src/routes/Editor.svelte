<script lang="ts">
  import { api, ApiError } from "../lib/api";
  import type { Dashboard, Panel, DatabaseSchema } from "../lib/types";
  import { initialParamValues } from "../lib/params";
  import ParameterBar from "../components/ParameterBar.svelte";
  import DashboardGrid from "../components/DashboardGrid.svelte";
  import Inspector from "../editor/Inspector.svelte";
  import DataManager from "../editor/DataManager.svelte";
  import AssetManager from "../editor/AssetManager.svelte";

  let { id }: { id: string } = $props();

  let dashboard = $state<Dashboard | null>(null);
  let schema = $state<DatabaseSchema | null>(null);
  let paramValues = $state<Record<string, unknown>>({});
  let selectedPanelId = $state<string | null>(null);
  let showData = $state(false);
  let showAssets = $state(false);
  let dataVersion = $state(0);
  let dirty = $state(false);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let showJson = $state(false);
  let jsonDraft = $state("");
  let jsonError = $state<string | null>(null);
  let counter = 0;

  $effect(() => {
    const dashId = id;
    loading = true;
    Promise.all([api.getDashboard(dashId), api.schema().catch(() => null)])
      .then(([d, s]) => {
        dashboard = d;
        schema = s;
        paramValues = initialParamValues(d.parameters ?? []);
        loading = false;
      })
      .catch((e: unknown) => {
        error = e instanceof ApiError ? e.message : String(e);
        loading = false;
      });
  });

  const selectedPanel = $derived.by<Panel | null>(() => {
    if (!dashboard || !selectedPanelId) return null;
    for (const row of dashboard.layout.rows) {
      const found = row.panels.find((p) => p.id === selectedPanelId);
      if (found) return found;
    }
    return null;
  });

  function markDirty() {
    dirty = true;
  }

  function setParam(name: string, value: unknown) {
    paramValues = { ...paramValues, [name]: value };
  }

  async function onDataChanged() {
    // A dataset was added/renamed/removed: refresh the schema reference and
    // re-key the grid so panels re-execute against the rebuilt query context.
    schema = await api.schema().catch(() => schema);
    dataVersion += 1;
  }

  async function save() {
    if (!dashboard) return;
    saving = true;
    error = null;
    try {
      await api.updateDashboard(id, dashboard);
      dirty = false;
    } catch (e) {
      error = e instanceof ApiError ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  function newPanel(kind: Panel["kind"]): Panel {
    const pid = `${kind}-${++counter}-${id.slice(0, 4)}`;
    switch (kind) {
      case "metric":
        return { id: pid, span: 3, kind, query: { sql: "SELECT 0 AS value" } };
      case "chart":
        return { id: pid, span: 6, kind, chart_type: "bar", x: "", y: "", query: { sql: "" } };
      case "table":
        return { id: pid, span: 6, kind, sortable: true, query: { sql: "" } };
      case "text":
        return { id: pid, span: 12, kind, content: "Text" };
      case "image":
        return { id: pid, span: 4, kind, asset_path: "" };
      case "divider":
        return { id: pid, span: 12, kind };
    }
  }

  function addPanel(kind: Panel["kind"]) {
    if (!dashboard) return;
    if (dashboard.layout.rows.length === 0) dashboard.layout.rows.push({ panels: [] });
    const panel = newPanel(kind);
    dashboard.layout.rows[dashboard.layout.rows.length - 1].panels.push(panel);
    selectedPanelId = panel.id;
    markDirty();
  }

  function addRow() {
    if (!dashboard) return;
    dashboard.layout.rows.push({ panels: [] });
    markDirty();
  }

  function locate(panelId: string): [number, number] | null {
    if (!dashboard) return null;
    for (let ri = 0; ri < dashboard.layout.rows.length; ri++) {
      const pi = dashboard.layout.rows[ri].panels.findIndex((p) => p.id === panelId);
      if (pi >= 0) return [ri, pi];
    }
    return null;
  }

  function removeSelected() {
    if (!dashboard || !selectedPanelId) return;
    const loc = locate(selectedPanelId);
    if (!loc) return;
    dashboard.layout.rows[loc[0]].panels.splice(loc[1], 1);
    if (dashboard.layout.rows[loc[0]].panels.length === 0 && dashboard.layout.rows.length > 1) {
      dashboard.layout.rows.splice(loc[0], 1);
    }
    selectedPanelId = null;
    markDirty();
  }

  function move(delta: number) {
    if (!dashboard || !selectedPanelId) return;
    const loc = locate(selectedPanelId);
    if (!loc) return;
    const [ri, pi] = loc;
    const panels = dashboard.layout.rows[ri].panels;
    const ni = pi + delta;
    if (ni < 0 || ni >= panels.length) return;
    [panels[pi], panels[ni]] = [panels[ni], panels[pi]];
    markDirty();
  }

  function openJson() {
    jsonDraft = JSON.stringify(dashboard, null, 2);
    jsonError = null;
    showJson = true;
  }

  function applyJson() {
    try {
      dashboard = JSON.parse(jsonDraft) as Dashboard;
      paramValues = initialParamValues(dashboard.parameters ?? []);
      jsonError = null;
      showJson = false;
      markDirty();
    } catch (e) {
      jsonError = e instanceof Error ? e.message : String(e);
    }
  }

  function exportJson() {
    if (!dashboard) return;
    const blob = new Blob([JSON.stringify(dashboard, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${dashboard.name.replace(/[^a-z0-9]+/gi, "-").toLowerCase() || "dashboard"}.json`;
    a.click();
    URL.revokeObjectURL(url);
  }

  const PALETTE: Panel["kind"][] = ["metric", "chart", "table", "text", "image", "divider"];
</script>

<div class="editor">
  <div class="toolbar">
    <a class="back" href="#/">←</a>
    {#if dashboard}<span class="title">{dashboard.name}</span>{/if}
    {#if dirty}<span class="dot" title="Unsaved changes"></span>{/if}
    <div class="spacer"></div>
    {#if error}<span class="err">{error}</span>{/if}
    <a class="link" href={`#/view/${id}`}>View</a>
    <button class="ghost" onclick={() => (showData = true)}>Data</button>
    <button class="ghost" onclick={() => (showAssets = true)}>Assets</button>
    <button class="ghost" onclick={openJson}>JSON</button>
    <button class="ghost" onclick={exportJson}>Export</button>
    <button class="primary" disabled={!dirty || saving} onclick={save}>{saving ? "Saving…" : "Save"}</button>
  </div>

  {#if loading}
    <p class="muted pad">Loading…</p>
  {:else if dashboard}
    <div class="workspace">
      <div class="canvas">
        <div class="add-bar">
          <span class="muted">Add panel:</span>
          {#each PALETTE as kind (kind)}
            <button class="chip" onclick={() => addPanel(kind)}>{kind}</button>
          {/each}
          <button class="chip" onclick={addRow}>+ row</button>
          {#if selectedPanelId}
            <span class="sep"></span>
            <button class="chip" onclick={() => move(-1)}>◀ move</button>
            <button class="chip" onclick={() => move(1)}>move ▶</button>
            <button class="chip danger" onclick={removeSelected}>Delete</button>
          {/if}
        </div>

        <ParameterBar
          parameters={dashboard.parameters ?? []}
          values={paramValues}
          dashboardId={id}
          onSetParam={setParam}
        />
        {#key dataVersion}
          <DashboardGrid
            {dashboard}
            dashboardId={id}
            {paramValues}
            {selectedPanelId}
            onSetParam={setParam}
            onSelectPanel={(pid) => (selectedPanelId = selectedPanelId === pid ? null : pid)}
          />
        {/key}
      </div>

      <Inspector {dashboard} panel={selectedPanel} {schema} onEdit={markDirty} />
    </div>
  {/if}

  {#if showJson}
    <div
      class="modal-backdrop"
      onclick={() => (showJson = false)}
      onkeydown={(e) => e.key === "Escape" && (showJson = false)}
      role="presentation"
    >
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="modal"
        onclick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        tabindex="-1"
      >
        <h3>Dashboard JSON</h3>
        <textarea bind:value={jsonDraft} spellcheck="false"></textarea>
        {#if jsonError}<p class="err">{jsonError}</p>{/if}
        <div class="modal-actions">
          <button class="ghost" onclick={() => (showJson = false)}>Cancel</button>
          <button class="primary" onclick={applyJson}>Apply</button>
        </div>
      </div>
    </div>
  {/if}

  {#if showData}
    <div
      class="modal-backdrop"
      onclick={() => (showData = false)}
      onkeydown={(e) => e.key === "Escape" && (showData = false)}
      role="presentation"
    >
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="modal wide" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
        <DataManager onClose={() => (showData = false)} onChanged={onDataChanged} />
      </div>
    </div>
  {/if}

  {#if showAssets}
    <div
      class="modal-backdrop"
      onclick={() => (showAssets = false)}
      onkeydown={(e) => e.key === "Escape" && (showAssets = false)}
      role="presentation"
    >
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="modal wide" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
        <AssetManager onClose={() => (showAssets = false)} />
      </div>
    </div>
  {/if}
</div>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.6rem 1rem;
    border-bottom: 1px solid var(--border, #e9ecef);
    background: var(--surface, #fff);
  }
  .back {
    text-decoration: none;
    font-size: 1.1rem;
    color: var(--muted, #495057);
  }
  .title {
    font-weight: 650;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent, #1c7ed6);
  }
  .spacer {
    flex: 1;
  }
  .workspace {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .canvas {
    flex: 1;
    overflow-y: auto;
    padding: 1rem 1.25rem;
  }
  .add-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-bottom: 1rem;
  }
  .sep {
    width: 1px;
    height: 18px;
    background: var(--border, #dee2e6);
    margin: 0 0.3rem;
  }
  .chip {
    border: 1px solid var(--border, #ced4da);
    background: var(--surface, #fff);
    border-radius: 6px;
    padding: 0.25rem 0.55rem;
    cursor: pointer;
    font-size: 0.8rem;
  }
  .chip.danger {
    color: var(--bad, #e03131);
    border-color: var(--bad, #e03131);
  }
  .primary {
    background: var(--accent, #1c7ed6);
    color: #fff;
    border: none;
    padding: 0.4rem 0.85rem;
    border-radius: 6px;
    cursor: pointer;
    font-weight: 600;
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .ghost,
  .link {
    border: 1px solid var(--border, #ced4da);
    background: var(--surface, #fff);
    padding: 0.35rem 0.7rem;
    border-radius: 6px;
    cursor: pointer;
    font-size: 0.85rem;
    color: inherit;
    text-decoration: none;
  }
  .err {
    color: var(--bad, #e03131);
    font-size: 0.85rem;
  }
  .muted {
    color: var(--muted, #868e96);
  }
  .pad {
    padding: 1rem 1.25rem;
  }
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: grid;
    place-items: center;
    z-index: 50;
  }
  .modal {
    background: var(--surface, #fff);
    border-radius: 10px;
    padding: 1.25rem;
    width: min(720px, 92vw);
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .modal.wide {
    width: min(900px, 94vw);
    max-height: 85vh;
    overflow-y: auto;
  }
  .modal h3 {
    margin: 0;
  }
  .modal textarea {
    width: 100%;
    height: 50vh;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.8rem;
    border: 1px solid var(--border, #ced4da);
    border-radius: 6px;
    padding: 0.6rem;
    resize: vertical;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }
</style>

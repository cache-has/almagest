<script lang="ts">
  import type { Dashboard, Panel, Parameter, ParamKind, ChartType, DatabaseSchema, Format } from "../lib/types";
  import { panelNeedsQuery } from "../lib/types";

  let {
    dashboard,
    panel,
    schema,
    onEdit,
  }: {
    dashboard: Dashboard;
    panel: Panel | null;
    schema: DatabaseSchema | null;
    onEdit: () => void;
  } = $props();

  // Dynamic field access for kind-specific config (the union is awkward to index).
  const p = $derived(panel as unknown as Record<string, unknown> | null);

  const CHART_TYPES: ChartType[] = ["line", "bar", "area", "donut", "pie", "scatter"];
  const PARAM_KINDS: ParamKind[] = ["text", "number", "boolean", "date", "daterange", "select", "multiselect"];
  const FORMAT_KINDS = ["(none)", "number", "currency", "percent", "compact", "datetime", "duration"];

  function edit<T>(fn: () => T): T {
    const r = fn();
    onEdit();
    return r;
  }

  function getSql(): string {
    const q = panel?.query;
    if (q && "sql" in q) return q.sql;
    return "";
  }
  function setSql(sql: string) {
    if (!panel) return;
    edit(() => (panel.query = { sql }));
  }
  const isReference = $derived(!!panel?.query && "query_id" in panel.query);

  function formatKind(): string {
    const f = (p?.format as Format | undefined) ?? undefined;
    return f?.kind ?? "(none)";
  }
  function setFormatKind(kind: string) {
    if (!panel) return;
    const obj = panel as unknown as Record<string, unknown>;
    edit(() => {
      if (kind === "(none)") delete obj.format;
      else if (kind === "currency") obj.format = { kind: "currency" };
      else if (kind === "datetime") obj.format = { kind: "datetime" };
      else if (kind === "duration") obj.format = { kind: "duration" };
      else obj.format = { kind };
    });
  }

  function addParameter() {
    const params = (dashboard.parameters ??= []);
    const id = `param_${params.length + 1}`;
    edit(() => params.push({ id, kind: "text" } as Parameter));
  }
  function removeParameter(i: number) {
    edit(() => dashboard.parameters?.splice(i, 1));
  }
</script>

<div class="inspector">
  {#if panel && p}
    <h2>Panel</h2>

    <section>
      <label>Title<input value={panel.title ?? ""} oninput={(e) => edit(() => (panel.title = e.currentTarget.value))} /></label>
      <label>Description<input value={panel.description ?? ""} oninput={(e) => edit(() => (panel.description = e.currentTarget.value))} /></label>
      <label>Kind<input value={panel.kind} disabled /></label>
      <label>Span (1–12)
        <input type="number" min="1" max="12" value={panel.span}
          oninput={(e) => edit(() => (panel.span = Math.max(1, Math.min(12, e.currentTarget.valueAsNumber || 1))))} />
      </label>
    </section>

    <!-- Display -->
    <h3>Display</h3>
    <section>
      {#if panel.kind === "metric"}
        <label>Value format
          <select value={formatKind()} onchange={(e) => setFormatKind(e.currentTarget.value)}>
            {#each FORMAT_KINDS as k (k)}<option value={k}>{k}</option>{/each}
          </select>
        </label>
      {:else if panel.kind === "chart"}
        <label>Chart type
          <select value={p.chart_type as string} onchange={(e) => edit(() => (p.chart_type = e.currentTarget.value))}>
            {#each CHART_TYPES as t (t)}<option value={t}>{t}</option>{/each}
          </select>
        </label>
        {#if ["line", "bar", "area", "scatter"].includes(panel.kind === "chart" ? (p.chart_type as string) : "")}
          <label>X column<input value={(p.x as string) ?? ""} oninput={(e) => edit(() => (p.x = e.currentTarget.value))} /></label>
          <label>Y column<input value={(p.y as string) ?? ""} oninput={(e) => edit(() => (p.y = e.currentTarget.value))} /></label>
          <label>Series (optional)<input value={(p.series as string) ?? ""} oninput={(e) => edit(() => (p.series = e.currentTarget.value || undefined))} /></label>
          <label class="row"><input type="checkbox" checked={!!p.stacked} onchange={(e) => edit(() => (p.stacked = e.currentTarget.checked))} /> Stacked</label>
        {:else}
          <label>Category column<input value={(p.category as string) ?? ""} oninput={(e) => edit(() => (p.category = e.currentTarget.value))} /></label>
          <label>Value column<input value={(p.value as string) ?? ""} oninput={(e) => edit(() => (p.value = e.currentTarget.value))} /></label>
          <label class="row"><input type="checkbox" checked={!!p.show_percent} onchange={(e) => edit(() => (p.show_percent = e.currentTarget.checked))} /> Show %</label>
        {/if}
      {:else if panel.kind === "table"}
        <label class="row"><input type="checkbox" checked={!!p.sortable} onchange={(e) => edit(() => (p.sortable = e.currentTarget.checked))} /> Sortable</label>
        <label>Page size<input type="number" min="0" value={(p.page_size as number) ?? 0} oninput={(e) => edit(() => (p.page_size = e.currentTarget.valueAsNumber || undefined))} /></label>
      {:else if panel.kind === "text"}
        <label>Markdown<textarea rows="6" value={p.content as string} oninput={(e) => edit(() => (p.content = e.currentTarget.value))}></textarea></label>
      {:else if panel.kind === "image"}
        <label>Asset path<input value={(p.asset_path as string) ?? ""} oninput={(e) => edit(() => (p.asset_path = e.currentTarget.value))} /></label>
        <label>Alt text<input value={(p.alt as string) ?? ""} oninput={(e) => edit(() => (p.alt = e.currentTarget.value || undefined))} /></label>
      {:else if panel.kind === "divider"}
        <label>Label<input value={(p.label as string) ?? ""} oninput={(e) => edit(() => (p.label = e.currentTarget.value || undefined))} /></label>
      {/if}
    </section>

    <!-- Query -->
    {#if panelNeedsQuery(panel.kind)}
      <h3>Query</h3>
      <section>
        {#if isReference}
          <p class="note">Uses saved query <code>{(panel.query as { query_id: string }).query_id}</code>.</p>
        {:else}
          <textarea class="sql" rows="6" value={getSql()} oninput={(e) => setSql(e.currentTarget.value)} placeholder="SELECT …"></textarea>
        {/if}
        {#if schema}
          <details class="schema">
            <summary>Tables ({schema.tables.length})</summary>
            {#each schema.tables as t (t.name)}
              <div class="tbl"><strong>{t.name}</strong> <span class="muted">({t.row_count} rows)</span>
                <div class="cols">{t.columns.map((c) => c.name).join(", ")}</div>
              </div>
            {/each}
          </details>
        {/if}
      </section>
    {/if}
  {:else}
    <h2>Dashboard</h2>
    <section>
      <label>Name<input value={dashboard.name} oninput={(e) => edit(() => (dashboard.name = e.currentTarget.value))} /></label>
      <label>Description<input value={dashboard.description ?? ""} oninput={(e) => edit(() => (dashboard.description = e.currentTarget.value || undefined))} /></label>
    </section>

    <h3>Parameters <button class="mini" onclick={addParameter}>+ Add</button></h3>
    <section>
      {#each dashboard.parameters ?? [] as param, i (i)}
        <div class="param-edit">
          <div class="param-head">
            <input class="pid" value={param.id} oninput={(e) => edit(() => (param.id = e.currentTarget.value))} />
            <select value={param.kind} onchange={(e) => edit(() => (param.kind = e.currentTarget.value as ParamKind))}>
              {#each PARAM_KINDS as k (k)}<option value={k}>{k}</option>{/each}
            </select>
            <button class="mini danger" onclick={() => removeParameter(i)}>✕</button>
          </div>
          {#if param.kind === "select" || param.kind === "multiselect"}
            <input class="opts" placeholder="comma,separated,options"
              value={(param.options ?? []).join(",")}
              oninput={(e) => edit(() => (param.options = e.currentTarget.value.split(",").map((s) => s.trim()).filter(Boolean)))} />
          {/if}
        </div>
      {:else}
        <p class="muted">No parameters.</p>
      {/each}
    </section>
  {/if}
</div>

<style>
  .inspector {
    width: 320px;
    flex-shrink: 0;
    border-left: 1px solid var(--border, #e9ecef);
    background: var(--surface, #fff);
    padding: 1rem;
    overflow-y: auto;
    font-size: 0.85rem;
  }
  h2 {
    margin: 0 0 0.75rem;
    font-size: 1rem;
  }
  h3 {
    margin: 1rem 0 0.5rem;
    font-size: 0.85rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted, #868e96);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-weight: 600;
    color: var(--muted, #495057);
  }
  label.row {
    flex-direction: row;
    align-items: center;
    gap: 0.4rem;
  }
  input,
  select,
  textarea {
    font: inherit;
    padding: 0.3rem 0.45rem;
    border: 1px solid var(--border, #ced4da);
    border-radius: 6px;
    background: var(--surface, #fff);
    font-weight: 400;
  }
  textarea.sql {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.8rem;
  }
  .note {
    color: var(--muted, #868e96);
  }
  .schema {
    font-size: 0.78rem;
  }
  .tbl {
    padding: 0.25rem 0;
  }
  .cols {
    color: var(--muted, #868e96);
    font-family: ui-monospace, monospace;
    font-size: 0.72rem;
  }
  .param-edit {
    border: 1px solid var(--border, #e9ecef);
    border-radius: 6px;
    padding: 0.4rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .param-head {
    display: flex;
    gap: 0.35rem;
  }
  .pid {
    flex: 1;
  }
  .mini {
    border: 1px solid var(--border, #ced4da);
    background: var(--surface, #fff);
    border-radius: 5px;
    cursor: pointer;
    font-size: 0.75rem;
    padding: 0.15rem 0.4rem;
  }
  .mini.danger {
    color: var(--bad, #e03131);
  }
  .muted {
    color: var(--muted, #868e96);
    font-weight: 400;
  }
</style>

<script lang="ts">
  import { api, ApiError } from "../lib/api";
  import type { Dashboard } from "../lib/types";
  import { initialParamValues } from "../lib/params";
  import { encodeUrlState, decodeUrlState, layeredState } from "../lib/urlstate";
  import { exportSnapshot } from "../lib/snapshot";
  import { getSnapshot } from "../lib/snapshotData";
  import ParameterBar from "../components/ParameterBar.svelte";
  import DashboardGrid from "../components/DashboardGrid.svelte";

  let { id, query = "" }: { id: string; query?: string } = $props();

  // A baked snapshot renders read-only: results are frozen, parameters can't be
  // changed (there's no engine to re-query), and the live-only toolbar is hidden.
  const snap = getSnapshot();
  const isSnapshot = !!snap;

  let dashboard = $state<Dashboard | null>(null);
  let paramValues = $state<Record<string, unknown>>({});
  let error = $state<string | null>(null);
  let loading = $state(true);
  let refreshKey = $state(0);
  let copied = $state(false);
  let exporting = $state(false);
  let seeded = false;

  $effect(() => {
    const dashId = id;
    loading = true;
    error = null;
    seeded = false;
    api
      .getDashboard(dashId)
      .then((d) => {
        dashboard = d;
        const decls = d.parameters ?? [];
        // Snapshot: the frozen param values. Live: URL state over declared defaults.
        paramValues = isSnapshot
          ? layeredState(snap!.params, initialParamValues(decls))
          : layeredState(decodeUrlState(query, decls), initialParamValues(decls));
        seeded = true;
        loading = false;
      })
      .catch((e: unknown) => {
        error = e instanceof ApiError ? e.message : String(e);
        loading = false;
      });
  });

  // Reflect parameter changes into the URL (silently — no history spam, no
  // hashchange loop) so the address bar is always a shareable link. Read the
  // reactive values *before* the guard: a short-circuiting `if` would skip the
  // dependency reads and the effect would never re-run on param changes.
  $effect(() => {
    const dash = dashboard;
    const values = paramValues;
    if (!seeded || !dash || isSnapshot) return;
    const qs = encodeUrlState(values, dash.parameters ?? []);
    const url = `#/view/${encodeURIComponent(id)}${qs ? `?${qs}` : ""}`;
    history.replaceState(history.state, "", url);
  });

  function setParam(name: string, value: unknown) {
    paramValues = { ...paramValues, [name]: value };
  }

  function refresh() {
    refreshKey += 1;
  }

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(location.href);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      // Clipboard blocked (insecure context) — select-and-copy is the fallback,
      // but the address bar already reflects the shareable URL.
    }
  }

  async function snapshot() {
    if (!dashboard || exporting) return;
    exporting = true;
    try {
      await exportSnapshot(dashboard, id, paramValues);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      exporting = false;
    }
  }
</script>

<div class="viewer">
  <header class="topbar">
    {#if !isSnapshot}<a class="back" href="#/">← All dashboards</a>{/if}
    {#if dashboard}
      <h1>{dashboard.name}</h1>
      {#if !isSnapshot}
        <div class="actions">
          <button onclick={refresh} title="Re-run all panels">⟳ Refresh</button>
          <button onclick={copyLink} title="Copy a shareable link to this view">
            {copied ? "✓ Copied" : "Share"}
          </button>
          <button onclick={snapshot} disabled={exporting} title="Download a static HTML snapshot">
            {exporting ? "Exporting…" : "Export"}
          </button>
          <a class="edit" href={`#/edit/${id}`}>Edit</a>
        </div>
      {/if}
    {/if}
  </header>

  {#if isSnapshot && snap}
    <div class="snapshot-banner">
      Static snapshot — data frozen at {snap.generatedAt.replace("T", " ").slice(0, 19)} UTC.
      Parameters are read-only.
    </div>
  {/if}

  {#if loading}
    <p class="muted">Loading…</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if dashboard}
    <ParameterBar
      parameters={dashboard.parameters ?? []}
      values={paramValues}
      dashboardId={id}
      disabled={isSnapshot}
      onSetParam={setParam}
    />
    <DashboardGrid {dashboard} dashboardId={id} {paramValues} {refreshKey} onSetParam={setParam} />
  {/if}
</div>

<style>
  .viewer {
    max-width: 1200px;
    margin: 0 auto;
    padding: 1.25rem;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 1rem;
    margin-bottom: 1rem;
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-size: 1.35rem;
    flex: 1;
    min-width: 0;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .actions button {
    border: 1px solid var(--border, #dee2e6);
    background: var(--surface, #fff);
    border-radius: 6px;
    padding: 0.35rem 0.7rem;
    font-size: 0.85rem;
    cursor: pointer;
    color: var(--text, #212529);
  }
  .actions button:hover:not(:disabled) {
    background: var(--hover, #f1f3f5);
  }
  .actions button:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .back,
  .edit {
    color: var(--accent, #1c7ed6);
    text-decoration: none;
    font-size: 0.9rem;
  }
  .muted {
    color: var(--muted, #868e96);
  }
  .error {
    color: var(--bad, #e03131);
  }
  .snapshot-banner {
    background: #fff9db;
    border: 1px solid #ffe066;
    color: #846a00;
    border-radius: 8px;
    padding: 0.5rem 0.85rem;
    font-size: 0.82rem;
    margin-bottom: 1rem;
  }

  /* Print: drop the chrome, keep the dashboard. */
  @media print {
    .topbar,
    .back,
    .edit,
    .actions {
      display: none;
    }
    .viewer {
      padding: 0;
      max-width: none;
    }
  }

  /* Narrow screens: actions wrap under the title, larger tap targets. */
  @media (max-width: 600px) {
    h1 {
      flex-basis: 100%;
    }
    .actions button {
      padding: 0.45rem 0.8rem;
    }
  }
</style>

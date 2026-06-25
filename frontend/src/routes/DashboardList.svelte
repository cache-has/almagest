<script lang="ts">
  import { api, ApiError } from "../lib/api";
  import { navigate } from "../lib/router";
  import { canEdit } from "../lib/authState.svelte";
  import type { AlmagestMeta, DashboardSummary, Dashboard } from "../lib/types";
  import { DASHBOARD_DSL_VERSION } from "../lib/types";

  const editable = $derived(canEdit());

  let meta = $state<AlmagestMeta | null>(null);
  let dashboards = $state<DashboardSummary[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(true);

  async function load() {
    loading = true;
    error = null;
    try {
      [meta, dashboards] = await Promise.all([api.meta(), api.listDashboards()]);
    } catch (e) {
      error = e instanceof ApiError ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  load();

  async function createBlank() {
    const dash: Dashboard = {
      version: DASHBOARD_DSL_VERSION,
      name: "Untitled dashboard",
      layout: {
        rows: [{ panels: [{ id: "text-1", span: 12, kind: "text", content: "# New dashboard" }] }],
      },
    };
    try {
      const { id } = await api.createDashboard(dash);
      navigate(`/edit/${id}`);
    } catch (e) {
      error = e instanceof ApiError ? e.message : String(e);
    }
  }
</script>

<div class="list">
  <header>
    <div>
      <h1>{meta?.title || "Almagest"}</h1>
      {#if meta}<p class="sub">{meta.dashboard_count} dashboard{meta.dashboard_count === 1 ? "" : "s"} · format v{meta.format_version}</p>{/if}
    </div>
    {#if editable}<button class="primary" onclick={createBlank}>New dashboard</button>{/if}
  </header>

  {#if loading}
    <p class="muted">Loading…</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if dashboards.length === 0}
    <p class="muted">No dashboards yet. Create one to get started.</p>
  {:else}
    <ul>
      {#each dashboards as d (d.id)}
        <li>
          <div class="info">
            <span class="name">{d.name}</span>
            {#if d.description}<span class="desc">{d.description}</span>{/if}
          </div>
          <div class="actions">
            <a href={`#/view/${d.id}`}>View</a>
            {#if editable}<a href={`#/edit/${d.id}`}>Edit</a>{/if}
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .list {
    max-width: 880px;
    margin: 0 auto;
    padding: 2rem 1.25rem;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1.5rem;
  }
  h1 {
    margin: 0;
    font-size: 1.5rem;
  }
  .sub {
    margin: 0.2rem 0 0;
    color: var(--muted, #868e96);
    font-size: 0.85rem;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.85rem 1rem;
    background: var(--surface, #fff);
    border: 1px solid var(--border, #e9ecef);
    border-radius: 8px;
  }
  .name {
    font-weight: 600;
  }
  .desc {
    margin-left: 0.6rem;
    color: var(--muted, #868e96);
    font-size: 0.85rem;
  }
  .actions {
    display: flex;
    gap: 1rem;
  }
  .actions a {
    color: var(--accent, #1c7ed6);
    text-decoration: none;
    font-size: 0.9rem;
  }
  .primary {
    background: var(--accent, #1c7ed6);
    color: #fff;
    border: none;
    padding: 0.5rem 0.9rem;
    border-radius: 7px;
    cursor: pointer;
    font-weight: 600;
  }
  .muted {
    color: var(--muted, #868e96);
  }
  .error {
    color: var(--bad, #e03131);
  }
</style>

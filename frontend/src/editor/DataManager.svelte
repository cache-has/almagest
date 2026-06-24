<script lang="ts">
  import { api, ApiError } from "../lib/api";
  import type { DatasetInfo, IngestResult } from "../lib/types";
  import { applyFormat } from "../lib/format";

  let { onClose, onChanged }: { onClose: () => void; onChanged: () => void } = $props();

  let datasets = $state<DatasetInfo[]>([]);
  let error = $state<string | null>(null);
  let busy = $state(false);

  // Pending upload + its dry-run preview.
  let pendingFile = $state<File | null>(null);
  let targetName = $state("");
  let replace = $state(false);
  let preview = $state<IngestResult | null>(null);

  async function load() {
    try {
      datasets = await api.listDatasets();
    } catch (e) {
      error = msg(e);
    }
  }
  load();

  function msg(e: unknown): string {
    return e instanceof ApiError ? e.message : String(e);
  }

  function stem(name: string): string {
    const base = name.split(/[\\/]/).pop() ?? name;
    const dot = base.lastIndexOf(".");
    return dot > 0 ? base.slice(0, dot) : base;
  }

  async function pickFile(file: File) {
    pendingFile = file;
    targetName = stem(file.name);
    replace = false;
    preview = null;
    error = null;
    busy = true;
    try {
      preview = await api.ingestDataset(file, { name: targetName, dryRun: true });
    } catch (e) {
      error = msg(e);
      pendingFile = null;
    } finally {
      busy = false;
    }
  }

  async function commit() {
    if (!pendingFile) return;
    busy = true;
    error = null;
    try {
      await api.ingestDataset(pendingFile, { name: targetName, replace });
      pendingFile = null;
      preview = null;
      await load();
      onChanged();
    } catch (e) {
      error = msg(e);
    } finally {
      busy = false;
    }
  }

  async function rename(name: string) {
    const to = prompt(`Rename "${name}" to:`, name);
    if (!to || to === name) return;
    try {
      await api.renameDataset(name, to);
      await load();
      onChanged();
    } catch (e) {
      error = msg(e);
    }
  }

  async function remove(name: string) {
    if (!confirm(`Delete dataset "${name}"? This cannot be undone.`)) return;
    try {
      await api.deleteDataset(name);
      await load();
      onChanged();
    } catch (e) {
      error = msg(e);
    }
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    const file = e.dataTransfer?.files?.[0];
    if (file) pickFile(file);
  }

  function onInput(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    if (file) pickFile(file);
  }

  function fmtBytes(n: number): string {
    return applyFormat({ kind: "compact" }, n) + "B";
  }
</script>

<div class="dm">
  <header><h3>Data</h3><button class="x" onclick={onClose}>✕</button></header>

  {#if error}<p class="err">{error}</p>{/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="drop" ondragover={(e) => e.preventDefault()} ondrop={onDrop}>
    <p>Drop a CSV, Parquet, JSON, or SQLite file here</p>
    <label class="filebtn">
      Choose file…
      <input type="file" accept=".csv,.parquet,.pq,.json,.ndjson,.jsonl,.sqlite,.sqlite3,.db" onchange={onInput} />
    </label>
  </div>

  {#if busy && !preview}<p class="muted">Analyzing…</p>{/if}

  {#if preview && pendingFile}
    <div class="preview">
      <h4>Preview: {pendingFile.name}</h4>
      {#each preview.datasets as d (d.name)}
        <p class="muted">{d.row_count} rows · {d.columns.length} columns</p>
        <div class="cols">
          {#each d.columns as c (c.name)}<span class="col">{c.name} <em>{c.data_type}</em></span>{/each}
        </div>
        {#if d.warnings.length}<p class="warn">{d.warnings.join("; ")}</p>{/if}
      {/each}
      <div class="commit">
        <label>Table name <input bind:value={targetName} /></label>
        <label class="row"><input type="checkbox" bind:checked={replace} /> Replace if exists</label>
        <button class="primary" disabled={busy || !targetName} onclick={commit}>Import</button>
        <button class="ghost" onclick={() => { pendingFile = null; preview = null; }}>Cancel</button>
      </div>
    </div>
  {/if}

  <h4>Tables ({datasets.length})</h4>
  <table>
    <thead><tr><th>Name</th><th>Rows</th><th>Size</th><th>Columns</th><th></th></tr></thead>
    <tbody>
      {#each datasets as d (d.name)}
        <tr>
          <td class="mono">{d.name}</td>
          <td>{d.row_count.toLocaleString()}</td>
          <td>{fmtBytes(d.byte_size)}</td>
          <td class="muted">{d.columns.map((c) => c.name).join(", ")}</td>
          <td class="actions">
            <button class="mini" onclick={() => rename(d.name)}>Rename</button>
            <button class="mini danger" onclick={() => remove(d.name)}>Delete</button>
          </td>
        </tr>
      {:else}
        <tr><td colspan="5" class="muted empty">No datasets yet.</td></tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .dm { display: flex; flex-direction: column; gap: 0.75rem; }
  header { display: flex; align-items: center; justify-content: space-between; }
  header h3 { margin: 0; }
  h4 { margin: 0.5rem 0 0.25rem; font-size: 0.9rem; }
  .x { border: none; background: none; cursor: pointer; font-size: 1rem; }
  .drop {
    border: 2px dashed var(--border, #ced4da);
    border-radius: 10px;
    padding: 1.25rem;
    text-align: center;
    color: var(--muted, #868e96);
  }
  .filebtn {
    display: inline-block;
    margin-top: 0.5rem;
    cursor: pointer;
    color: var(--accent, #1c7ed6);
    font-weight: 600;
  }
  .filebtn input { display: none; }
  .preview {
    border: 1px solid var(--border, #e9ecef);
    border-radius: 8px;
    padding: 0.75rem;
    background: var(--hover, #f8f9fa);
  }
  .preview h4 { margin-top: 0; }
  .cols { display: flex; flex-wrap: wrap; gap: 0.35rem; margin: 0.4rem 0; }
  .col {
    font-size: 0.75rem;
    background: var(--surface, #fff);
    border: 1px solid var(--border, #e9ecef);
    border-radius: 5px;
    padding: 0.1rem 0.4rem;
  }
  .col em { color: var(--muted, #868e96); font-style: normal; }
  .commit { display: flex; align-items: flex-end; gap: 0.75rem; flex-wrap: wrap; margin-top: 0.5rem; }
  .commit label { display: flex; flex-direction: column; gap: 0.2rem; font-size: 0.8rem; font-weight: 600; }
  .commit label.row { flex-direction: row; align-items: center; gap: 0.3rem; font-weight: 400; }
  .commit label input:not([type]) {
    padding: 0.3rem 0.45rem; border: 1px solid var(--border, #ced4da); border-radius: 6px;
  }
  table { border-collapse: collapse; width: 100%; font-size: 0.82rem; }
  th, td { text-align: left; padding: 0.35rem 0.5rem; border-bottom: 1px solid var(--border, #e9ecef); }
  th { color: var(--muted, #868e96); font-weight: 650; }
  .mono { font-family: ui-monospace, monospace; }
  .actions { display: flex; gap: 0.35rem; justify-content: flex-end; }
  .mini { border: 1px solid var(--border, #ced4da); background: var(--surface, #fff); border-radius: 5px; cursor: pointer; font-size: 0.75rem; padding: 0.15rem 0.45rem; }
  .mini.danger { color: var(--bad, #e03131); }
  .primary { background: var(--accent, #1c7ed6); color: #fff; border: none; padding: 0.4rem 0.85rem; border-radius: 6px; cursor: pointer; font-weight: 600; }
  .primary:disabled { opacity: 0.5; cursor: default; }
  .ghost { border: 1px solid var(--border, #ced4da); background: var(--surface, #fff); padding: 0.4rem 0.7rem; border-radius: 6px; cursor: pointer; }
  .muted { color: var(--muted, #868e96); }
  .empty { text-align: center; padding: 0.75rem; }
  .err { color: var(--bad, #e03131); }
  .warn { color: #b8860b; font-size: 0.8rem; }
</style>

<script lang="ts">
  import { api, ApiError, assetUrl } from "../lib/api";
  import type { AssetEntry } from "../lib/types";

  let { onClose }: { onClose: () => void } = $props();

  let assets = $state<AssetEntry[]>([]);
  let error = $state<string | null>(null);
  let busy = $state(false);

  async function load() {
    try {
      assets = await api.listAssets();
    } catch (e) {
      error = msg(e);
    }
  }
  load();

  function msg(e: unknown): string {
    return e instanceof ApiError ? e.message : String(e);
  }

  async function upload(file: File) {
    const path = prompt("Asset path (e.g. logo.png):", file.name);
    if (!path) return;
    busy = true;
    error = null;
    try {
      await api.uploadAsset(path, file);
      await load();
    } catch (e) {
      error = msg(e);
    } finally {
      busy = false;
    }
  }

  async function remove(path: string) {
    if (!confirm(`Delete asset "${path}"?`)) return;
    try {
      await api.deleteAsset(path);
      await load();
    } catch (e) {
      error = msg(e);
    }
  }

  function onInput(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    if (file) upload(file);
  }

  function isImage(ct: string): boolean {
    return ct.startsWith("image/");
  }
</script>

<div class="am">
  <header><h3>Assets</h3><button class="x" onclick={onClose}>✕</button></header>
  {#if error}<p class="err">{error}</p>{/if}

  <label class="filebtn">
    {busy ? "Uploading…" : "Upload asset…"}
    <input type="file" onchange={onInput} disabled={busy} />
  </label>

  <div class="grid">
    {#each assets as a (a.path)}
      <div class="asset">
        {#if isImage(a.content_type)}
          <img src={assetUrl(a.path)} alt={a.path} />
        {:else}
          <div class="placeholder">{a.content_type}</div>
        {/if}
        <div class="meta">
          <span class="path">{a.path}</span>
          <button class="mini danger" onclick={() => remove(a.path)}>Delete</button>
        </div>
      </div>
    {:else}
      <p class="muted">No assets uploaded.</p>
    {/each}
  </div>
</div>

<style>
  .am { display: flex; flex-direction: column; gap: 0.75rem; }
  header { display: flex; align-items: center; justify-content: space-between; }
  header h3 { margin: 0; }
  .x { border: none; background: none; cursor: pointer; font-size: 1rem; }
  .filebtn { display: inline-block; cursor: pointer; color: var(--accent, #1c7ed6); font-weight: 600; }
  .filebtn input { display: none; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 0.75rem; }
  .asset { border: 1px solid var(--border, #e9ecef); border-radius: 8px; overflow: hidden; }
  .asset img { width: 100%; height: 110px; object-fit: contain; background: var(--hover, #f1f3f5); display: block; }
  .placeholder { height: 110px; display: grid; place-items: center; background: var(--hover, #f1f3f5); color: var(--muted, #868e96); font-size: 0.75rem; }
  .meta { display: flex; align-items: center; justify-content: space-between; padding: 0.35rem 0.5rem; gap: 0.4rem; }
  .path { font-size: 0.75rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mini { border: 1px solid var(--border, #ced4da); background: var(--surface, #fff); border-radius: 5px; cursor: pointer; font-size: 0.72rem; padding: 0.1rem 0.4rem; }
  .mini.danger { color: var(--bad, #e03131); }
  .muted { color: var(--muted, #868e96); }
  .err { color: var(--bad, #e03131); }
</style>

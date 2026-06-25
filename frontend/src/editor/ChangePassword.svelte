<script lang="ts">
  // SPDX-License-Identifier: MIT OR Apache-2.0
  // Self-service password change (doc 13). Rendered inside App's modal backdrop.
  import { api, ApiError } from "../lib/api";

  let { onClose }: { onClose: () => void } = $props();

  let current = $state("");
  let next = $state("");
  let confirm = $state("");
  let error = $state<string | null>(null);
  let done = $state(false);
  let busy = $state(false);

  async function submit(e: Event) {
    e.preventDefault();
    if (busy) return;
    if (next !== confirm) {
      error = "New passwords do not match";
      return;
    }
    busy = true;
    error = null;
    try {
      await api.changePassword(current, next);
      done = true;
    } catch (err) {
      error = err instanceof ApiError ? err.message : String(err);
    } finally {
      busy = false;
    }
  }
</script>

<form class="cp" onsubmit={submit}>
  <header>
    <h3>Change password</h3>
    <button class="x" type="button" onclick={onClose}>✕</button>
  </header>

  {#if done}
    <p class="ok">Password changed.</p>
    <div class="actions"><button class="primary" type="button" onclick={onClose}>Done</button></div>
  {:else}
    <label>
      <span>Current password</span>
      <input type="password" bind:value={current} autocomplete="current-password" required />
    </label>
    <label>
      <span>New password</span>
      <input type="password" bind:value={next} autocomplete="new-password" required />
    </label>
    <label>
      <span>Confirm new password</span>
      <input type="password" bind:value={confirm} autocomplete="new-password" required />
    </label>
    {#if error}<p class="err">{error}</p>{/if}
    <div class="actions">
      <button class="ghost" type="button" onclick={onClose}>Cancel</button>
      <button class="primary" type="submit" disabled={busy}>{busy ? "Saving…" : "Change"}</button>
    </div>
  {/if}
</form>

<style>
  .cp {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  header h3 {
    margin: 0;
  }
  .x {
    border: none;
    background: none;
    cursor: pointer;
    font-size: 1rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: 0.85rem;
    font-weight: 600;
  }
  input {
    padding: 0.5rem 0.6rem;
    border: 1px solid var(--border, #ced4da);
    border-radius: 6px;
    font-size: 0.9rem;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }
  .primary {
    background: var(--accent, #1c7ed6);
    color: #fff;
    border: none;
    padding: 0.45rem 0.85rem;
    border-radius: 6px;
    cursor: pointer;
    font-weight: 600;
  }
  .ghost {
    background: var(--surface, #fff);
    border: 1px solid var(--border, #ced4da);
    border-radius: 6px;
    padding: 0.45rem 0.75rem;
    cursor: pointer;
  }
  .err {
    color: var(--bad, #e03131);
    font-size: 0.85rem;
    margin: 0;
  }
  .ok {
    color: var(--accent, #1c7ed6);
    margin: 0;
  }
  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>

<script lang="ts">
  // SPDX-License-Identifier: MIT OR Apache-2.0
  // First-admin setup: shown when auth is enabled but the file has no users yet.
  // Creating the admin enables auth and logs the creator in.
  import { api, ApiError } from "../lib/api";

  let { ondone }: { ondone: () => void } = $props();

  let username = $state("");
  let email = $state("");
  let password = $state("");
  let confirm = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);

  async function submit(e: Event) {
    e.preventDefault();
    if (busy) return;
    if (password !== confirm) {
      error = "Passwords do not match";
      return;
    }
    busy = true;
    error = null;
    try {
      await api.setup(username.trim(), password, email.trim() || undefined);
      ondone();
    } catch (err) {
      error = err instanceof ApiError ? err.message : String(err);
    } finally {
      busy = false;
    }
  }
</script>

<div class="auth-screen">
  <form class="auth-card" onsubmit={submit}>
    <h1>Set up team access</h1>
    <p class="sub">Create the first administrator for this file. This enables login.</p>

    <label>
      <span>Admin username</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={username} autocomplete="username" autofocus required />
    </label>
    <label>
      <span>Email <em>(optional)</em></span>
      <input type="email" bind:value={email} autocomplete="email" />
    </label>
    <label>
      <span>Password</span>
      <input type="password" bind:value={password} autocomplete="new-password" required />
    </label>
    <label>
      <span>Confirm password</span>
      <input type="password" bind:value={confirm} autocomplete="new-password" required />
    </label>

    {#if error}<p class="error">{error}</p>{/if}

    <button class="primary" type="submit" disabled={busy}>
      {busy ? "Creating…" : "Create admin & enable auth"}
    </button>
  </form>
</div>

<style>
  .auth-screen {
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1.5rem;
  }
  .auth-card {
    width: 100%;
    max-width: 380px;
    background: var(--surface, #fff);
    border: 1px solid var(--border, #e9ecef);
    border-radius: 12px;
    padding: 1.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
  }
  h1 {
    margin: 0;
    font-size: 1.3rem;
  }
  .sub {
    margin: -0.4rem 0 0.4rem;
    color: var(--muted, #868e96);
    font-size: 0.88rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: 0.85rem;
    font-weight: 600;
  }
  label em {
    color: var(--muted, #868e96);
    font-weight: 400;
  }
  input {
    padding: 0.55rem 0.65rem;
    border: 1px solid var(--border, #e9ecef);
    border-radius: 7px;
    font-size: 0.95rem;
  }
  .primary {
    margin-top: 0.4rem;
    background: var(--accent, #1c7ed6);
    color: #fff;
    border: none;
    padding: 0.6rem 0.9rem;
    border-radius: 7px;
    cursor: pointer;
    font-weight: 600;
  }
  .primary:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .error {
    margin: 0;
    color: var(--bad, #e03131);
    font-size: 0.85rem;
  }
</style>

<script lang="ts">
  // SPDX-License-Identifier: MIT OR Apache-2.0
  // Admin account management + audit log (doc 13). Rendered inside App's modal
  // backdrop. Admin-only; the button that opens it is hidden for non-admins.
  import { api, ApiError } from "../lib/api";
  import { auth, refreshAuth } from "../lib/authState.svelte";
  import type { HistoryEntry, Role, User } from "../lib/types";

  let { onClose }: { onClose: () => void } = $props();

  let users = $state<User[]>([]);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let busy = $state(false);

  // New-user form.
  let nuUsername = $state("");
  let nuPassword = $state("");
  let nuRole = $state<Role>("viewer");
  let nuEmail = $state("");

  // Audit log.
  let showAudit = $state(false);
  let audit = $state<HistoryEntry[]>([]);

  const ROLES: Role[] = ["admin", "editor", "viewer"];

  async function load() {
    try {
      users = await api.listUsers();
    } catch (e) {
      error = msg(e);
    }
  }
  load();

  function msg(e: unknown): string {
    return e instanceof ApiError ? e.message : String(e);
  }

  async function run(fn: () => Promise<void>) {
    busy = true;
    error = null;
    notice = null;
    try {
      await fn();
    } catch (e) {
      error = msg(e);
    } finally {
      busy = false;
    }
  }

  async function create(e: Event) {
    e.preventDefault();
    await run(async () => {
      await api.createUser(nuUsername.trim(), nuPassword, nuRole, nuEmail.trim() || undefined);
      nuUsername = "";
      nuPassword = "";
      nuEmail = "";
      nuRole = "viewer";
      await load();
    });
  }

  async function changeRole(u: User, role: Role) {
    if (role === u.role) return;
    await run(async () => {
      await api.updateUserRole(u.id, role);
      await load();
    });
  }

  async function reset(u: User) {
    await run(async () => {
      const { temporary_password } = await api.resetPassword(u.id);
      notice = `Temporary password for ${u.username}: ${temporary_password}`;
    });
  }

  async function remove(u: User) {
    if (!confirm(`Delete user "${u.username}"?`)) return;
    await run(async () => {
      await api.deleteUser(u.id);
      await load();
    });
  }

  async function unlock(u: User) {
    await run(async () => {
      await api.unlockUser(u.id);
      notice = `Cleared login lockout for ${u.username}.`;
    });
  }

  async function toggleAudit() {
    showAudit = !showAudit;
    if (showAudit && audit.length === 0) {
      await run(async () => {
        audit = await api.audit({ limit: 100 });
      });
    }
  }

  async function disableAuth() {
    if (!confirm("Turn auth off? Accounts are kept, but anyone with the file can open it.")) return;
    await run(async () => {
      await api.disableAuth();
      await refreshAuth();
      onClose();
    });
  }
</script>

<div class="um">
  <header>
    <h3>Users &amp; access</h3>
    <button class="x" onclick={onClose}>✕</button>
  </header>

  {#if error}<p class="err">{error}</p>{/if}
  {#if notice}<p class="notice">{notice}</p>{/if}

  <table>
    <thead>
      <tr><th>User</th><th>Role</th><th>Last login</th><th></th></tr>
    </thead>
    <tbody>
      {#each users as u (u.id)}
        <tr>
          <td>
            <span class="name">{u.username}</span>
            {#if u.id === auth.user?.id}<span class="you">you</span>{/if}
            {#if u.email}<span class="email">{u.email}</span>{/if}
          </td>
          <td>
            <select
              value={u.role}
              disabled={busy}
              onchange={(e) => changeRole(u, (e.currentTarget as HTMLSelectElement).value as Role)}
            >
              {#each ROLES as r}<option value={r}>{r}</option>{/each}
            </select>
          </td>
          <td class="muted">{u.last_login_at ? u.last_login_at.slice(0, 16).replace("T", " ") : "—"}</td>
          <td class="row-actions">
            <button class="mini" onclick={() => reset(u)} disabled={busy}>Reset pw</button>
            <button class="mini" onclick={() => unlock(u)} disabled={busy}>Unlock</button>
            <button class="mini danger" onclick={() => remove(u)} disabled={busy}>Delete</button>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>

  <form class="newuser" onsubmit={create}>
    <h4>Add user</h4>
    <div class="fields">
      <input placeholder="username" bind:value={nuUsername} required />
      <input type="password" placeholder="password" bind:value={nuPassword} required />
      <input type="email" placeholder="email (optional)" bind:value={nuEmail} />
      <select bind:value={nuRole}>
        {#each ROLES as r}<option value={r}>{r}</option>{/each}
      </select>
      <button class="primary" type="submit" disabled={busy}>Add</button>
    </div>
  </form>

  <div class="footer">
    <button class="ghost" onclick={toggleAudit} disabled={busy}>
      {showAudit ? "Hide" : "Show"} audit log
    </button>
    <button class="ghost danger" onclick={disableAuth} disabled={busy}>Disable auth</button>
  </div>

  {#if showAudit}
    <div class="audit">
      {#if audit.length === 0}
        <p class="muted">No audit entries.</p>
      {:else}
        <table>
          <thead><tr><th>When</th><th>Event</th><th>Entity</th><th>User</th></tr></thead>
          <tbody>
            {#each audit as a (a.id)}
              <tr>
                <td class="muted">{a.occurred_at.slice(0, 19).replace("T", " ")}</td>
                <td>{a.event_kind}</td>
                <td class="muted">{a.entity_id ?? "—"}</td>
                <td class="muted">{a.user_id ?? "—"}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  {/if}
</div>

<style>
  .um {
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
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
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }
  th {
    text-align: left;
    color: var(--muted, #868e96);
    font-weight: 600;
    border-bottom: 1px solid var(--border, #e9ecef);
    padding: 0.3rem 0.4rem;
  }
  td {
    padding: 0.4rem;
    border-bottom: 1px solid var(--border, #f1f3f5);
    vertical-align: middle;
  }
  .name {
    font-weight: 600;
  }
  .you {
    margin-left: 0.35rem;
    font-size: 0.7rem;
    color: var(--accent, #1c7ed6);
    border: 1px solid var(--accent, #1c7ed6);
    border-radius: 4px;
    padding: 0 0.25rem;
  }
  .email {
    display: block;
    color: var(--muted, #868e96);
    font-size: 0.75rem;
  }
  .row-actions {
    display: flex;
    gap: 0.3rem;
    justify-content: flex-end;
  }
  select {
    padding: 0.25rem 0.4rem;
    border: 1px solid var(--border, #ced4da);
    border-radius: 5px;
  }
  .newuser h4 {
    margin: 0.5rem 0 0.4rem;
  }
  .fields {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .fields input {
    flex: 1 1 120px;
    padding: 0.4rem 0.5rem;
    border: 1px solid var(--border, #ced4da);
    border-radius: 6px;
    font-size: 0.85rem;
  }
  .footer {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    border-top: 1px solid var(--border, #f1f3f5);
    padding-top: 0.6rem;
  }
  .audit {
    max-height: 240px;
    overflow-y: auto;
  }
  .primary {
    background: var(--accent, #1c7ed6);
    color: #fff;
    border: none;
    padding: 0.4rem 0.8rem;
    border-radius: 6px;
    cursor: pointer;
    font-weight: 600;
  }
  .ghost {
    background: var(--surface, #fff);
    border: 1px solid var(--border, #ced4da);
    border-radius: 6px;
    padding: 0.4rem 0.7rem;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .mini {
    border: 1px solid var(--border, #ced4da);
    background: var(--surface, #fff);
    border-radius: 5px;
    cursor: pointer;
    font-size: 0.72rem;
    padding: 0.15rem 0.45rem;
  }
  .mini.danger,
  .ghost.danger {
    color: var(--bad, #e03131);
  }
  .muted {
    color: var(--muted, #868e96);
  }
  .err {
    color: var(--bad, #e03131);
  }
  .notice {
    color: var(--accent, #1c7ed6);
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.82rem;
    word-break: break-all;
  }
  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>

<script lang="ts">
  import { onMount } from "svelte";
  import { route } from "./lib/router";
  import { auth, refreshAuth, doLogout, isAdmin } from "./lib/authState.svelte";
  import DashboardList from "./routes/DashboardList.svelte";
  import Viewer from "./routes/Viewer.svelte";
  import Editor from "./routes/Editor.svelte";
  import Login from "./routes/Login.svelte";
  import Setup from "./routes/Setup.svelte";
  import UserManager from "./editor/UserManager.svelte";
  import ChangePassword from "./editor/ChangePassword.svelte";

  const current = $derived($route);

  let showUsers = $state(false);
  let showPassword = $state(false);

  onMount(() => {
    refreshAuth();
  });
</script>

{#if !auth.loaded}
  <div class="auth-loading">Loading…</div>
{:else if auth.enabled && auth.needsSetup}
  <Setup ondone={refreshAuth} />
{:else if auth.enabled && !auth.user}
  <Login ondone={refreshAuth} />
{:else}
  {#if auth.enabled && auth.user}
    <div class="authbar">
      <span class="who">
        <strong>{auth.user.username}</strong>
        <span class="role">{auth.user.role}</span>
      </span>
      <span class="spacer"></span>
      {#if isAdmin()}
        <button class="link" onclick={() => (showUsers = true)}>Users</button>
      {/if}
      <button class="link" onclick={() => (showPassword = true)}>Password</button>
      <button class="link" onclick={doLogout}>Sign out</button>
    </div>
  {/if}

  <div class="content" class:has-bar={auth.enabled && !!auth.user}>
    {#if current.name === "list"}
      <DashboardList />
    {:else if current.name === "view"}
      {#key current.id}<Viewer id={current.id} query={current.query} />{/key}
    {:else if current.name === "edit"}
      {#key current.id}<Editor id={current.id} />{/key}
    {:else}
      <div class="notfound">
        <p>Not found: <code>{current.path}</code></p>
        <a href="#/">← Back to dashboards</a>
      </div>
    {/if}
  </div>

  {#if showUsers}
    <div
      class="modal-backdrop"
      onclick={() => (showUsers = false)}
      onkeydown={(e) => e.key === "Escape" && (showUsers = false)}
      role="presentation"
    >
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="modal wide" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
        <UserManager onClose={() => (showUsers = false)} />
      </div>
    </div>
  {/if}

  {#if showPassword}
    <div
      class="modal-backdrop"
      onclick={() => (showPassword = false)}
      onkeydown={(e) => e.key === "Escape" && (showPassword = false)}
      role="presentation"
    >
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
        <ChangePassword onClose={() => (showPassword = false)} />
      </div>
    </div>
  {/if}
{/if}

<style>
  .auth-loading {
    display: grid;
    place-items: center;
    min-height: 100vh;
    color: var(--muted, #868e96);
  }
  .authbar {
    position: sticky;
    top: 0;
    z-index: 40;
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.4rem 1rem;
    background: var(--surface, #fff);
    border-bottom: 1px solid var(--border, #e9ecef);
    font-size: 0.85rem;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .role {
    color: var(--muted, #868e96);
    border: 1px solid var(--border, #e9ecef);
    border-radius: 4px;
    padding: 0 0.3rem;
    font-size: 0.72rem;
  }
  .spacer {
    flex: 1;
  }
  .link {
    background: none;
    border: none;
    color: var(--accent, #1c7ed6);
    cursor: pointer;
    font-size: 0.85rem;
    padding: 0.2rem 0.3rem;
  }
  .notfound {
    max-width: 600px;
    margin: 4rem auto;
    text-align: center;
  }
  .notfound a {
    color: var(--accent, #1c7ed6);
  }
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: grid;
    place-items: center;
    z-index: 60;
  }
  .modal {
    background: var(--surface, #fff);
    border-radius: 10px;
    padding: 1.25rem;
    width: min(440px, 92vw);
  }
  .modal.wide {
    width: min(820px, 94vw);
    max-height: 85vh;
    overflow-y: auto;
  }
</style>

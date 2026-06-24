<script lang="ts">
  import { route } from "./lib/router";
  import DashboardList from "./routes/DashboardList.svelte";
  import Viewer from "./routes/Viewer.svelte";
  import Editor from "./routes/Editor.svelte";

  const current = $derived($route);
</script>

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

<style>
  .notfound {
    max-width: 600px;
    margin: 4rem auto;
    text-align: center;
  }
  .notfound a {
    color: var(--accent, #1c7ed6);
  }
</style>

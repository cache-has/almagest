<script lang="ts">
  import type { DividerPanel } from "../../lib/types";

  let { panel }: { panel: DividerPanel } = $props();
</script>

<div class="divider" class:labeled={!!panel.label}>
  {#if panel.label}
    <span class="label">{panel.label}</span>
  {/if}
  <hr />
</div>

<style>
  .divider {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    width: 100%;
    height: 100%;
  }
  .label {
    font-size: 0.8rem;
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--muted, #868e96);
    white-space: nowrap;
  }
  hr {
    flex: 1;
    border: none;
    border-top: 1px solid var(--border, #dee2e6);
  }
</style>

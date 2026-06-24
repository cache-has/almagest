<script lang="ts">
  import type { Parameter } from "../lib/types";
  import ParamInput from "./inputs/ParamInput.svelte";

  let {
    parameters,
    values,
    dashboardId,
    disabled = false,
    onSetParam,
  }: {
    parameters: Parameter[];
    values: Record<string, unknown>;
    dashboardId: string;
    disabled?: boolean;
    onSetParam: (name: string, value: unknown) => void;
  } = $props();
</script>

{#if parameters.length > 0}
  <div class="param-bar">
    {#each parameters as param (param.id)}
      <ParamInput
        {param}
        {dashboardId}
        {disabled}
        value={values[param.id]}
        onChange={(v) => onSetParam(param.id, v)}
      />
    {/each}
  </div>
{/if}

<style>
  .param-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem 1.5rem;
    align-items: flex-end;
    padding: 0.85rem 1rem;
    background: var(--surface, #fff);
    border: 1px solid var(--border, #e9ecef);
    border-radius: 10px;
    margin-bottom: 1rem;
  }
</style>

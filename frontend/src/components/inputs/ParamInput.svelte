<script lang="ts">
  import type { Parameter } from "../../lib/types";
  import { api } from "../../lib/api";

  let {
    param,
    value,
    dashboardId,
    onChange,
  }: {
    param: Parameter;
    value: unknown;
    dashboardId: string;
    onChange: (value: unknown) => void;
  } = $props();

  // For select/multiselect, resolve options (static or via options_query).
  let options = $state<string[]>([]);
  $effect(() => {
    if ((param.kind === "select" || param.kind === "multiselect") && param.options_query) {
      api
        .resolveOptions(dashboardId, param.id)
        .then((o) => (options = o))
        .catch(() => (options = param.options ?? []));
    } else {
      options = param.options ?? [];
    }
  });

  const range = $derived(
    param.kind === "daterange"
      ? (value as { start?: string; end?: string }) ?? { start: "", end: "" }
      : { start: "", end: "" },
  );

  function multiToggle(opt: string, checked: boolean) {
    const cur = Array.isArray(value) ? [...(value as string[])] : [];
    onChange(checked ? [...cur, opt] : cur.filter((v) => v !== opt));
  }

  // Debounce free-text typing (text/number) so panels don't re-query on every
  // keystroke; blur/Enter flushes immediately. Selects, dates, booleans and
  // multiselect commit on the spot — there's no per-character churn there.
  const DEBOUNCE_MS = 300;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pending: unknown = undefined;

  function debouncedChange(v: unknown) {
    pending = v;
    clearTimeout(timer);
    timer = setTimeout(() => {
      timer = undefined;
      onChange(pending);
    }, DEBOUNCE_MS);
  }

  function flush() {
    if (timer === undefined) return;
    clearTimeout(timer);
    timer = undefined;
    onChange(pending);
  }
</script>

<label class="param">
  <span class="label">{param.label ?? param.id}</span>

  {#if param.kind === "text"}
    <input
      type="text"
      value={value as string}
      oninput={(e) => debouncedChange(e.currentTarget.value)}
      onchange={flush}
    />
  {:else if param.kind === "number"}
    <input
      type="number"
      value={value as number}
      min={param.min}
      max={param.max}
      oninput={(e) => debouncedChange(e.currentTarget.valueAsNumber)}
      onchange={flush}
    />
  {:else if param.kind === "boolean"}
    <input type="checkbox" checked={!!value} onchange={(e) => onChange(e.currentTarget.checked)} />
  {:else if param.kind === "date"}
    <input type="date" value={value as string} oninput={(e) => onChange(e.currentTarget.value)} />
  {:else if param.kind === "daterange"}
    <span class="range">
      <input type="date" value={range.start} oninput={(e) => onChange({ ...range, start: e.currentTarget.value })} />
      <span class="dash">–</span>
      <input type="date" value={range.end} oninput={(e) => onChange({ ...range, end: e.currentTarget.value })} />
    </span>
  {:else if param.kind === "select"}
    <select value={value as string} onchange={(e) => onChange(e.currentTarget.value)}>
      {#if param.allow_all}<option value="All">All</option>{/if}
      {#each options as opt (opt)}<option value={opt}>{opt}</option>{/each}
    </select>
  {:else if param.kind === "multiselect"}
    <span class="multi">
      {#each options as opt (opt)}
        <label class="chk">
          <input
            type="checkbox"
            checked={Array.isArray(value) && (value as string[]).includes(opt)}
            onchange={(e) => multiToggle(opt, e.currentTarget.checked)}
          />
          {opt}
        </label>
      {/each}
    </span>
  {/if}

  {#if param.description}<span class="hint">{param.description}</span>{/if}
</label>

<style>
  .param {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.8rem;
  }
  .label {
    font-weight: 600;
    color: var(--muted, #495057);
  }
  input[type="text"],
  input[type="number"],
  input[type="date"],
  select {
    padding: 0.3rem 0.45rem;
    border: 1px solid var(--border, #ced4da);
    border-radius: 6px;
    font-size: 0.85rem;
    background: var(--surface, #fff);
  }
  .range {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }
  .multi {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .chk {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-weight: 400;
  }
  .hint {
    color: var(--muted, #adb5bd);
    font-size: 0.72rem;
  }
</style>

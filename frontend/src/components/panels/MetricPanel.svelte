<script lang="ts">
  import type { MetricPanel } from "../../lib/types";
  import type { ArrowResult } from "../../lib/arrow";
  import { applyFormat, toNumber, NULL_DISPLAY } from "../../lib/format";

  let { panel, result }: { panel: MetricPanel; result: ArrowResult | null } = $props();

  const row = $derived(result?.rows[0] ?? null);
  const value = $derived(row ? row["value"] : null);
  const display = $derived(value === undefined || value === null ? NULL_DISPLAY : applyFormat(panel.format, value));

  const delta = $derived.by(() => {
    if (!panel.comparison || !row) return null;
    const cur = toNumber(value);
    const prev = toNumber(row[panel.comparison.previous_field]);
    if (cur === null || prev === null) return null;
    const fmt = panel.comparison.delta_format ?? "percent";
    const raw = fmt === "percent" ? (prev === 0 ? null : (cur - prev) / prev) : cur - prev;
    if (raw === null) return null;
    const text =
      fmt === "percent"
        ? `${raw >= 0 ? "+" : ""}${(raw * 100).toFixed(1)}%`
        : `${raw >= 0 ? "+" : ""}${applyFormat(panel.format, cur - prev)}`;
    return { raw, text };
  });

  const tone = $derived.by(() => {
    if (!delta || !panel.comparison) return "neutral";
    const dir = panel.comparison.direction ?? "higher_better";
    if (dir === "neutral") return "neutral";
    const good = dir === "higher_better" ? delta.raw > 0 : delta.raw < 0;
    if (delta.raw === 0) return "neutral";
    return good ? "good" : "bad";
  });
</script>

<div class="metric">
  <div class="value">{display}</div>
  {#if delta}
    <div class="delta {tone}">{delta.text}</div>
  {/if}
</div>

<style>
  .metric {
    display: flex;
    flex-direction: column;
    justify-content: center;
    height: 100%;
    gap: 0.35rem;
  }
  .value {
    font-size: 2.2rem;
    font-weight: 650;
    letter-spacing: -0.02em;
    line-height: 1.1;
  }
  .delta {
    font-size: 0.9rem;
    font-weight: 600;
  }
  .delta.good {
    color: var(--good, #2f9e44);
  }
  .delta.bad {
    color: var(--bad, #e03131);
  }
  .delta.neutral {
    color: var(--muted, #868e96);
  }
</style>

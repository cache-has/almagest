<script lang="ts">
  import type { TextPanel } from "../../lib/types";
  import { marked } from "marked";
  import DOMPurify from "dompurify";

  let { panel }: { panel: TextPanel } = $props();

  // Markdown → sanitized HTML (no script/style/event handlers survive purify).
  const html = $derived(DOMPurify.sanitize(marked.parse(panel.content ?? "", { async: false }) as string));
</script>

<!-- eslint-disable-next-line svelte/no-at-html-tags -->
<div class="text">{@html html}</div>

<style>
  .text {
    line-height: 1.5;
    font-size: 0.95rem;
  }
  .text :global(h1),
  .text :global(h2),
  .text :global(h3) {
    margin: 0.4em 0 0.3em;
    letter-spacing: -0.01em;
  }
  .text :global(p) {
    margin: 0.4em 0;
  }
  .text :global(a) {
    color: var(--accent, #1c7ed6);
  }
  .text :global(code) {
    background: var(--code-bg, #f1f3f5);
    padding: 0.1em 0.35em;
    border-radius: 4px;
    font-size: 0.85em;
  }
  .text :global(table) {
    border-collapse: collapse;
  }
  .text :global(td),
  .text :global(th) {
    border: 1px solid var(--border, #dee2e6);
    padding: 0.25em 0.5em;
  }
</style>

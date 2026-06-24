<script lang="ts">
  import type { TablePanel, SortDirection } from "../../lib/types";
  import type { ArrowResult } from "../../lib/arrow";
  import { applyFormat, toNumber, plain } from "../../lib/format";

  let {
    panel,
    result,
    onCellClick,
  }: {
    panel: TablePanel;
    result: ArrowResult | null;
    onCellClick?: (row: Record<string, unknown>, column: string) => void;
  } = $props();

  const columns = $derived(result?.fields.map((f) => f.name) ?? []);

  let sortCol = $state<string | null>(null);
  let sortDir = $state<SortDirection>("asc");
  let page = $state(0);

  // Apply the declared default sort once a result arrives.
  $effect(() => {
    if (panel.sort_default && sortCol === null) {
      sortCol = panel.sort_default.column;
      sortDir = panel.sort_default.direction ?? "asc";
    }
  });

  const sortedRows = $derived.by(() => {
    const rows = result?.rows ?? [];
    if (!sortCol) return rows;
    const col = sortCol;
    const dir = sortDir === "asc" ? 1 : -1;
    return [...rows].sort((a, b) => dir * compare(a[col], b[col]));
  });

  const pageSize = $derived(panel.page_size ?? 0);
  const pageCount = $derived(pageSize > 0 ? Math.max(1, Math.ceil(sortedRows.length / pageSize)) : 1);

  // Virtualization: when there's no pagination and the result is large, render
  // only the rows in (and around) the viewport so scrolling stays smooth. Cells
  // are `white-space: nowrap`, so a fixed row height is safe.
  const VIRT_THRESHOLD = 100;
  const ROW_H = 29; // px — must match the rendered <td> height
  const OVERSCAN = 8;
  const virtualized = $derived(pageSize === 0 && sortedRows.length > VIRT_THRESHOLD);

  let scrollTop = $state(0);
  let viewportH = $state(360);

  const startIndex = $derived(
    virtualized ? Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN) : 0,
  );
  const endIndex = $derived(
    virtualized
      ? Math.min(sortedRows.length, startIndex + Math.ceil(viewportH / ROW_H) + OVERSCAN * 2)
      : sortedRows.length,
  );
  const topPad = $derived(virtualized ? startIndex * ROW_H : 0);
  const bottomPad = $derived(virtualized ? (sortedRows.length - endIndex) * ROW_H : 0);

  const visibleRows = $derived.by(() => {
    if (pageSize > 0) return sortedRows.slice(page * pageSize, page * pageSize + pageSize);
    if (virtualized) return sortedRows.slice(startIndex, endIndex);
    return sortedRows;
  });

  function onScroll(e: Event) {
    scrollTop = (e.currentTarget as HTMLElement).scrollTop;
  }

  function compare(a: unknown, b: unknown): number {
    const na = toNumber(a);
    const nb = toNumber(b);
    if (na !== null && nb !== null) return na - nb;
    return plain(a).localeCompare(plain(b));
  }

  function headerLabel(col: string): string {
    return panel.columns?.[col]?.label ?? col;
  }

  function cell(col: string, value: unknown): string {
    return applyFormat(panel.columns?.[col]?.format, value);
  }

  function toggleSort(col: string) {
    if (!panel.sortable) return;
    if (sortCol === col) {
      sortDir = sortDir === "asc" ? "desc" : "asc";
    } else {
      sortCol = col;
      sortDir = "asc";
    }
    page = 0;
  }
</script>

<div
  class="table-wrap"
  class:virt={virtualized}
  onscroll={virtualized ? onScroll : undefined}
  bind:clientHeight={viewportH}
>
  <table>
    <thead>
      <tr>
        {#each columns as col (col)}
          <th
            class:sortable={panel.sortable}
            style:width={panel.columns?.[col]?.width}
            onclick={() => toggleSort(col)}
          >
            {headerLabel(col)}
            {#if sortCol === col}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#if topPad > 0}
        <tr class="spacer" style:height={`${topPad}px`}><td colspan={Math.max(1, columns.length)}></td></tr>
      {/if}
      {#each visibleRows as row, i (i)}
        <tr>
          {#each columns as col (col)}
            <td
              class:clickable={!!onCellClick}
              onclick={() => onCellClick?.(row, col)}
            >
              {cell(col, row[col])}
            </td>
          {/each}
        </tr>
      {/each}
      {#if bottomPad > 0}
        <tr class="spacer" style:height={`${bottomPad}px`}><td colspan={Math.max(1, columns.length)}></td></tr>
      {/if}
      {#if visibleRows.length === 0}
        <tr><td class="empty" colspan={Math.max(1, columns.length)}>No rows</td></tr>
      {/if}
    </tbody>
  </table>

  {#if pageSize > 0 && pageCount > 1}
    <div class="pager">
      <button disabled={page === 0} onclick={() => (page = Math.max(0, page - 1))}>‹</button>
      <span>{page + 1} / {pageCount}</span>
      <button disabled={page >= pageCount - 1} onclick={() => (page = Math.min(pageCount - 1, page + 1))}>›</button>
    </div>
  {/if}
</div>

<style>
  .table-wrap {
    overflow: auto;
    height: 100%;
  }
  /* Bound the height when virtualizing so the viewport actually scrolls. */
  .table-wrap.virt {
    max-height: 420px;
  }
  .spacer td {
    padding: 0;
    border: none;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 0.85rem;
  }
  th,
  td {
    text-align: left;
    padding: 0.35rem 0.6rem;
    border-bottom: 1px solid var(--border, #e9ecef);
    white-space: nowrap;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--surface, #fff);
    font-weight: 650;
    color: var(--muted, #495057);
  }
  th.sortable {
    cursor: pointer;
    user-select: none;
  }
  .arrow {
    font-size: 0.65em;
    margin-left: 0.25em;
  }
  td.clickable {
    cursor: pointer;
  }
  td.clickable:hover {
    background: var(--hover, #f1f3f5);
  }
  .empty {
    text-align: center;
    color: var(--muted, #868e96);
    padding: 1rem;
  }
  .pager {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.5rem;
    padding: 0.4rem 0.2rem 0;
    font-size: 0.8rem;
  }
  .pager button {
    border: 1px solid var(--border, #dee2e6);
    background: var(--surface, #fff);
    border-radius: 4px;
    cursor: pointer;
    padding: 0.1rem 0.5rem;
  }
  .pager button:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>

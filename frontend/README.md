# Almagest frontend

The web UI for Almagest — one Svelte 5 + TypeScript app, built with Vite, that
talks to `almagest-server` over a JSON HTTP API and nothing else. The same bundle
powers every interactive deploy mode (desktop browser-serve, headless serve,
embedded-at-a-URL) and is embedded into the Rust binary at build time via
`rust-embed`.

**Status:** Phase 09 foundation built. Vite + Svelte 5 + TS SPA over the Phase 08
JSON API, with the shared rendering pipeline (grid, all six panel kinds incl.
ECharts charts, parameter bar), a read-only **Viewer**, and a functional minimal
**Studio editor**. See `planning/09-frontend-editor.md` for what's done vs.
deferred (Monaco/CodeMirror, drag-resize, data-manager ingest, asset upload,
saved-query manager, undo/redo, Playwright e2e). Viewer-specific polish is
`planning/10-frontend-viewer.md`.

## Develop

```sh
just frontend-install     # npm install
npm run dev               # Vite dev server, proxies /api → localhost:8080
# in another shell: almagest serve some.alm --port 8080
```

A production build (`just frontend` or `npm run build`) emits `frontend/dist`,
which `almagest-server` embeds via `rust-embed`. Type-check with `npm run check`.

## Architectural commitments (decided up front)

- **Renderer:** client-side JS (Svelte 5 + ECharts), *not* a native Rust GUI and
  *not* server-rendered HTML. The browser is the rendering surface in every
  deploy mode, so one web frontend serves all of them.
- **Charts:** ECharts (best-in-class interactive BI viz; rebuilding it in Rust
  is the reinvention to avoid).
- **Data boundary:** the app speaks JSON over HTTP through a single client
  module. Desktop (browser-serve now, Tauri shell later) changes only *who owns
  the window*, never the data path.

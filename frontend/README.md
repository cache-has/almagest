# Almagest frontend

The web UI for Almagest — one Svelte 5 + TypeScript app, built with Vite, that
talks to `almagest-server` over a JSON HTTP API and nothing else. The same bundle
powers every interactive deploy mode (desktop browser-serve, headless serve,
embedded-at-a-URL) and is embedded into the Rust binary at build time via
`rust-embed`.

**Status:** intentionally deferred. The toolchain (Vite, Svelte 5, TS) is stood
up in Phase 09 (frontend editor) / Phase 10 (viewer), once the server exposes an
API worth rendering. Scaffolding it earlier would only rot. See
`planning/09-frontend-editor.md` and `planning/10-frontend-viewer.md`.

## Architectural commitments (decided up front)

- **Renderer:** client-side JS (Svelte 5 + ECharts), *not* a native Rust GUI and
  *not* server-rendered HTML. The browser is the rendering surface in every
  deploy mode, so one web frontend serves all of them.
- **Charts:** ECharts (best-in-class interactive BI viz; rebuilding it in Rust
  is the reinvention to avoid).
- **Data boundary:** the app speaks JSON over HTTP through a single client
  module. Desktop (browser-serve now, Tauri shell later) changes only *who owns
  the window*, never the data path.

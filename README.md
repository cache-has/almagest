<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Almagest

**Dashboards as files, not services.**

Almagest is a single-file, self-contained BI and dashboarding tool. The unit of
distribution is one file you double-click. Dashboards, data, query engine, UI
assets, and cached results all live inside a single SQLite `.alm` file — no
server to administer, no Docker, no database to back up. Backup is `cp`.

> **Status:** early development (0.x, pre-release). The `.alm` format is
> unstable until 1.0. Don't commit `.alm` files to long-term storage yet.

## Why

Hosted BI tools (Metabase, Superset, Redash) can't be emailed, committed to git,
embedded in another product, handed to a client, or run air-gapped. A *file* can.
Almagest is for the people who were never going to stand up Metabase in the first
place.

## Workspace layout

| Crate | Role |
|-------|------|
| `almagest-core` | The `.alm` file format, schema, migrations, models |
| `almagest-query` | Multi-backend query engine, caching, parameterization |
| `almagest-connectors` | Data source drivers (SQLite, DuckDB, Postgres, MySQL, files, REST) |
| `almagest-server` | Axum HTTP server + embedded frontend |
| `almagest-cli` | The `almagest` binary (`new`, `open`, `serve`, `export`) |
| `almagest-embed` | Embedding API for bundling Almagest into a host program |
| `frontend/` | Svelte 5 + ECharts web UI (deferred to Phase 09) |

## Development

```sh
just build    # build the workspace
just test     # run all tests
just check    # fmt + clippy + tests (CI gate)
just dev -- --help
```

Requires the Rust toolchain pinned in `rust-toolchain.toml` and `just`.

## License

Dual-licensed under [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE), at your
option. © 2026 Horizon Analytic Studios, LLC.

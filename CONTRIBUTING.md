# Contributing to Almagest

Thanks for your interest in Almagest.

## Philosophy

Good software comes from active collaboration, not blind obedience. If you see a
better approach, a potential problem, or think a design decision is misguided —
say so, with reasoning. Pushback is welcome. The goal is the best possible
outcome, not agreement.

Almagest's north star: **dashboards should be files, not services.** Every change
should preserve the property that a `.alm` file is self-contained, portable,
and openable on any supported platform with identical behavior.

## Development

Prerequisites: a Rust toolchain (pinned in `rust-toolchain.toml`), `just`, and
Node.js (for the frontend, once it exists).

```sh
just build    # build the workspace
just test     # run all tests
just check    # fmt + clippy + tests (the gate CI enforces)
just dev -- --version
```

## Conventions

- Rust throughout the backend, edition 2024, `rustfmt` with `max_width = 100`.
- Libraries use `thiserror` for error types; the CLI uses `anyhow`.
- Every source file carries an SPDX header: `// SPDX-License-Identifier: MIT OR Apache-2.0`.
- Keep the dependency surface small. A new dependency should earn its place.

## Licensing

Almagest is dual-licensed under MIT OR Apache-2.0. By contributing, you agree that
your contributions are licensed under the same terms.

## Contact

- Author: Cache McClure — cache@horizonanalytic.com
- Company: Horizon Analytic Studios, LLC

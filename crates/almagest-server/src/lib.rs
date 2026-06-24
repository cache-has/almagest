// SPDX-License-Identifier: MIT OR Apache-2.0

//! # almagest-server
//!
//! The Axum HTTP server that backs every interactive deploy mode (desktop
//! browser-serve, headless `serve`, and embedded-at-a-URL). It exposes a JSON
//! API over a `.alm` file and serves the embedded frontend bundle.
//!
//! The frontend talks to this server over HTTP and nothing else — keeping the
//! desktop-vs-browser-vs-Tauri choice a thin outer ring rather than a fork.
//!
//! This crate is a stub during Phase 01; the API lands in Phase 08.

/// Build information for the running server.
pub fn server_version() -> &'static str {
    almagest_core::ALMAGEST_VERSION
}

// SPDX-License-Identifier: MIT OR Apache-2.0

//! Serving the embedded frontend bundle.
//!
//! The Svelte build in `frontend/dist` is baked into the binary at compile time
//! by `rust-embed` (the folder is guaranteed to exist by `build.rs`). `/` and
//! any unmatched non-API route serve `index.html` (SPA client-side routing);
//! Vite's content-hashed assets get an immutable, long-lived cache while
//! `index.html` is always revalidated so new builds are picked up on reload.

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

// Path is relative to this crate's manifest dir; `build.rs` guarantees it exists.
#[derive(RustEmbed)]
#[folder = "../../frontend/dist"]
struct FrontendAssets;

/// Serve `index.html` for the SPA shell at `/` and `/dashboard/:name`.
pub async fn serve_index() -> Response {
    serve_path("index.html")
}

/// Catch-all for any route the API router didn't claim: serve the requested
/// embedded asset, falling back to `index.html` for client-side routes.
pub async fn serve_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.is_empty() {
        return serve_path("index.html");
    }
    match FrontendAssets::get(path) {
        Some(_) => serve_path(path),
        // Unknown path that isn't an asset → SPA route; hand back the shell.
        None => serve_path("index.html"),
    }
}

/// Serve a single embedded file by its bundle-relative path.
fn serve_path(path: &str) -> Response {
    let Some(file) = FrontendAssets::get(path) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    // index.html must always revalidate so a new build is seen on reload;
    // hashed assets are safe to cache aggressively.
    let cache = if path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    (
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, cache),
        ],
        file.data,
    )
        .into_response()
}

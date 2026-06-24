// SPDX-License-Identifier: MIT OR Apache-2.0

//! # almagest-server
//!
//! The Axum HTTP server that backs every interactive deploy mode (desktop
//! browser-serve, headless `serve`, and embedded-at-a-URL). It serves the
//! embedded Svelte bundle and a JSON-over-HTTP API over one open `.alm` file.
//!
//! The frontend talks to this server over HTTP and nothing else — keeping the
//! desktop-vs-browser-vs-Tauri choice a thin outer ring rather than a fork, and
//! letting the interactive-HTML export swap *this* backend for DuckDB-WASM
//! behind the same client boundary (doc 08).
//!
//! ## Shape (Phase 08)
//!
//! - Runtime/viewer API: file metadata, dashboard list/detail, schema
//!   introspection, parameter options, and **panel execution → Arrow IPC**.
//! - Editor CRUD: dashboard create/update/delete and JSON import/export.
//! - Static serving of the embedded frontend with SPA fallback.
//! - A WebSocket events channel and a graceful-shutdown endpoint.
//!
//! There are **no connection endpoints** — Almagest is embedded-only, so the
//! doc's live-connection routes are cut. Saved-query write CRUD, ad-hoc query
//! preview, asset upload, and auth land with later phases.

mod api;
mod datasets;
mod error;
mod events;
mod export;
mod state;
mod static_assets;

#[cfg(test)]
mod tests;

pub use error::{ApiError, ApiResult};
pub use export::export_snapshot_html;
pub use state::{AppState, ServerEvent};

use almagest_core::AlmagestFile;
use almagest_query::AlmagestQueryContext;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tower_http::trace::TraceLayer;

/// Errors raised while starting or running the server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// Opening the `.alm` file failed.
    #[error(transparent)]
    Core(#[from] almagest_core::AlmagestError),
    /// Building the query engine over the file's data failed.
    #[error(transparent)]
    Query(#[from] almagest_query::QueryError),
    /// Binding the listener or an I/O fault during serve.
    #[error("server io error: {0}")]
    Io(#[from] std::io::Error),
    /// Arrow IPC encoding failed while baking a snapshot.
    #[error("arrow encode error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    /// Serializing the snapshot payload failed.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    /// An export precondition failed (e.g. the file has no dashboards).
    #[error("{0}")]
    Export(String),
}

/// How to bind and run the server.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// Address to bind. Desktop mode uses an ephemeral port (`127.0.0.1:0`);
    /// headless mode pins a chosen port (loopback unless `--host` widens it).
    pub bind_addr: SocketAddr,
    /// Open the system browser at the bound URL once listening (desktop mode).
    pub open_browser: bool,
    /// Apply a permissive CORS policy — for embedded mode where the host page is
    /// on a different origin. Off by default (localhost desktop needs none).
    pub enable_cors: bool,
    /// Serve the file read-only: every mutating endpoint returns 403.
    pub read_only: bool,
    /// Run the heartbeat watchdog: once the frontend's pings stop (the last tab
    /// closed), shut down. Desktop lifecycle; off for long-lived headless serve.
    pub heartbeat_shutdown: bool,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            open_browser: false,
            enable_cors: false,
            read_only: false,
            heartbeat_shutdown: false,
        }
    }
}

impl ServerOptions {
    /// Desktop mode: ephemeral localhost port, auto-open the browser, and a
    /// heartbeat watchdog so closing the last tab exits the process.
    pub fn desktop() -> Self {
        Self {
            open_browser: true,
            heartbeat_shutdown: true,
            ..Self::default()
        }
    }

    /// Headless mode: bind a fixed port on loopback, no browser, long-lived.
    /// Widen the host with [`ServerOptions::with_bind`] (`--host 0.0.0.0`).
    pub fn headless(port: u16) -> Self {
        Self {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], port)),
            ..Self::default()
        }
    }

    /// Override the bind address (host + port).
    pub fn with_bind(mut self, addr: SocketAddr) -> Self {
        self.bind_addr = addr;
        self
    }

    /// Serve the file read-only.
    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Override whether the browser auto-opens.
    pub fn with_open_browser(mut self, open: bool) -> Self {
        self.open_browser = open;
        self
    }
}

/// A running server. Holds the bound address, the serving task, and a handle to
/// trigger graceful shutdown.
pub struct ServerHandle {
    addr: SocketAddr,
    task: JoinHandle<()>,
    shutdown: Arc<Notify>,
}

impl ServerHandle {
    /// The address the server is actually listening on (resolves the ephemeral
    /// port chosen for desktop mode).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The base URL clients should hit.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Request a graceful shutdown without waiting for it.
    pub fn trigger_shutdown(&self) {
        self.shutdown.notify_one();
    }

    /// Wait for the server task to finish (after a shutdown trigger or signal).
    pub async fn join(self) -> Result<(), ServerError> {
        self.task
            .await
            .map_err(|e| ServerError::Io(std::io::Error::other(e)))
    }

    /// Trigger a graceful shutdown and wait for the server to stop.
    pub async fn shutdown(self) -> Result<(), ServerError> {
        self.shutdown.notify_one();
        self.join().await
    }
}

/// Build the Axum router for an assembled [`AppState`].
pub(crate) fn build_router(state: AppState, enable_cors: bool) -> Router {
    let api = Router::new()
        .route("/", get(api::get_meta))
        .route(
            "/dashboards",
            get(api::list_dashboards).post(api::create_dashboard),
        )
        .route(
            "/dashboards/{id}",
            get(api::get_dashboard)
                .put(api::update_dashboard)
                .delete(api::delete_dashboard),
        )
        .route("/schema", get(api::get_schema))
        .route("/panels/execute", post(api::execute_panel))
        .route("/options", post(api::resolve_options))
        .route(
            "/datasets",
            get(datasets::list_datasets).post(datasets::ingest),
        )
        .route(
            "/datasets/{name}",
            get(datasets::get_dataset).delete(datasets::delete_dataset),
        )
        .route("/datasets/{name}/rename", post(datasets::rename_dataset))
        .route("/assets", get(api::list_assets))
        .route(
            "/assets/{*path}",
            get(api::get_asset)
                .put(api::upload_asset)
                .delete(api::delete_asset),
        )
        .route("/export/dashboard/{id}", post(api::export_dashboard))
        .route("/import/dashboard", post(api::import_dashboard))
        .route("/events", get(events::ws_events))
        .route("/shutdown", post(api::shutdown))
        .route("/heartbeat", post(api::heartbeat))
        // Author-time uploads (ingest, assets) can be large — lift the 2 MB default.
        .layer(DefaultBodyLimit::max(1024 * 1024 * 1024));

    let mut router = Router::new()
        .route("/", get(static_assets::serve_index))
        .nest("/api/almagest", api)
        .fallback(static_assets::serve_asset)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    if enable_cors {
        router = router.layer(tower_http::cors::CorsLayer::permissive());
    }
    router
}

/// Open `path`, build the query engine, and start serving per `options`.
///
/// Returns once the listener is bound (so callers can read
/// [`ServerHandle::addr`]); the server runs on a spawned task until shutdown is
/// triggered or a Ctrl-C / SIGTERM arrives.
pub async fn start_server(
    path: &Path,
    options: ServerOptions,
) -> Result<ServerHandle, ServerError> {
    let file = AlmagestFile::open(path)?;
    let query = AlmagestQueryContext::open(&file)?;
    let state =
        AppState::new(file, query).with_flags(options.read_only, options.heartbeat_shutdown);
    let shutdown = state.shutdown.clone();

    // Desktop lifecycle: once the frontend's heartbeats stop (the last tab
    // closed), shut the process down. A generous startup grace covers the time
    // before the browser connects; after that, a stretch with no heartbeat means
    // nobody is watching.
    if options.heartbeat_shutdown {
        spawn_heartbeat_watchdog(state.clone());
    }

    let app = build_router(state, options.enable_cors);

    let listener = TcpListener::bind(options.bind_addr).await?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, "almagest server listening");

    let signal_shutdown = shutdown.clone();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        wait_for_shutdown(signal_shutdown).await;
        tracing::info!("almagest server shutting down");
    });

    let task = tokio::spawn(async move {
        if let Err(e) = server.await {
            tracing::error!(error = %e, "almagest server error");
        }
    });

    if options.open_browser {
        let url = format!("http://{addr}");
        if let Err(e) = open_browser(&url) {
            tracing::warn!(error = %e, %url, "could not open browser automatically");
        }
    }

    Ok(ServerHandle {
        addr,
        task,
        shutdown,
    })
}

/// Startup grace before the heartbeat watchdog starts enforcing — long enough
/// for the OS to launch the browser and the SPA to load and send its first ping.
const HEARTBEAT_GRACE: std::time::Duration = std::time::Duration::from_secs(30);
/// How stale the last heartbeat may get before we conclude the last tab closed.
const HEARTBEAT_IDLE: std::time::Duration = std::time::Duration::from_secs(12);
/// How often the watchdog checks heartbeat staleness.
const HEARTBEAT_POLL: std::time::Duration = std::time::Duration::from_secs(3);

/// Spawn the desktop heartbeat watchdog: after a startup grace, trigger shutdown
/// once the last heartbeat is older than [`HEARTBEAT_IDLE`] (every connected tab
/// pings on an interval, so a quiet stretch means none are left).
fn spawn_heartbeat_watchdog(state: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(HEARTBEAT_GRACE).await;
        let mut ticker = tokio::time::interval(HEARTBEAT_POLL);
        loop {
            ticker.tick().await;
            if state.heartbeat_age() > HEARTBEAT_IDLE {
                tracing::info!(
                    "no heartbeat in {:?}; last tab closed — shutting down",
                    HEARTBEAT_IDLE
                );
                state.shutdown.notify_one();
                break;
            }
        }
    });
}

/// Resolve when either the in-process shutdown is triggered or an OS signal
/// arrives (Ctrl-C everywhere, SIGTERM on Unix).
async fn wait_for_shutdown(notify: Arc<Notify>) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = notify.notified() => {}
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

/// Open the system default browser at `url` without pulling in a dependency:
/// the platform's standard opener (`open` / `xdg-open` / `cmd start`).
fn open_browser(url: &str) -> std::io::Result<()> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Build information for the running server.
pub fn server_version() -> &'static str {
    almagest_core::ALMAGEST_VERSION
}

// SPDX-License-Identifier: MIT OR Apache-2.0

//! # almagest-embed
//!
//! The embedding API: a clean builder for a host program to bundle Almagest
//! inside its own binary and mount a `.alm` dashboard at a URL it controls. This
//! is Almagest's highest-commercial-value deploy mode (per-product / per-seat
//! licensing) — see `planning/12-embedding-library.md`.
//!
//! It reuses the **same Axum server and Svelte bundle** as every other deploy
//! mode; there is no separate embedded frontend. The host owns the Tokio runtime
//! and process lifecycle: [`AlmagestServer`] runs as a spawned task, listens for
//! **no OS signals**, and exits only when the host calls
//! [`AlmagestServer::shutdown`] (or drops the process).
//!
//! ## Data boundary (unchanged when embedded)
//!
//! Embedding does not relax the embedded-only guarantee: Almagest queries only
//! data local to the `.alm`. If the host has remote data it must fetch it and
//! hand Almagest a local copy; Almagest never dials out at view time.
//!
//! ## Example
//!
//! ```no_run
//! # async fn run() -> Result<(), almagest_embed::EmbedError> {
//! use almagest_embed::AlmagestServer;
//!
//! let server = AlmagestServer::builder()
//!     .alm_file("./reports/default.alm")
//!     .bind_address("127.0.0.1:0") // ephemeral port
//!     .read_only(true)
//!     .auth_hook(|headers| {
//!         // Trust a header the host set behind its own auth/proxy.
//!         headers.get("x-host-user").is_some()
//!     })
//!     .start()
//!     .await?;
//!
//! println!("Almagest mounted at {}", server.url());
//! // ... point a webview / reverse proxy at server.url() ...
//! server.shutdown().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Logging
//!
//! Almagest logs via [`tracing`]. This crate **never installs a subscriber** —
//! the host's global subscriber receives Almagest's spans and events directly.
//! Install your subscriber before calling [`AlmagestServerBuilder::start`].

use almagest_server::{
    AuthHook, ServerEvent, ServerHandle, ServerOptions, start_server, start_server_with_auth,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

pub use almagest_server::{HeaderMap, ServerEvent as Event};

/// Errors raised while configuring or starting an embedded server.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// [`AlmagestServerBuilder::alm_file`] was never called.
    #[error("no .alm file was set on the builder (call .alm_file(path))")]
    NoFile,
    /// The bind address could not be parsed as `host:port`.
    #[error("invalid bind address {addr:?}: {source}")]
    BadBind {
        /// The offending address string.
        addr: String,
        /// The underlying parse error.
        source: std::net::AddrParseError,
    },
    /// Opening the file or binding/serving failed.
    #[error(transparent)]
    Server(#[from] almagest_server::ServerError),
}

/// A running embedded Almagest server, owned by the host. Reachable at
/// [`AlmagestServer::url`]; stopped with [`AlmagestServer::shutdown`].
pub struct AlmagestServer {
    handle: ServerHandle,
}

impl AlmagestServer {
    /// Start configuring a server.
    pub fn builder() -> AlmagestServerBuilder {
        AlmagestServerBuilder::default()
    }

    /// The base URL the server is listening on (resolves an ephemeral port).
    pub fn url(&self) -> String {
        self.handle.url()
    }

    /// The bound socket address.
    pub fn addr(&self) -> SocketAddr {
        self.handle.addr()
    }

    /// Subscribe to the server's lifecycle/data events for host observability or
    /// audit (a lagging/dropped receiver never blocks the server).
    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.handle.subscribe()
    }

    /// Request shutdown without waiting (e.g. from a signal handler the host owns).
    pub fn trigger_shutdown(&self) {
        self.handle.trigger_shutdown();
    }

    /// Trigger a graceful shutdown and wait for the server to stop.
    pub async fn shutdown(self) -> Result<(), EmbedError> {
        self.handle.shutdown().await.map_err(EmbedError::Server)
    }

    /// Wait for the server to stop (after the host triggers shutdown).
    pub async fn join(self) -> Result<(), EmbedError> {
        self.handle.join().await.map_err(EmbedError::Server)
    }
}

/// Builder for [`AlmagestServer`]. All settings are optional except the file.
#[derive(Default)]
pub struct AlmagestServerBuilder {
    file: Option<PathBuf>,
    bind: Option<String>,
    read_only: bool,
    cors: bool,
    open_browser: bool,
    auth: Option<AuthHook>,
}

impl AlmagestServerBuilder {
    /// The `.alm` file to open and serve (required).
    pub fn alm_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.file = Some(path.into());
        self
    }

    /// The HTTP bind address as `host:port` (default `127.0.0.1:0` — an ephemeral
    /// loopback port). Use `:0` to let the OS pick the port and read it back from
    /// [`AlmagestServer::url`].
    pub fn bind_address(mut self, addr: impl Into<String>) -> Self {
        self.bind = Some(addr.into());
        self
    }

    /// Serve read-only — every mutating endpoint returns `403` (default false).
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Apply a permissive CORS policy, for a host page on a different origin
    /// (default false — same-origin needs none).
    pub fn cors(mut self, cors: bool) -> Self {
        self.cors = cors;
        self
    }

    /// Auto-open the system browser once listening (default false for embedding).
    pub fn open_browser(mut self, open_browser: bool) -> Self {
        self.open_browser = open_browser;
        self
    }

    /// Gate every request through a host check on the request headers. Returning
    /// `false` rejects with `401`. The host validates a header/token it set on
    /// the way in (behind its own auth / reverse proxy).
    pub fn auth_hook<F>(mut self, hook: F) -> Self
    where
        F: Fn(&HeaderMap) -> bool + Send + Sync + 'static,
    {
        self.auth = Some(Arc::new(hook));
        self
    }

    /// Open the file, bind, and start serving on the host's Tokio runtime.
    /// Returns once the listener is bound.
    pub async fn start(self) -> Result<AlmagestServer, EmbedError> {
        let file = self.file.ok_or(EmbedError::NoFile)?;
        let bind = self.bind.unwrap_or_else(|| "127.0.0.1:0".to_string());
        let bind_addr: SocketAddr = bind.parse().map_err(|source| EmbedError::BadBind {
            addr: bind.clone(),
            source,
        })?;

        let options = ServerOptions {
            bind_addr,
            open_browser: self.open_browser,
            enable_cors: self.cors,
            read_only: self.read_only,
            listen_for_signals: false, // the host owns process lifecycle
            ..ServerOptions::default()
        };

        let handle = match self.auth {
            Some(auth) => start_server_with_auth(&file, options, auth).await?,
            None => start_server(&file, options).await?,
        };
        Ok(AlmagestServer { handle })
    }
}

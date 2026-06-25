// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared server state.
//!
//! [`AppState`] is cloned into every handler (all fields are `Arc`-shared). It
//! holds the open `.alm` file (behind a `Mutex` for the writing CRUD paths), the
//! in-memory DataFusion query context built from the file's embedded Parquet
//! blobs, a broadcast channel for WebSocket events, and a shutdown trigger.

use crate::auth::LoginThrottle;
use crate::error::{ApiError, ApiResult};
use almagest_core::{AlmagestFile, AuthConfig};
use almagest_query::AlmagestQueryContext;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, broadcast};

/// State shared across all request handlers. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    /// The open `.alm` file. A `Mutex` because rusqlite's `Connection` is `Send`
    /// but not `Sync`; the lock is only ever held for short, synchronous bursts
    /// (never across an `.await`).
    pub file: Arc<Mutex<AlmagestFile>>,
    /// The in-memory query engine over the file's embedded data, behind an
    /// `RwLock` so author-time data mutations (ingest / rename / delete) can swap
    /// in a freshly-built context. Handlers clone the inner `Arc` out under a
    /// short read lock, then run async work on the clone (never holding the lock
    /// across an `.await`).
    query: Arc<RwLock<Arc<AlmagestQueryContext>>>,
    /// Broadcast channel for real-time events to connected WebSocket clients.
    pub events: broadcast::Sender<ServerEvent>,
    /// Fired to request a graceful shutdown (desktop "close" / `/shutdown`).
    pub shutdown: Arc<Notify>,
    /// When true, every mutating endpoint is rejected with 403 — the file is
    /// served as a fixed deliverable (`--read-only`).
    read_only: bool,
    /// When true, the frontend pings `/heartbeat` and a watchdog shuts the server
    /// down once the pings stop (desktop "last tab closed" lifecycle).
    heartbeat_enabled: bool,
    /// Last time a heartbeat was observed; the watchdog compares against it.
    last_heartbeat: Arc<Mutex<Instant>>,
    /// Live auth state (enabled flag, session secret, lifetime) + login throttle,
    /// mirrored from the file's `almagest_auth` config (doc 13). Cached here so the
    /// per-request middleware never locks the file; refreshed via
    /// [`AppState::reload_auth`] whenever the config changes.
    pub auth: Arc<AuthRuntime>,
}

/// Cached auth configuration plus the in-memory login throttle. Mirrors the
/// file's `almagest_auth` row so middleware can check it lock-free on every
/// request; [`AppState::reload_auth`] re-syncs it after a config change.
pub struct AuthRuntime {
    enabled: AtomicBool,
    lifetime_secs: AtomicI64,
    secret: RwLock<Option<Vec<u8>>>,
    /// Per-username failed-login throttle / lockout.
    pub throttle: LoginThrottle,
}

impl AuthRuntime {
    fn from_config(cfg: AuthConfig) -> Self {
        Self {
            enabled: AtomicBool::new(cfg.enabled),
            lifetime_secs: AtomicI64::new(cfg.session_lifetime_secs),
            secret: RwLock::new(cfg.session_secret),
            throttle: LoginThrottle::default(),
        }
    }

    /// Whether auth is enforced.
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Session lifetime in seconds.
    pub fn lifetime_secs(&self) -> i64 {
        self.lifetime_secs.load(Ordering::Relaxed)
    }

    /// A clone of the current session-signing secret, if set.
    pub fn secret(&self) -> Option<Vec<u8>> {
        self.secret.read().expect("auth secret rwlock").clone()
    }

    /// Re-sync from a freshly read [`AuthConfig`] (after enable/disable/secret
    /// change). The throttle state is preserved across reloads.
    fn apply(&self, cfg: AuthConfig) {
        self.enabled.store(cfg.enabled, Ordering::Relaxed);
        self.lifetime_secs
            .store(cfg.session_lifetime_secs, Ordering::Relaxed);
        *self.secret.write().expect("auth secret rwlock") = cfg.session_secret;
    }
}

impl AppState {
    /// Assemble state from an open file and its query context. Read-write, no
    /// heartbeat watchdog by default; [`AppState::with_flags`] opts into either.
    pub fn new(file: AlmagestFile, query: AlmagestQueryContext) -> Self {
        let (events, _) = broadcast::channel(64);
        // A freshly migrated file always has the almagest_auth row; fall back to a
        // disabled default if it somehow can't be read so the server still starts.
        let auth_cfg = file.auth_config().unwrap_or(AuthConfig {
            enabled: false,
            session_secret: None,
            session_lifetime_secs: 86400,
        });
        Self {
            file: Arc::new(Mutex::new(file)),
            query: Arc::new(RwLock::new(Arc::new(query))),
            events,
            shutdown: Arc::new(Notify::new()),
            read_only: false,
            heartbeat_enabled: false,
            last_heartbeat: Arc::new(Mutex::new(Instant::now())),
            auth: Arc::new(AuthRuntime::from_config(auth_cfg)),
        }
    }

    /// Re-read the file's auth config into the cached [`AuthRuntime`] after a
    /// change (enable, disable, secret rotation). Cheap; call under no lock.
    pub fn reload_auth(&self) -> ApiResult<()> {
        let cfg = self.file().auth_config()?;
        self.auth.apply(cfg);
        Ok(())
    }

    /// Set the deploy-mode flags (read-only file, heartbeat lifecycle).
    pub fn with_flags(mut self, read_only: bool, heartbeat_enabled: bool) -> Self {
        self.read_only = read_only;
        self.heartbeat_enabled = heartbeat_enabled;
        self
    }

    /// Whether the file is served read-only.
    pub fn read_only(&self) -> bool {
        self.read_only
    }

    /// Whether the heartbeat lifecycle is active (exposed to the frontend so it
    /// only pings in desktop mode).
    pub fn heartbeat_enabled(&self) -> bool {
        self.heartbeat_enabled
    }

    /// Reject a mutating request when the file is read-only.
    pub fn ensure_writable(&self) -> ApiResult<()> {
        if self.read_only {
            Err(ApiError::forbidden(
                "this Almagest file is served read-only",
            ))
        } else {
            Ok(())
        }
    }

    /// Record a heartbeat from a connected client.
    pub fn touch_heartbeat(&self) {
        *self
            .last_heartbeat
            .lock()
            .expect("heartbeat mutex poisoned") = Instant::now();
    }

    /// How long since the last observed heartbeat.
    pub fn heartbeat_age(&self) -> Duration {
        self.last_heartbeat
            .lock()
            .expect("heartbeat mutex poisoned")
            .elapsed()
    }

    /// Lock the file. Callers must not hold the guard across an `.await`.
    pub fn file(&self) -> std::sync::MutexGuard<'_, AlmagestFile> {
        self.file.lock().expect("almagest file mutex poisoned")
    }

    /// Snapshot the current query context. Cheap `Arc` clone; the returned handle
    /// stays valid for an async query even if the context is later swapped.
    pub fn query(&self) -> Arc<AlmagestQueryContext> {
        self.query.read().expect("query rwlock poisoned").clone()
    }

    /// Rebuild the query context from the (mutated) file and swap it in, so newly
    /// ingested / renamed / removed datasets are registered for querying.
    pub fn rebuild_query(&self, file: &AlmagestFile) -> almagest_query::Result<()> {
        let rebuilt = AlmagestQueryContext::open(file)?;
        *self.query.write().expect("query rwlock poisoned") = Arc::new(rebuilt);
        Ok(())
    }

    /// Best-effort broadcast of an event (a send with no receivers is fine).
    pub fn emit(&self, event: ServerEvent) {
        let _ = self.events.send(event);
    }
}

/// An event pushed to connected clients over the WebSocket. Used to keep
/// multiple tabs viewing the same file consistent (doc 08).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerEvent {
    /// A dashboard's definition changed.
    DashboardUpdated {
        /// The affected dashboard id.
        dashboard_id: String,
    },
    /// A dashboard was deleted.
    DashboardDeleted {
        /// The removed dashboard id.
        dashboard_id: String,
    },
    /// The query result cache was invalidated.
    CacheInvalidated,
    /// A dataset was added, replaced, renamed, or removed — clients should
    /// refresh the schema and re-run panels.
    DataChanged,
    /// An embedded asset was uploaded or removed.
    AssetChanged {
        /// The affected asset path.
        path: String,
    },
    /// The file's auth configuration changed (enabled/disabled). Embedding hosts
    /// can observe this; the SPA re-checks `/auth/me`.
    AuthChanged,
    /// A heartbeat to keep the connection alive and let clients detect drop.
    Heartbeat,
}

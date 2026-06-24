// SPDX-License-Identifier: MIT OR Apache-2.0

//! The `almagest` command-line interface.
//!
//! `new` creates an empty `.alm`; `open` and `serve` start the HTTP server
//! (Phase 08) in desktop (ephemeral port, auto-open browser, heartbeat-managed
//! lifecycle) and headless (fixed port, long-lived) modes respectively. `export`
//! (Phase 11) bakes a self-contained static HTML snapshot.

use almagest_core::AlmagestFile;
use almagest_server::{ServerOptions, start_server};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

mod export;

/// Almagest — dashboards as files, not services.
#[derive(Debug, Parser)]
#[command(name = "almagest", version, about, long_about = None)]
struct Cli {
    /// Increase log verbosity (-v, -vv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new, empty `.alm` file.
    New {
        /// Path to the `.alm` file to create.
        path: PathBuf,
    },
    /// Open a `.alm` file in the local browser (desktop mode).
    Open {
        /// Path to the `.alm` file to open.
        path: PathBuf,
        /// Fix the port (default: an ephemeral one).
        #[arg(long)]
        port: Option<u16>,
        /// Start the server without opening a browser.
        #[arg(long)]
        no_open: bool,
        /// Serve the file read-only (no edits, ingest, or uploads).
        #[arg(long)]
        read_only: bool,
    },
    /// Serve a `.alm` file over HTTP (headless mode).
    Serve {
        /// Path to the `.alm` file to serve.
        path: PathBuf,
        /// Port to listen on.
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Host/interface to bind (default loopback; `0.0.0.0` to expose).
        #[arg(long, default_value = "127.0.0.1")]
        host: std::net::IpAddr,
        /// Serve the file read-only (no edits, ingest, or uploads).
        #[arg(long)]
        read_only: bool,
    },
    /// Export a `.alm` dashboard as a self-contained static HTML snapshot.
    Export {
        /// Path to the `.alm` file to export.
        path: PathBuf,
        /// Output `.html` path (default: `<dashboard>-snapshot.html`).
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Which dashboard to export (default: the only/first one).
        #[arg(long)]
        dashboard: Option<String>,
        /// Output format. Only `html` is supported today (`pdf` is planned).
        #[arg(long, default_value = "html")]
        format: String,
        /// Parameter values as a JSON object, e.g. `{"region":"EU"}`.
        #[arg(long)]
        parameters: Option<String>,
    },
}

fn init_tracing(verbose: u8) {
    let default = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        None => {
            // No subcommand: print a short banner pointing at --help.
            println!("almagest {}", almagest_core::ALMAGEST_VERSION);
            println!("Dashboards as files, not services. Try `almagest --help`.");
            Ok(())
        }
        Some(Command::New { path }) => cmd_new(&path),
        Some(Command::Open {
            path,
            port,
            no_open,
            read_only,
        }) => {
            let mut opts = ServerOptions::desktop()
                .with_open_browser(!no_open)
                .with_read_only(read_only);
            if let Some(port) = port {
                opts = opts.with_bind(SocketAddr::from(([127, 0, 0, 1], port)));
            }
            cmd_serve(&path, opts).await
        }
        Some(Command::Serve {
            path,
            port,
            host,
            read_only,
        }) => {
            let opts = ServerOptions::headless(port)
                .with_bind(SocketAddr::new(host, port))
                .with_read_only(read_only);
            cmd_serve(&path, opts).await
        }
        Some(Command::Export {
            path,
            output,
            dashboard,
            format,
            parameters,
        }) => {
            export::run(
                &path,
                output.as_deref(),
                dashboard.as_deref(),
                &format,
                parameters.as_deref(),
            )
            .await
        }
    }
}

/// Create a new, empty `.alm` file.
fn cmd_new(path: &std::path::Path) -> Result<()> {
    let file =
        AlmagestFile::create(path).with_context(|| format!("creating {}", path.display()))?;
    file.close().context("finalizing the new file")?;
    println!("Created {}", path.display());
    Ok(())
}

/// Start the server for `path` and run until shutdown (Ctrl-C / SIGTERM, or the
/// frontend's shutdown call in desktop mode).
async fn cmd_serve(path: &std::path::Path, options: ServerOptions) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("{} does not exist", path.display());
    }
    let handle = start_server(path, options)
        .await
        .with_context(|| format!("starting server for {}", path.display()))?;
    println!("Almagest serving {} at {}", path.display(), handle.url());
    println!("Press Ctrl-C to stop.");
    handle.join().await.context("server task failed")?;
    println!("Server stopped.");
    Ok(())
}

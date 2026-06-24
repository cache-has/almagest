// SPDX-License-Identifier: MIT OR Apache-2.0

//! The `almagest` command-line interface.
//!
//! `new` creates an empty `.alm`; `open` and `serve` start the HTTP server
//! (Phase 08) in desktop (ephemeral port, auto-open browser) and headless
//! (fixed port, long-lived) modes respectively. `export` is fleshed out in
//! Phase 11.

use almagest_core::AlmagestFile;
use almagest_server::{ServerOptions, start_server};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    },
    /// Serve a `.alm` file over HTTP (headless mode).
    Serve {
        /// Path to the `.alm` file to serve.
        path: PathBuf,
        /// Port to listen on.
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
    /// Export a `.alm` dashboard as a static HTML snapshot.
    Export {
        /// Path to the `.alm` file to export.
        path: PathBuf,
        /// Output directory for the static bundle.
        #[arg(long, default_value = "./static")]
        output: PathBuf,
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
        Some(Command::Open { path }) => cmd_serve(&path, ServerOptions::desktop()).await,
        Some(Command::Serve { path, port }) => {
            cmd_serve(&path, ServerOptions::headless(port)).await
        }
        Some(Command::Export { path, output }) => {
            tracing::info!(?path, ?output, "export is not yet implemented");
            anyhow::bail!("`almagest export` is not yet implemented (Phase 11)")
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

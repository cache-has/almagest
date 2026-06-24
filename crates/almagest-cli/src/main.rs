// SPDX-License-Identifier: MIT OR Apache-2.0

//! The `almagest` command-line interface.
//!
//! Phase 01 wires up the command surface and a working `--version`. The
//! subcommands are stubbed and fleshed out in later phases:
//! `new`/`open`/`serve`/`export` map to the format, server, and deployment work.

use anyhow::Result;
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        None => {
            // No subcommand: print a short banner pointing at --help.
            println!("almagest {}", almagest_core::ALMAGEST_VERSION);
            println!("Dashboards as files, not services. Try `almagest --help`.");
            Ok(())
        }
        Some(Command::New { path }) => not_yet("new", &path),
        Some(Command::Open { path }) => not_yet("open", &path),
        Some(Command::Serve { path, port }) => {
            tracing::info!(?path, port, "serve is not yet implemented");
            anyhow::bail!("`almagest serve` is not yet implemented (Phase 08)")
        }
        Some(Command::Export { path, output }) => {
            tracing::info!(?path, ?output, "export is not yet implemented");
            anyhow::bail!("`almagest export` is not yet implemented (Phase 11)")
        }
    }
}

fn not_yet(cmd: &str, path: &std::path::Path) -> Result<()> {
    tracing::info!(?path, "command `{cmd}` is not yet implemented");
    anyhow::bail!("`almagest {cmd}` is not yet implemented")
}

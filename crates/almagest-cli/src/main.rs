// SPDX-License-Identifier: MIT OR Apache-2.0

//! The `almagest` command-line interface (doc 14).
//!
//! Primary commands: `new` (optionally `--from-template`), `open` (desktop
//! browser-serve), `serve` (headless), and `export` (static HTML snapshot).
//! Management: `info`, `validate`, `dashboard`, `user`, `import`, `doctor`.
//! `almagest <file.alm>` with no subcommand is shorthand for `almagest open`.
//! Every read command supports `--json` for scripting; `--quiet` trims chatter.

use almagest_server::{ServerOptions, start_server};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

mod dashboard;
mod doctor;
mod export;
mod import;
mod info;
mod output;
mod template;
mod user;
mod validate;

use output::Out;

/// Almagest — dashboards as files, not services.
#[derive(Debug, Parser)]
#[command(name = "almagest", version, about, long_about = None)]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    /// Increase log verbosity (-v, -vv). Logs go to stderr.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    /// Emit machine-readable JSON (where a command supports it).
    #[arg(long, global = true)]
    json: bool,
    /// Suppress non-essential output.
    #[arg(short, long, global = true)]
    quiet: bool,

    /// A `.alm` file to open — shorthand for `almagest open <file>`.
    file: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new `.alm` file (optionally from a template).
    New {
        /// Path to the `.alm` file to create.
        path: PathBuf,
        /// Seed from a bundled template (`blank`, `sales`).
        #[arg(long)]
        from_template: Option<String>,
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
        /// Output format. Only `html` is supported today (`pdf`/`png` planned).
        #[arg(long, default_value = "html")]
        format: String,
        /// Parameter values as a JSON object, e.g. `{"region":"North"}`.
        #[arg(long)]
        parameters: Option<String>,
    },
    /// Import a standalone dashboard JSON into a `.alm` file.
    Import {
        /// The `.alm` file to import into.
        path: PathBuf,
        /// The dashboard `.json` file to import.
        dashboard: PathBuf,
        /// Optional organizational folder.
        #[arg(long)]
        folder: Option<String>,
    },
    /// Show a summary of a `.alm` file.
    Info {
        /// Path to the `.alm` file.
        path: PathBuf,
    },
    /// Validate a `.alm` file's integrity and schema (exit non-zero on failure).
    Validate {
        /// Path to the `.alm` file.
        path: PathBuf,
        /// Also decode every dataset and build the query engine.
        #[arg(long)]
        strict: bool,
    },
    /// Manage dashboards in a `.alm` file.
    Dashboard {
        #[command(subcommand)]
        cmd: DashboardCmd,
    },
    /// Manage user accounts (auth-enabled files).
    User {
        #[command(subcommand)]
        cmd: UserCmd,
    },
    /// Diagnose the environment.
    Doctor,
}

#[derive(Debug, Subcommand)]
enum DashboardCmd {
    /// List dashboards.
    List {
        /// Path to the `.alm` file.
        path: PathBuf,
    },
    /// Print a dashboard's definition JSON.
    Show {
        /// Path to the `.alm` file.
        path: PathBuf,
        /// Dashboard id or (unique) name.
        dashboard: String,
    },
    /// Delete a dashboard.
    Delete {
        /// Path to the `.alm` file.
        path: PathBuf,
        /// Dashboard id or (unique) name.
        dashboard: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum UserCmd {
    /// List user accounts.
    List {
        /// Path to the `.alm` file.
        path: PathBuf,
    },
    /// Add a user (the first one also enables auth on the file).
    Add {
        /// Path to the `.alm` file.
        path: PathBuf,
        /// Login name.
        #[arg(long)]
        username: String,
        /// Role: `admin`, `editor`, or `viewer`.
        #[arg(long, default_value = "viewer")]
        role: String,
        /// Optional email.
        #[arg(long)]
        email: Option<String>,
        /// Password (else `$ALMAGEST_PASSWORD`, else an interactive prompt).
        #[arg(long)]
        password: Option<String>,
    },
    /// Reset a user's password to a generated temporary one.
    ResetPassword {
        /// Path to the `.alm` file.
        path: PathBuf,
        /// The user to reset.
        #[arg(long)]
        username: String,
    },
    /// Remove a user.
    Remove {
        /// Path to the `.alm` file.
        path: PathBuf,
        /// The user to remove.
        #[arg(long)]
        username: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

fn init_tracing(verbose: u8, terse: bool) {
    // One-shot commands (and --json/--quiet) keep logs to errors so stdout stays
    // clean for piping; long-running serve/open default to info.
    let default = if terse {
        "error"
    } else {
        match verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let out = Out {
        json: cli.json,
        quiet: cli.quiet,
    };
    // Serve/open want their "listening" line; everything else stays terse.
    let long_running = matches!(
        cli.command,
        Some(Command::Open { .. }) | Some(Command::Serve { .. })
    ) || (cli.command.is_none() && cli.file.is_some());
    init_tracing(cli.verbose, cli.quiet || cli.json || !long_running);

    match (cli.command, cli.file) {
        // `almagest <file.alm>` → open with desktop defaults.
        (None, Some(file)) => cmd_serve(&file, ServerOptions::desktop()).await,
        (None, None) => {
            println!("almagest {}", almagest_core::ALMAGEST_VERSION);
            println!("Dashboards as files, not services. Try `almagest --help`.");
            Ok(())
        }
        (Some(command), _) => dispatch(command, &out).await,
    }
}

async fn dispatch(command: Command, out: &Out) -> Result<()> {
    match command {
        Command::New {
            path,
            from_template,
        } => template::create(&path, from_template.as_deref(), out),
        Command::Open {
            path,
            port,
            no_open,
            read_only,
        } => {
            let mut opts = ServerOptions::desktop()
                .with_open_browser(!no_open)
                .with_read_only(read_only);
            if let Some(port) = port {
                opts = opts.with_bind(SocketAddr::from(([127, 0, 0, 1], port)));
            }
            cmd_serve(&path, opts).await
        }
        Command::Serve {
            path,
            port,
            host,
            read_only,
        } => {
            let opts = ServerOptions::headless(port)
                .with_bind(SocketAddr::new(host, port))
                .with_read_only(read_only);
            cmd_serve(&path, opts).await
        }
        Command::Export {
            path,
            output,
            dashboard,
            format,
            parameters,
        } => {
            export::run(
                &path,
                output.as_deref(),
                dashboard.as_deref(),
                &format,
                parameters.as_deref(),
            )
            .await
        }
        Command::Import {
            path,
            dashboard,
            folder,
        } => import::run(&path, &dashboard, folder.as_deref(), out),
        Command::Info { path } => info::run(&path, out),
        Command::Validate { path, strict } => {
            let valid = validate::run(&path, strict, out)?;
            if !valid {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Dashboard { cmd } => match cmd {
            DashboardCmd::List { path } => dashboard::list(&path, out),
            DashboardCmd::Show { path, dashboard } => dashboard::show(&path, &dashboard, out),
            DashboardCmd::Delete {
                path,
                dashboard,
                yes,
            } => dashboard::delete(&path, &dashboard, yes, out),
        },
        Command::User { cmd } => match cmd {
            UserCmd::List { path } => user::list(&path, out),
            UserCmd::Add {
                path,
                username,
                role,
                email,
                password,
            } => user::add(
                &path,
                &username,
                &role,
                email.as_deref(),
                password.as_deref(),
                out,
            ),
            UserCmd::ResetPassword { path, username } => {
                user::reset_password(&path, &username, out)
            }
            UserCmd::Remove {
                path,
                username,
                yes,
            } => user::remove(&path, &username, yes, out),
        },
        Command::Doctor => {
            let ok = doctor::run(out)?;
            if !ok {
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

/// Start the server for `path` and run until shutdown (Ctrl-C / SIGTERM, or the
/// frontend's shutdown call in desktop mode).
async fn cmd_serve(path: &Path, options: ServerOptions) -> Result<()> {
    if !path.exists() {
        bail!("{} does not exist", path.display());
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

/// Prompt for a yes/no confirmation on stderr; error out unless the answer is
/// affirmative. Used by destructive commands (bypass with `--yes`).
pub fn confirm(question: &str) -> Result<()> {
    use std::io::{BufRead, Write};
    eprint!("{question} [y/N] ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let ans = line.trim().to_ascii_lowercase();
    if ans == "y" || ans == "yes" {
        Ok(())
    } else {
        bail!("aborted");
    }
}

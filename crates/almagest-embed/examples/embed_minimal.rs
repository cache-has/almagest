// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Minimal embedding example: a host program opens a `.alm` and serves it on an
// ephemeral loopback port, read-only, then waits for Ctrl-C. The embedded server
// does not listen for signals itself — the host owns lifecycle — so this example
// handles Ctrl-C and asks the server to shut down cleanly.
//
//   cargo run -p almagest-embed --example embed_minimal -- ./my.alm

use almagest_embed::AlmagestServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The host installs its own tracing subscriber; Almagest logs flow into it.
    tracing_subscriber::fmt::init();

    let path = std::env::args()
        .nth(1)
        .ok_or("usage: embed_minimal <file.alm>")?;

    let server = AlmagestServer::builder()
        .alm_file(path)
        .bind_address("127.0.0.1:0")
        .read_only(true)
        .auth_hook(|headers| {
            // Toy gate: require a header the host would set behind its own auth.
            // Remove this to serve openly on loopback.
            headers.get("x-host-user").is_some()
        })
        .start()
        .await?;

    println!("Almagest mounted at {}", server.url());
    println!("(requests need an `x-host-user` header in this example)");
    println!("Press Ctrl-C to stop.");

    tokio::signal::ctrl_c().await?;
    server.shutdown().await?;
    Ok(())
}

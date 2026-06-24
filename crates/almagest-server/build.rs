//! Ensure `frontend/dist/` exists at compile time.
//!
//! The server embeds the built Svelte bundle from `frontend/dist` via
//! `rust-embed`, which requires the folder to exist when the crate compiles.
//! The real bundle lands in Phase 09/10; until then (and on a fresh clone,
//! where `frontend/dist` is git-ignored) this writes a placeholder `index.html`
//! so the crate always builds and `almagest serve` renders *something*. A real
//! frontend build overwrites the placeholder.

use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest_dir.join("../../frontend/dist");
    let index = dist.join("index.html");

    if !index.exists() {
        fs::create_dir_all(&dist).expect("create frontend/dist");
        fs::write(&index, PLACEHOLDER_INDEX).expect("write placeholder index.html");
    }

    // Only re-run when the dist directory changes, not on every build.
    println!("cargo:rerun-if-changed=../../frontend/dist");
}

const PLACEHOLDER_INDEX: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Almagest</title>
  <style>
    body { font-family: system-ui, sans-serif; margin: 0; display: grid;
           place-items: center; min-height: 100vh; background: #0f1115; color: #e6e6e6; }
    main { text-align: center; max-width: 32rem; padding: 2rem; }
    h1 { font-weight: 600; letter-spacing: -0.02em; }
    code { background: #1c1f26; padding: 0.15em 0.4em; border-radius: 4px; }
    a { color: #7aa2f7; }
  </style>
</head>
<body>
  <main>
    <h1>Almagest</h1>
    <p>The server is running and the JSON API is live under <code>/api/almagest</code>.</p>
    <p>The dashboard frontend bundle hasn't been built yet — it ships in a later
       phase. Until then, query the API directly (e.g.
       <code>GET /api/almagest</code>).</p>
  </main>
</body>
</html>
"#;

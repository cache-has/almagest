// SPDX-License-Identifier: MIT OR Apache-2.0

//! `almagest doctor` — environment diagnostics with actionable hints.

use crate::output::Out;
use almagest_core::AlmagestFile;
use almagest_query::AlmagestQueryContext;
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
struct Check {
    name: String,
    ok: bool,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

#[derive(Serialize)]
struct DoctorReport {
    version: String,
    os: String,
    arch: String,
    checks: Vec<Check>,
    ok: bool,
}

/// Run `almagest doctor`. Returns `Ok(false)` if any check failed.
pub fn run(out: &Out) -> Result<bool> {
    let mut checks = Vec::new();

    // Frontend bundle baked into the binary.
    let assets = almagest_server::frontend_asset_count();
    checks.push(Check {
        name: "frontend assets".into(),
        ok: assets > 0,
        detail: format!("{assets} embedded files"),
        hint: (assets == 0)
            .then(|| "build the frontend (`just frontend`) before compiling the binary".into()),
    });

    // SQLite (the .alm container).
    checks.push(Check {
        name: "sqlite".into(),
        ok: true,
        detail: format!("version {}", almagest_core::sqlite_version()),
        hint: None,
    });

    // DataFusion engine — build a context over a throwaway empty file.
    let engine = engine_self_check();
    checks.push(Check {
        name: "query engine".into(),
        ok: engine.is_ok(),
        detail: match &engine {
            Ok(()) => "DataFusion initializes".into(),
            Err(e) => format!("failed to initialize: {e}"),
        },
        hint: None,
    });

    let all_ok = checks.iter().all(|c| c.ok);
    let report = DoctorReport {
        version: almagest_core::ALMAGEST_VERSION.to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        checks,
        ok: all_ok,
    };

    if out.json {
        out.emit(&report)?;
        return Ok(all_ok);
    }

    println!("Almagest version: {}", report.version);
    println!("Platform: {} {}", report.os, report.arch);
    println!();
    println!("Checks:");
    for c in &report.checks {
        let mark = if c.ok { "✓" } else { "✗" };
        println!("  {mark} {}: {}", c.name, c.detail);
        if let Some(h) = &c.hint {
            println!("    Hint: {h}");
        }
    }
    Ok(all_ok)
}

/// Create a throwaway `.alm` and build the query engine over it.
fn engine_self_check() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("almagest-doctor-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("doctor.alm");
    let _ = std::fs::remove_file(&path);
    let file = AlmagestFile::create(&path)?;
    let result = AlmagestQueryContext::open(&file).map(|_| ());
    let _ = file.close();
    let _ = std::fs::remove_dir_all(&dir);
    result.map_err(Into::into)
}

// SPDX-License-Identifier: MIT OR Apache-2.0

//! Output context shared by every command.
//!
//! `--json` switches a command to structured output (for scripting); `--quiet`
//! suppresses incidental human lines (errors still surface via the process exit
//! code and stderr). Commands branch on [`Out::json`] to emit either a
//! human-readable summary or a JSON document.

use anyhow::Result;
use serde::Serialize;

/// How a command should present its output.
#[derive(Debug, Clone, Copy)]
pub struct Out {
    /// Emit machine-readable JSON instead of human text.
    pub json: bool,
    /// Suppress non-essential human lines.
    pub quiet: bool,
}

impl Out {
    /// Print a human line unless `--quiet` (no-op in `--json` mode).
    pub fn line(&self, msg: impl std::fmt::Display) {
        if !self.quiet && !self.json {
            println!("{msg}");
        }
    }

    /// Print a human line even when quiet (but never in JSON mode) — for the
    /// primary result of a command (e.g. "Created …").
    pub fn result(&self, msg: impl std::fmt::Display) {
        if !self.json {
            println!("{msg}");
        }
    }

    /// Emit a value as pretty JSON (only in `--json` mode).
    pub fn emit<T: Serialize>(&self, value: &T) -> Result<()> {
        if self.json {
            println!("{}", serde_json::to_string_pretty(value)?);
        }
        Ok(())
    }
}

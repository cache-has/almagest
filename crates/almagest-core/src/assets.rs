// SPDX-License-Identifier: MIT OR Apache-2.0

//! Embedded presentation assets — images, custom CSS, fonts — stored in
//! `almagest_assets` and addressed by a logical path (e.g. `logo.png`).
//!
//! This is strictly for presentation bytes; row data never goes here (it lives
//! in `almagest_data` as Parquet blobs — see [`crate::data`]).

use crate::AlmagestFile;
use crate::error::{AlmagestError, Result};

/// A stored asset: its logical path, content type, and bytes.
#[derive(Debug, Clone)]
pub struct Asset {
    /// Logical path used to address the asset, unique within the file.
    pub path: String,
    /// MIME content type (e.g. `image/png`).
    pub content_type: String,
    /// Raw asset bytes.
    pub content: Vec<u8>,
}

impl AlmagestFile {
    /// Store (or replace) an asset at `path`. If `content_type` is `None` it is
    /// guessed from the path's extension, falling back to
    /// `application/octet-stream`.
    pub fn put_asset(
        &mut self,
        path: &str,
        content: &[u8],
        content_type: Option<&str>,
    ) -> Result<()> {
        if path.trim().is_empty() {
            return Err(AlmagestError::Invalid(
                "asset path must not be empty".into(),
            ));
        }
        let ctype = content_type
            .map(str::to_string)
            .unwrap_or_else(|| guess_content_type(path).to_string());
        let now = crate::now_rfc3339();
        // `path` is the conflict key, so on replacement the id is left untouched;
        // a fresh id only matters on first insert.
        let id = uuid::Uuid::new_v4().to_string();
        self.conn().execute(
            "INSERT INTO almagest_assets (id, path, content_type, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
                content_type = excluded.content_type,
                content = excluded.content",
            rusqlite::params![id, path, ctype, content, now],
        )?;
        Ok(())
    }

    /// Retrieve an asset by logical path.
    pub fn asset(&self, path: &str) -> Result<Asset> {
        self.conn()
            .query_row(
                "SELECT path, content_type, content FROM almagest_assets WHERE path = ?1",
                [path],
                |r| {
                    Ok(Asset {
                        path: r.get(0)?,
                        content_type: r.get(1)?,
                        content: r.get(2)?,
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => AlmagestError::NotFound {
                    kind: "asset",
                    id: path.to_string(),
                },
                other => AlmagestError::Sqlite(other),
            })
    }

    /// List stored asset paths (and content types), ordered by path.
    pub fn list_assets(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT path, content_type FROM almagest_assets ORDER BY path")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Remove an asset by path. Returns `true` if one was removed.
    pub fn remove_asset(&mut self, path: &str) -> Result<bool> {
        let n = self
            .conn()
            .execute("DELETE FROM almagest_assets WHERE path = ?1", [path])?;
        Ok(n > 0)
    }
}

/// Guess a content type from a file extension. Small built-in table covering the
/// asset kinds Almagest actually stores; anything else is octet-stream.
fn guess_content_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "css" => "text/css",
        "js" => "text/javascript",
        "json" => "application/json",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "html" | "htm" => "text/html",
        _ => "application/octet-stream",
    }
}

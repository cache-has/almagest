// SPDX-License-Identifier: MIT OR Apache-2.0

//! Typed access to `almagest_metadata` — the small key/value table holding the
//! file's identity and descriptive fields.

use crate::AlmagestFile;
use crate::error::{AlmagestError, Result};

impl AlmagestFile {
    /// Read a raw metadata value by key, if present.
    pub fn metadata(&self, key: &str) -> Result<Option<String>> {
        let v = self
            .conn()
            .query_row(
                "SELECT value FROM almagest_metadata WHERE key = ?1",
                [key],
                |r| r.get::<_, String>(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        Ok(v)
    }

    /// Set (insert or replace) a raw metadata value.
    pub fn set_metadata(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO almagest_metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// The format version recorded in the file.
    pub fn format_version(&self) -> Result<u32> {
        let raw = self.metadata("format_version")?.ok_or_else(|| {
            AlmagestError::Integrity("almagest_metadata is missing format_version".to_string())
        })?;
        raw.parse::<u32>().map_err(|_| {
            AlmagestError::Integrity(format!("format_version '{raw}' is not an integer"))
        })
    }

    /// The stable per-file identity (UUID) assigned at creation.
    pub fn almagest_id(&self) -> Result<String> {
        self.metadata("almagest_id")?.ok_or_else(|| {
            AlmagestError::Integrity("almagest_metadata is missing almagest_id".to_string())
        })
    }

    /// The human title (may be empty).
    pub fn title(&self) -> Result<String> {
        Ok(self.metadata("title")?.unwrap_or_default())
    }

    /// Set the human title.
    pub fn set_title(&mut self, title: &str) -> Result<()> {
        self.set_metadata("title", title)
    }

    /// The human description (may be empty).
    pub fn description(&self) -> Result<String> {
        Ok(self.metadata("description")?.unwrap_or_default())
    }

    /// Set the human description.
    pub fn set_description(&mut self, description: &str) -> Result<()> {
        self.set_metadata("description", description)
    }
}

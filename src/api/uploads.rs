// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::DiscourseClient;
use super::error::http_error;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Distilled fields from `/uploads.json` — the response carries more (id,
/// width/height, dominant_color, etc.) but these are the ones every caller
/// will reach for.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UploadInfo {
    pub id: u64,
    pub url: String,
    #[serde(default)]
    pub short_url: Option<String>,
    #[serde(default)]
    pub short_path: Option<String>,
    pub original_filename: String,
    pub filesize: u64,
    #[serde(default)]
    pub width: Option<u64>,
    #[serde(default)]
    pub height: Option<u64>,
}

impl DiscourseClient {
    /// Upload a file. `upload_type` is Discourse's `type` field — typical
    /// values: `composer` (default; for embedding in posts), `avatar`,
    /// `profile_background`, `card_background`, `custom_emoji`.
    pub fn upload_file(&self, file_path: &Path, upload_type: &str) -> Result<UploadInfo> {
        let make_form = || -> Result<reqwest::blocking::multipart::Form> {
            let bytes = std::fs::read(file_path)
                .with_context(|| format!("reading {}", file_path.display()))?;
            let filename = file_path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow!("upload path missing filename: {}", file_path.display()))?
                .to_string();
            let part = reqwest::blocking::multipart::Part::bytes(bytes).file_name(filename);
            Ok(reqwest::blocking::multipart::Form::new()
                .part("file", part)
                .text("type", upload_type.to_string())
                .text("synchronous", "true".to_string()))
        };

        let response =
            self.send_retrying(|| Ok(self.post("/uploads.json")?.multipart(make_form()?)))?;
        let status = response.status();
        let text = response.text().context("reading upload response body")?;
        if !status.is_success() {
            return Err(http_error("upload request", status, &text));
        }
        let info: UploadInfo = serde_json::from_str(&text).context("parsing upload response")?;
        Ok(info)
    }
}

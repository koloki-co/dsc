// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::{Context, Result};
use serde_json::Value;

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;

impl DiscourseClient {
    /// List installed plugins on the Discourse instance.
    pub fn list_plugins(&self) -> Result<Value> {
        let response = self.get("/admin/plugins.json")?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading plugins response body")?;
        if !status.is_success() {
            return Err(http_error("plugins request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing plugins response")?;
        Ok(value)
    }
}

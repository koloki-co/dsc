// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One row from /admin/api/keys.json.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ApiKeySummary {
    pub id: u64,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, alias = "user_username")]
    pub username: Option<String>,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub revoked_at: Option<String>,
    #[serde(default)]
    pub truncated_key: Option<String>,
}

/// Response from POST /admin/api/keys.json — includes the full secret `key`.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CreatedApiKey {
    pub id: u64,
    pub key: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, alias = "user_username")]
    pub username: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

impl DiscourseClient {
    pub fn list_api_keys(&self) -> Result<Vec<ApiKeySummary>> {
        let response = self.get("/admin/api/keys.json")?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading api keys response")?;
        if !status.is_success() {
            return Err(http_error("api keys list request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing api keys response json")?;
        let keys_value = value
            .get("keys")
            .cloned()
            .unwrap_or(Value::Array(Vec::new()));
        let keys: Vec<ApiKeySummary> =
            serde_json::from_value(keys_value).context("deserialising api keys")?;
        Ok(keys)
    }

    /// Create a new API key. `username` of `None` makes a global all-users key.
    pub fn create_api_key(
        &self,
        description: &str,
        username: Option<&str>,
    ) -> Result<CreatedApiKey> {
        let mut payload: Vec<(&str, &str)> = vec![("key[description]", description)];
        if let Some(u) = username {
            payload.push(("key[username]", u));
        }
        let response =
            self.send_retrying(|| Ok(self.post("/admin/api/keys.json")?.form(&payload)))?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading api key create response")?;
        if !status.is_success() {
            return Err(http_error("api key create request", status, &text));
        }
        let value: Value =
            serde_json::from_str(&text).context("parsing api key create response")?;
        let key_obj = value.get("key").unwrap_or(&value);
        let created: CreatedApiKey =
            serde_json::from_value(key_obj.clone()).context("deserialising created api key")?;
        Ok(created)
    }

    pub fn revoke_api_key(&self, key_id: u64) -> Result<()> {
        let path = format!("/admin/api/keys/{}.json", key_id);
        let response = self.send_retrying(|| self.delete_builder(&path))?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text_capped()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            return Err(http_error("api key revoke request", status, &text));
        }
        Ok(())
    }
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::{DiscourseClient, MAX_PAGINATION_PAGES, ResponseBody};
use super::error::http_error;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Discourse's `Admin::ApiController::INDEX_LIMIT`: the most keys one page returns.
const API_KEYS_PAGE_SIZE: usize = 50;

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
    /// List every API key, following Discourse's `offset`/`limit` pagination
    /// (the controller caps each page at 50 keys).
    pub fn list_api_keys(&self) -> Result<Vec<ApiKeySummary>> {
        let mut all: Vec<ApiKeySummary> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..MAX_PAGINATION_PAGES {
            let path = format!(
                "/admin/api/keys.json?offset={}&limit={API_KEYS_PAGE_SIZE}",
                all.len()
            );
            let response = self.get(&path)?;
            let status = response.status();
            let text = response
                .text_capped()
                .context("reading api keys response")?;
            if !status.is_success() {
                return Err(http_error("api keys list request", status, &text));
            }
            let value: Value =
                serde_json::from_str(&text).context("parsing api keys response json")?;
            let keys_value = value
                .get("keys")
                .cloned()
                .unwrap_or(Value::Array(Vec::new()));
            let page: Vec<ApiKeySummary> =
                serde_json::from_value(keys_value).context("deserialising api keys")?;
            let page_len = page.len();
            let mut added = 0;
            for key in page {
                if seen.insert(key.id) {
                    all.push(key);
                    added += 1;
                }
            }
            if page_len < API_KEYS_PAGE_SIZE {
                return Ok(all);
            }
            if added == 0 {
                return Err(anyhow!("api key pagination made no progress"));
            }
        }
        Err(anyhow!(
            "api key pagination exceeded {MAX_PAGINATION_PAGES} pages"
        ))
    }

    /// Fetch one API key (`GET /admin/api/keys/:id.json`) as the raw key object,
    /// so fields such as `api_key_scopes` survive into structured output.
    pub fn get_api_key(&self, key_id: u64) -> Result<Value> {
        let path = format!("/admin/api/keys/{key_id}.json");
        let response = self.get(&path)?;
        let status = response.status();
        let text = response.text_capped().context("reading api key response")?;
        if !status.is_success() {
            return Err(http_error("api key show request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing api key response json")?;
        Ok(value.get("key").cloned().unwrap_or(value))
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

    /// Soft-revoke: sets `revoked_at`, reversible with [`Self::undo_revoke_api_key`].
    pub fn revoke_api_key(&self, key_id: u64) -> Result<()> {
        let path = format!("/admin/api/keys/{key_id}/revoke.json");
        let response = self.send_retrying(|| self.post(&path))?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text_capped()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            return Err(http_error("api key revoke request", status, &text));
        }
        Ok(())
    }

    /// Clears `revoked_at`, undoing a prior [`Self::revoke_api_key`].
    pub fn undo_revoke_api_key(&self, key_id: u64) -> Result<()> {
        let path = format!("/admin/api/keys/{key_id}/undo-revoke.json");
        let response = self.send_retrying(|| self.post(&path))?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text_capped()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            return Err(http_error("api key undo-revoke request", status, &text));
        }
        Ok(())
    }

    /// Permanently destroys the key record. Distinct from [`Self::revoke_api_key`],
    /// which is reversible.
    pub fn delete_api_key(&self, key_id: u64) -> Result<()> {
        let path = format!("/admin/api/keys/{key_id}.json");
        let response = self.send_retrying(|| self.delete_builder(&path))?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text_capped()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            return Err(http_error("api key delete request", status, &text));
        }
        Ok(())
    }
}

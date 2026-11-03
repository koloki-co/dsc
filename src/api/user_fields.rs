// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;

impl DiscourseClient {
    pub fn list_user_fields(&self) -> Result<Vec<Value>> {
        let response = self.get("/admin/config/user_fields.json")?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading user fields response")?;
        if !status.is_success() {
            return Err(http_error("list user fields request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing user fields response")?;
        value
            .get("user_fields")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| anyhow!("user fields response is missing user_fields"))
    }

    pub fn get_user_field(&self, id: i64) -> Result<Value> {
        let response = self.get(&format!("/admin/config/user_fields/{id}.json"))?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading user field response")?;
        if !status.is_success() {
            return Err(http_error("show user field request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing user field response")?;
        value
            .get("user_field")
            .cloned()
            .ok_or_else(|| anyhow!("user field response is missing user_field"))
    }

    pub fn create_user_field(&self, field: &Value) -> Result<Value> {
        let response = self.send_retrying(|| {
            Ok(self
                .post("/admin/config/user_fields.json")?
                .json(&json!({ "user_field": field })))
        })?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading create user field response")?;
        if !status.is_success() {
            return Err(http_error("create user field request", status, &text));
        }
        let value: Value =
            serde_json::from_str(&text).context("parsing create user field response")?;
        value
            .get("user_field")
            .cloned()
            .ok_or_else(|| anyhow!("create user field response is missing user_field"))
    }
}

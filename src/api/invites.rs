// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::DiscourseClient;
use super::error::http_error;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Distilled successful response from POST /invites.json.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InviteResult {
    pub id: u64,
    /// Magic-link the invitee receives.
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

impl DiscourseClient {
    /// Create a single email invite.
    ///
    /// `group_ids` is sent as comma-joined `group_ids` (Discourse accepts both
    /// `group_ids[]=...` repetition and a single comma-list; the latter is
    /// simpler in form-encoded bodies). `topic_id` and `custom_message` are
    /// optional. Returns the created invite (including the magic link).
    pub fn create_invite(
        &self,
        email: &str,
        group_ids: &[u64],
        topic_id: Option<u64>,
        custom_message: Option<&str>,
    ) -> Result<InviteResult> {
        let mut payload: Vec<(&str, String)> = vec![("email", email.to_string())];
        if !group_ids.is_empty() {
            payload.push((
                "group_ids",
                group_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ));
        }
        if let Some(topic) = topic_id {
            payload.push(("topic_id", topic.to_string()));
        }
        if let Some(msg) = custom_message
            && !msg.trim().is_empty()
        {
            payload.push(("custom_message", msg.to_string()));
        }
        let response = self.send_retrying(|| Ok(self.post("/invites.json")?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading invite response")?;
        if !status.is_success() {
            return Err(http_error("invite create request", status, &text));
        }
        // The response is sometimes the bare invite object, sometimes wrapped
        // — accept either.
        let value: Value = serde_json::from_str(&text).context("parsing invite response json")?;
        let target = value.get("invite").unwrap_or(&value);
        let result: InviteResult =
            serde_json::from_value(target.clone()).context("deserialising invite")?;
        Ok(result)
    }
}

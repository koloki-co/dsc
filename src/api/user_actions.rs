// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::DiscourseClient;
use super::error::http_error;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// One row from /user_actions.json. Distilled — Discourse returns more
/// fields than most callers need.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UserAction {
    pub action_type: u32,
    pub created_at: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    pub topic_id: u64,
    #[serde(default)]
    pub post_id: Option<u64>,
    #[serde(default)]
    pub post_number: Option<u64>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub excerpt: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserActionsResponse {
    #[serde(default)]
    user_actions: Vec<UserAction>,
}

impl DiscourseClient {
    /// Fetch a page of a user's activity. `filter_types` is a slice of
    /// Discourse's numeric action-type filters (e.g. 4 = new_topic, 5 = reply);
    /// they're joined with commas. `offset` paginates — Discourse returns
    /// ~10 items per page.
    pub fn fetch_user_actions(
        &self,
        username: &str,
        filter_types: &[u32],
        offset: u32,
    ) -> Result<Vec<UserAction>> {
        let types_csv = filter_types
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let path = format!(
            "/user_actions.json?username={}&filter={}&offset={}",
            username, types_csv, offset
        );
        let response = self.get(&path)?;
        let status = response.status();
        let text = response.text().context("reading user actions response")?;
        if !status.is_success() {
            return Err(http_error("user actions request", status, &text));
        }
        let body: UserActionsResponse =
            serde_json::from_str(&text).context("parsing user actions response")?;
        Ok(body.user_actions)
    }
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::DiscourseClient;
use super::error::http_error;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TagInfo {
    pub id: u64,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub pm_count: u64,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    #[serde(default)]
    tags: Vec<TagInfo>,
}

/// A tag group as returned by the Discourse admin API.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TagGroupInfo {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub tag_names: Vec<String>,
    #[serde(default)]
    pub one_per_topic: bool,
    #[serde(default)]
    pub parent_tag_name: Option<String>,
    #[serde(default)]
    pub permissions: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct TagGroupsResponse {
    #[serde(default)]
    tag_groups: Vec<TagGroupInfo>,
}

impl DiscourseClient {
    /// List every tag visible to the authenticated user.
    pub fn list_tags(&self) -> Result<Vec<TagInfo>> {
        let response = self.get("/tags.json")?;
        let status = response.status();
        let text = response.text().context("reading tags response body")?;
        if !status.is_success() {
            return Err(http_error("tags request", status, &text));
        }
        let body: TagsResponse =
            serde_json::from_str(&text).context("parsing tags response json")?;
        Ok(body.tags)
    }

    /// Fetch tag description via the tag detail endpoint.
    pub fn get_tag_description(&self, tag_name: &str) -> Result<Option<String>> {
        let path = format!("/tag/{}.json", tag_name);
        let response = self.get(&path)?;
        let status = response.status();
        let text = response.text().context("reading tag detail response")?;
        if !status.is_success() {
            return Ok(None);
        }
        let value: Value = serde_json::from_str(&text).unwrap_or_default();
        let desc = value
            .pointer("/topic_list/tags/0/description")
            .or_else(|| value.pointer("/tag/description"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        Ok(desc)
    }

    /// List tag groups (admin endpoint). Returns Err on non-2xx other than 403;
    /// returns Ok(None) on 403 (non-admin key).
    pub fn list_tag_groups(&self) -> Result<Option<Vec<TagGroupInfo>>> {
        let response = self.get("/tag_groups.json")?;
        let status = response.status();
        if status.as_u16() == 403 {
            return Ok(None);
        }
        let text = response
            .text()
            .context("reading tag groups response body")?;
        if !status.is_success() {
            return Err(http_error("tag groups request", status, &text));
        }
        let body: TagGroupsResponse =
            serde_json::from_str(&text).context("parsing tag groups response json")?;
        Ok(Some(body.tag_groups))
    }

    /// Create a tag group. Returns the created group's ID.
    pub fn create_tag_group(&self, payload: &Value) -> Result<u64> {
        let response = self.send_retrying(|| Ok(self.post("/tag_groups.json")?.json(payload)))?;
        let status = response.status();
        let text = response
            .text()
            .context("reading create tag group response")?;
        if !status.is_success() {
            return Err(http_error("create tag group", status, &text));
        }
        let value: Value =
            serde_json::from_str(&text).context("parsing create tag group response")?;
        let id = value
            .pointer("/tag_group/id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("tag group creation response missing id"))?;
        Ok(id)
    }

    /// Update an existing tag group.
    pub fn update_tag_group(&self, group_id: u64, payload: &Value) -> Result<()> {
        let path = format!("/tag_groups/{}.json", group_id);
        let response = self.send_retrying(|| Ok(self.put(&path)?.json(payload)))?;
        let status = response.status();
        let text = response
            .text()
            .context("reading update tag group response")?;
        if !status.is_success() {
            return Err(http_error("update tag group", status, &text));
        }
        Ok(())
    }

    /// Delete a tag group.
    pub fn delete_tag_group(&self, group_id: u64) -> Result<()> {
        let path = format!("/tag_groups/{}.json", group_id);
        let response = self.delete(&path)?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().unwrap_or_default();
            return Err(http_error("delete tag group", status, &text));
        }
        Ok(())
    }

    /// Update an existing tag's metadata (description).
    ///
    /// Note: this does NOT create a tag. Discourse exposes no standalone
    /// create-tag endpoint to an admin API key - `PUT /tag/{name}.json` returns
    /// 404 for a tag that does not yet exist (verified against 2026.7.0). Tags
    /// are materialised only implicitly: by a tag group (`POST /tag_groups.json`
    /// with `tag_names`) or by being assigned to a topic. Call this only for a
    /// tag that already exists (see `tag_push`, which reconciles groups first).
    pub fn update_tag(&self, tag_name: &str, description: Option<&str>) -> Result<()> {
        let path = format!("/tag/{}.json", tag_name);
        let mut payload = serde_json::Map::new();
        if let Some(desc) = description {
            payload.insert("tag".to_string(), serde_json::json!({"description": desc}));
        }
        let response = self.send_retrying(|| Ok(self.put(&path)?.json(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading update tag response")?;
        if !status.is_success() {
            return Err(http_error("update tag", status, &text));
        }
        Ok(())
    }

    /// Rename a tag, preserving topic associations. Discourse accepts a new
    /// `id` (slug) on the tag-update endpoint and reassigns every topic
    /// in-place.
    pub fn rename_tag(&self, old_name: &str, new_name: &str) -> Result<()> {
        let path = format!("/tag/{}.json", old_name);
        let payload = serde_json::json!({ "tag": { "id": new_name } });
        let response = self.send_retrying(|| Ok(self.put(&path)?.json(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading rename tag response")?;
        if !status.is_success() {
            return Err(http_error("rename tag", status, &text));
        }
        Ok(())
    }

    /// Delete a tag.
    ///
    /// The endpoint is singular `/tag/{name}.json` - the plural `/tags/...`
    /// form returns 404 and silently leaves the tag in place (verified against
    /// 2026.7.0).
    pub fn delete_tag(&self, tag_name: &str) -> Result<()> {
        let path = format!("/tag/{}.json", tag_name);
        let response = self.delete(&path)?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().unwrap_or_default();
            return Err(http_error("delete tag", status, &text));
        }
        Ok(())
    }

    /// Fetch the current tag list for a topic.
    pub fn fetch_topic_tags(&self, topic_id: u64) -> Result<Vec<String>> {
        let path = format!("/t/{}.json", topic_id);
        let response = self.get(&path)?;
        let status = response.status();
        let text = response.text().context("reading topic response body")?;
        if !status.is_success() {
            return Err(http_error("topic request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing topic response json")?;
        let tags = value
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(tags)
    }

    /// Replace the full tag list on a topic. Returns the resulting tag set.
    pub fn set_topic_tags(&self, topic_id: u64, tags: &[String]) -> Result<Vec<String>> {
        let path = format!("/t/{}.json", topic_id);
        let payload: Vec<(&str, &str)> = if tags.is_empty() {
            // Discourse needs at least one form field to clear tags;
            // sending an empty `tags[]` removes them all.
            vec![("tags[]", "")]
        } else {
            tags.iter().map(|t| ("tags[]", t.as_str())).collect()
        };
        let response = self.send_retrying(|| Ok(self.put(&path)?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading set-tags response body")?;
        if !status.is_success() {
            return Err(http_error("set tags request", status, &text));
        }
        // Confirm the post-update state by re-reading the topic; the PUT response
        // shape is awkward to depend on across versions.
        self.fetch_topic_tags(topic_id)
    }
}

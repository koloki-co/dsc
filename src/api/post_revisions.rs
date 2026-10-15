// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Client for Discourse's post revision routes
//! (`/posts/:id/revisions/latest.json`, `/posts/:id/revisions/:n.json`,
//! `PUT /posts/:id/revisions/:n/revert.json`). See
//! `spec/commands/post-revision-recovery.md`.

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Discourse only considers the latest 99 stored revisions of a post.
pub const REVISION_VISIBILITY_LIMIT: usize = 99;

/// One post revision as returned by the revision routes. The navigation
/// fields link the chain; the `*_changes` fields are kept as raw JSON because
/// their shape differs per field (body diffs are HTML, category is
/// previous/current, tags are arrays).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PostRevision {
    #[serde(default)]
    pub post_id: Option<u64>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub edit_reason: Option<String>,
    #[serde(default)]
    pub current_revision: Option<u64>,
    #[serde(default)]
    pub first_revision: Option<u64>,
    #[serde(default)]
    pub previous_revision: Option<u64>,
    #[serde(default)]
    pub next_revision: Option<u64>,
    #[serde(default)]
    pub last_revision: Option<u64>,
    #[serde(default)]
    pub version_count: Option<u64>,
    #[serde(default)]
    pub body_changes: Option<Value>,
    #[serde(default)]
    pub title_changes: Option<Value>,
    #[serde(default)]
    pub user_changes: Option<Value>,
    #[serde(default)]
    pub tags_changes: Option<Value>,
    #[serde(default)]
    pub category_id_changes: Option<Value>,
    #[serde(default)]
    pub wiki_changes: Option<Value>,
    #[serde(default)]
    pub post_type_changes: Option<Value>,
}

impl PostRevision {
    /// Names of the fields this revision changed, in a stable order.
    pub fn changed_fields(&self) -> Vec<&'static str> {
        [
            ("body", &self.body_changes),
            ("title", &self.title_changes),
            ("owner", &self.user_changes),
            ("tags", &self.tags_changes),
            ("category", &self.category_id_changes),
            ("wiki", &self.wiki_changes),
            ("post_type", &self.post_type_changes),
        ]
        .into_iter()
        .filter(|(_, v)| v.as_ref().is_some_and(value_is_substantive))
        .map(|(name, _)| name)
        .collect()
    }
}

fn value_is_substantive(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => o.values().any(value_is_substantive),
        _ => true,
    }
}

impl DiscourseClient {
    /// `GET /posts/:id/revisions/latest.json`.
    pub fn fetch_latest_post_revision(&self, post_id: u64) -> Result<PostRevision> {
        self.fetch_revision_at(&format!("/posts/{post_id}/revisions/latest.json"))
    }

    /// `GET /posts/:id/revisions/:revision.json`.
    pub fn fetch_post_revision(&self, post_id: u64, revision: u64) -> Result<PostRevision> {
        self.fetch_revision_at(&format!("/posts/{post_id}/revisions/{revision}.json"))
    }

    fn fetch_revision_at(&self, path: &str) -> Result<PostRevision> {
        let response = self.get(path)?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading post revision response body")?;
        if !status.is_success() {
            return Err(http_error("post revision request", status, &text));
        }
        serde_json::from_str(&text).context("parsing post revision response")
    }

    /// `PUT /posts/:id/revisions/:revision/revert.json` (staff only). Asks
    /// Discourse to apply the selected revision's prior values.
    pub fn revert_post_revision(&self, post_id: u64, revision: u64) -> Result<()> {
        let path = revert_path(post_id, revision);
        let response = self.send_retrying(|| self.put(&path))?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text_capped()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            return Err(http_error("revert post revision request", status, &text));
        }
        Ok(())
    }
}

pub fn revert_path(post_id: u64, revision: u64) -> String {
    format!("/posts/{post_id}/revisions/{revision}/revert.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_fields_ignores_empty_diffs() {
        let rev: PostRevision = serde_json::from_str(
            r#"{"body_changes":{"inline":"<div>x</div>"},"title_changes":{"inline":""},
                "tags_changes":{"previous":[],"current":[]},"category_id_changes":{"previous":1,"current":2}}"#,
        )
        .unwrap();
        assert_eq!(rev.changed_fields(), vec!["body", "category"]);
    }

    #[test]
    fn revert_path_is_exact() {
        assert_eq!(revert_path(5, 3), "/posts/5/revisions/3/revert.json");
    }
}

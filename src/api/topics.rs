// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::{DiscourseClient, json_page_path};
use super::error::http_error;
use super::models::{CreatePostResponse, TopicResponse};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PostInfo {
    pub id: u64,
    pub topic_id: u64,
    #[serde(default)]
    pub post_number: Option<u64>,
    #[serde(default)]
    pub raw: Option<String>,
}

/// Side-effect controls for a post edit (`PUT /posts/{id}.json`). The default
/// is an ordinary edit that bumps the topic and records a revision.
#[derive(Debug, Clone, Copy, Default)]
pub struct PostEditOptions {
    /// Send `post[no_bump]=true` so the edit does not bump the topic to the
    /// top of the category activity feed. For quiet maintenance edits.
    pub no_bump: bool,
    /// Send `post[skip_revision]=true` so the edit does not create a revision
    /// (edit-history) entry. Suppresses the online audit trail; use sparingly.
    pub skip_revision: bool,
}

/// Distilled row from /topics/private-messages-*.json.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PmTopicSummary {
    pub id: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub posts_count: Option<u64>,
    #[serde(default)]
    pub last_posted_at: Option<String>,
    #[serde(default)]
    pub last_poster_username: Option<String>,
    #[serde(default)]
    pub unread: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeletedTopicSummary {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub posts_count: u64,
    #[serde(default)]
    pub category_id: Option<u64>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

impl DiscourseClient {
    /// Fetch a topic by ID.
    pub fn fetch_topic(&self, topic_id: u64, include_raw: bool) -> Result<TopicResponse> {
        let path = if include_raw {
            format!("/t/{}.json?include_raw=1", topic_id)
        } else {
            format!("/t/{}.json", topic_id)
        };
        let response = self.get(&path)?;
        let status = response.status();
        let text = response.text().context("reading topic response body")?;
        if !status.is_success() {
            return Err(http_error("topic request", status, &text));
        }
        let body: TopicResponse = serde_json::from_str(&text).context("parsing topic json")?;
        Ok(body)
    }

    /// Fetch every post in a topic, in order.
    ///
    /// Discourse paginates `/t/{id}.json` at 20 posts per page. The first
    /// response also includes `post_stream.stream`, the flat array of every
    /// post ID in the thread. We page-1 first to learn the stream, then
    /// batch-fetch any remaining post IDs via
    /// `/t/{id}/posts.json?post_ids[]=…&include_raw=1`. Returns posts in
    /// stream order (matches topic display order).
    pub fn fetch_topic_all_posts(&self, topic_id: u64) -> Result<TopicResponse> {
        let mut topic = self.fetch_topic(topic_id, true)?;

        // Build the set of IDs we already have from page 1.
        let have: std::collections::HashSet<u64> =
            topic.post_stream.posts.iter().map(|p| p.id).collect();
        let missing: Vec<u64> = topic
            .post_stream
            .stream
            .iter()
            .copied()
            .filter(|id| !have.contains(id))
            .collect();

        // Batch-fetch missing posts in chunks of 20 (Discourse's page size).
        for chunk in missing.chunks(20) {
            let query: Vec<String> = chunk
                .iter()
                .map(|id| format!("post_ids[]={}", id))
                .collect();
            let path = format!(
                "/t/{}/posts.json?include_raw=1&{}",
                topic_id,
                query.join("&")
            );
            let response = self.get(&path)?;
            let status = response.status();
            let text = response
                .text()
                .context("reading topic posts response body")?;
            if !status.is_success() {
                return Err(http_error("topic posts request", status, &text));
            }
            let body: TopicResponse =
                serde_json::from_str(&text).context("parsing topic posts response")?;
            topic.post_stream.posts.extend(body.post_stream.posts);
        }

        // Reorder posts to match the canonical stream order.
        if !topic.post_stream.stream.is_empty() {
            let order: std::collections::HashMap<u64, usize> = topic
                .post_stream
                .stream
                .iter()
                .enumerate()
                .map(|(i, id)| (*id, i))
                .collect();
            topic
                .post_stream
                .posts
                .sort_by_key(|p| order.get(&p.id).copied().unwrap_or(usize::MAX));
        }

        Ok(topic)
    }

    /// Soft-delete or permanently delete a topic by ID (`DELETE /t/{id}`).
    pub fn delete_topic(&self, topic_id: u64, permanent: bool) -> Result<()> {
        let path = if permanent {
            format!("/t/{}.json?permanent=true", topic_id)
        } else {
            format!("/t/{}.json", topic_id)
        };
        let response = self.send_retrying(|| self.delete_builder(&path))?;
        let status = response.status();
        let text = response
            .text()
            .unwrap_or_else(|_| "<failed to read response body>".to_string());
        if !status.is_success() {
            return Err(http_error("delete topic request", status, &text));
        }
        Ok(())
    }

    /// Recover a soft-deleted topic by ID (`PUT /t/{id}/recover`).
    pub fn recover_topic(&self, topic_id: u64) -> Result<()> {
        let path = format!("/t/{}/recover.json", topic_id);
        let response = self.send_retrying(|| self.put(&path))?;
        let status = response.status();
        let text = response
            .text()
            .unwrap_or_else(|_| "<failed to read response body>".to_string());
        if !status.is_success() {
            return Err(http_error("recover topic request", status, &text));
        }
        Ok(())
    }

    /// List soft-deleted topics via Discourse search (`status:deleted`).
    pub fn list_deleted_topics(&self, query: Option<&str>) -> Result<Vec<DeletedTopicSummary>> {
        let q = deleted_topics_query(query);
        let hits = self.search_topics(&q)?;
        Ok(hits
            .into_iter()
            .map(|hit| DeletedTopicSummary {
                id: hit.id,
                title: hit.title,
                slug: hit.slug,
                posts_count: hit.posts_count,
                category_id: hit.category_id,
                tags: hit.tags,
            })
            .collect())
    }

    /// Fetch a post by ID and return its raw content.
    pub fn fetch_post_raw(&self, post_id: u64) -> Result<Option<String>> {
        Ok(self.fetch_post(post_id)?.raw)
    }

    /// Fetch a post's metadata (id, topic_id, post_number, raw).
    pub fn fetch_post(&self, post_id: u64) -> Result<PostInfo> {
        let path = format!("/posts/{}.json?include_raw=1", post_id);
        let response = self.get(&path)?;
        let status = response.status();
        let text = response.text().context("reading post response body")?;
        if !status.is_success() {
            return Err(http_error("post request", status, &text));
        }
        let info: PostInfo = serde_json::from_str(&text).context("parsing post response")?;
        Ok(info)
    }

    /// Soft-delete a post by ID (DELETE /posts/:id.json).
    pub fn delete_post(&self, post_id: u64) -> Result<()> {
        let path = format!("/posts/{}.json", post_id);
        let response = self.send_retrying(|| self.delete_builder(&path))?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            return Err(http_error("delete post request", status, &text));
        }
        Ok(())
    }

    /// Move one or more posts from their current topic to another topic.
    ///
    /// `source_topic_id` is the topic the posts currently live in.
    /// `post_ids` are the post IDs to move. `dest_topic_id` is where they land.
    /// Returns the new URL of the moved posts' topic.
    pub fn move_posts(
        &self,
        source_topic_id: u64,
        post_ids: &[u64],
        dest_topic_id: u64,
    ) -> Result<String> {
        if post_ids.is_empty() {
            return Err(anyhow!("no post IDs supplied to move"));
        }
        let dest = dest_topic_id.to_string();
        let path = format!("/t/{}/move-posts.json", source_topic_id);
        let mut payload: Vec<(String, String)> = Vec::new();
        payload.push(("destination_topic_id".to_string(), dest.clone()));
        for id in post_ids {
            payload.push(("post_ids[]".to_string(), id.to_string()));
        }
        let response = self.send_retrying(|| Ok(self.post(&path)?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading move-posts response")?;
        if !status.is_success() {
            return Err(http_error("move posts request", status, &text));
        }
        let value: Value = serde_json::from_str(&text).context("parsing move-posts response")?;
        let url = value
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("/t/{}", dest));
        Ok(url)
    }

    /// Rename a topic via `PUT /t/{id}.json` with `title=`. Surfaces
    /// Discourse's reserved-slug `403` (e.g. a topic whose slug is `contact`,
    /// a system route) with a clear message rather than the generic forbidden
    /// error.
    pub fn set_topic_title(&self, topic_id: u64, title: &str) -> Result<()> {
        let path = format!("/t/{}.json", topic_id);
        let payload = [("title", title)];
        let response = self.send_retrying(|| Ok(self.put(&path)?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading set-title response body")?;
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(anyhow!(
                "topic {} title cannot be changed (reserved slug or insufficient permission)",
                topic_id
            ));
        }
        if !status.is_success() {
            return Err(http_error("set title request", status, &text));
        }
        Ok(())
    }

    /// Update a post by ID. `opts` controls Discourse's edit side effects
    /// (topic bump, revision history); [`PostEditOptions::default`] applies a
    /// normal edit.
    pub fn update_post(&self, post_id: u64, raw: &str, opts: PostEditOptions) -> Result<()> {
        let path = format!("/posts/{}.json", post_id);
        let payload = post_edit_payload(raw, opts);
        let response = self.send_retrying(|| Ok(self.put(&path)?.form(&payload)))?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            return Err(http_error("update post request", status, &text));
        }
        Ok(())
    }

    /// Create a new topic in a category.
    pub fn create_topic(&self, category_id: u64, title: &str, raw: &str) -> Result<u64> {
        let category = category_id.to_string();
        let payload = [("title", title), ("raw", raw), ("category", &category)];
        let response = self.send_retrying(|| Ok(self.post("/posts.json")?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading create response body")?;
        if !status.is_success() {
            return Err(http_error("create topic request", status, &text));
        }
        let body: CreatePostResponse =
            serde_json::from_str(&text).context("parsing create topic response")?;
        Ok(body.topic_id)
    }

    /// Send a private message. `recipients` is comma-joined into Discourse's
    /// `target_recipients` field (usernames or group names accepted).
    /// Returns the new topic_id of the PM thread.
    pub fn create_private_message(
        &self,
        recipients: &[String],
        title: &str,
        raw: &str,
    ) -> Result<u64> {
        let recipients_csv = recipients.join(",");
        let payload = [
            ("title", title),
            ("raw", raw),
            ("archetype", "private_message"),
            ("target_recipients", recipients_csv.as_str()),
        ];
        let response = self.send_retrying(|| Ok(self.post("/posts.json")?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading PM create response body")?;
        if !status.is_success() {
            return Err(http_error("create PM request", status, &text));
        }
        let body: CreatePostResponse =
            serde_json::from_str(&text).context("parsing PM create response")?;
        Ok(body.topic_id)
    }

    /// List private messages for the given user. `direction` is one of
    /// `inbox` (received), `sent`, `archive`, `unread`, `new`. Returns
    /// distilled topic summaries.
    pub fn list_private_messages(
        &self,
        username: &str,
        direction: &str,
    ) -> Result<Vec<PmTopicSummary>> {
        let path = match direction {
            "inbox" => format!("/topics/private-messages/{}.json", username),
            "sent" => format!("/topics/private-messages-sent/{}.json", username),
            "archive" => format!("/topics/private-messages-archive/{}.json", username),
            "unread" => format!("/topics/private-messages-unread/{}.json", username),
            "new" => format!("/topics/private-messages-new/{}.json", username),
            other => format!("/topics/private-messages-{}/{}.json", other, username),
        };
        let mut topics = Vec::new();
        let mut seen_topics = HashSet::new();
        let mut seen_paths = HashSet::new();
        let mut next = Some(path);
        while let Some(path) = next {
            if !seen_paths.insert(path.clone()) {
                return Err(anyhow!(
                    "private-message pagination loop detected: {}",
                    path
                ));
            }
            let path = json_page_path(&path)?;
            let response = self.get(&path)?;
            let status = response.status();
            let text = response.text().context("reading PM list response")?;
            if !status.is_success() {
                return Err(http_error("PM list request", status, &text));
            }
            let value: Value = serde_json::from_str(&text).context("parsing PM list response")?;
            let topic_list = value
                .get("topic_list")
                .ok_or_else(|| anyhow!("PM list response missing topic_list"))?;
            let page_topics = topic_list
                .get("topics")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("PM list response missing topics array"))?;
            for value in page_topics {
                let topic: PmTopicSummary =
                    serde_json::from_value(value.clone()).context("parsing PM topic summary")?;
                if seen_topics.insert(topic.id) {
                    topics.push(topic);
                }
            }
            next = topic_list
                .get("more_topics_url")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        Ok(topics)
    }

    /// Create a reply post in a topic.
    pub fn create_post(&self, topic_id: u64, raw: &str) -> Result<u64> {
        let topic = topic_id.to_string();
        let payload = [("topic_id", topic.as_str()), ("raw", raw)];
        let response = self.send_retrying(|| Ok(self.post("/posts.json")?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading create response body")?;
        if !status.is_success() {
            return Err(http_error("create post request", status, &text));
        }
        let body: CreatePostResponse =
            serde_json::from_str(&text).context("parsing create post response")?;
        Ok(body.id)
    }
}

/// Build a Discourse search query for soft-deleted topics, preserving any
/// user-supplied narrowing terms while ensuring `status:deleted` is present.
fn deleted_topics_query(query: Option<&str>) -> String {
    match query.map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) if q.contains("status:deleted") => q.to_string(),
        Some(q) => format!("{} status:deleted", q),
        None => "status:deleted".to_string(),
    }
}

fn post_edit_payload<'a>(raw: &'a str, opts: PostEditOptions) -> Vec<(&'static str, &'a str)> {
    let mut payload: Vec<(&'static str, &'a str)> = vec![("post[raw]", raw)];
    if opts.no_bump {
        payload.push(("post[no_bump]", "true"));
    }
    if opts.skip_revision {
        payload.push(("post[skip_revision]", "true"));
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleted_topics_query_defaults_to_status_deleted() {
        assert_eq!(deleted_topics_query(None), "status:deleted");
        assert_eq!(deleted_topics_query(Some("  ")), "status:deleted");
    }

    #[test]
    fn deleted_topics_query_adds_status_deleted_to_terms() {
        assert_eq!(
            deleted_topics_query(Some("house archive")),
            "house archive status:deleted"
        );
    }

    #[test]
    fn deleted_topics_query_does_not_duplicate_status_deleted() {
        assert_eq!(
            deleted_topics_query(Some("status:deleted category:staff")),
            "status:deleted category:staff"
        );
    }

    #[test]
    fn default_edit_sends_only_raw() {
        let payload = post_edit_payload("hello", PostEditOptions::default());
        assert_eq!(payload, vec![("post[raw]", "hello")]);
    }

    #[test]
    fn no_bump_adds_form_field() {
        let payload = post_edit_payload(
            "hi",
            PostEditOptions {
                no_bump: true,
                skip_revision: false,
            },
        );
        assert_eq!(
            payload,
            vec![("post[raw]", "hi"), ("post[no_bump]", "true")]
        );
    }

    #[test]
    fn skip_revision_adds_form_field() {
        let payload = post_edit_payload(
            "hi",
            PostEditOptions {
                no_bump: false,
                skip_revision: true,
            },
        );
        assert_eq!(
            payload,
            vec![("post[raw]", "hi"), ("post[skip_revision]", "true")]
        );
    }

    #[test]
    fn both_flags_add_both_fields() {
        let payload = post_edit_payload(
            "x",
            PostEditOptions {
                no_bump: true,
                skip_revision: true,
            },
        );
        assert_eq!(
            payload,
            vec![
                ("post[raw]", "x"),
                ("post[no_bump]", "true"),
                ("post[skip_revision]", "true"),
            ]
        );
    }
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One result row in a search response — distilled from the topic stanza of
/// `/search.json` (which contains far more than we need).
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SearchHit {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub posts_count: u64,
    #[serde(default)]
    pub category_id: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_search_tags")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RawSearchResponse {
    #[serde(default)]
    topics: Vec<SearchHit>,
    #[serde(default)]
    more_full_page_results: bool,
}

impl DiscourseClient {
    /// Search for topics. The `query` is passed through to Discourse verbatim
    /// (so callers can use `category:`, `status:`, `@user`, etc. filters).
    pub fn search_topics(&self, query: &str) -> Result<Vec<SearchHit>> {
        let mut topics = Vec::new();
        let mut seen = HashSet::new();
        for page in 1..=10 {
            let path = format!("/search.json?q={}&page={}", urlencode_form(query), page);
            let response = self.get(&path)?;
            let status = response.status();
            let text = response
                .text_capped()
                .context("reading search response body")?;
            if !status.is_success() {
                return Err(http_error("search request", status, &text));
            }
            let body: RawSearchResponse =
                serde_json::from_str(&text).context("parsing search response json")?;
            for topic in body.topics {
                if seen.insert(topic.id) {
                    topics.push(topic);
                }
            }
            if !body.more_full_page_results {
                return Ok(topics);
            }
        }
        Err(anyhow::anyhow!(
            "search results exceed Discourse's 10-page API limit; narrow the query"
        ))
    }
}

fn deserialize_search_tags<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(items) = value.as_array() else {
        return Ok(None);
    };
    let tags = items
        .iter()
        .filter_map(|item| {
            item.as_str()
                .map(|s| s.to_string())
                .or_else(|| {
                    item.get("name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .or_else(|| {
                    item.get("slug")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
        })
        .collect();
    Ok(Some(tags))
}

/// Minimal `application/x-www-form-urlencoded` encoder for the query string.
/// Avoids pulling in an extra crate just for one field.
pub(super) fn urlencode_form(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        let b = *byte;
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{deserialize_search_tags, urlencode_form};
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct TagsWrapper {
        #[serde(default, deserialize_with = "deserialize_search_tags")]
        tags: Option<Vec<String>>,
    }

    #[test]
    fn search_tags_accept_plain_strings() {
        let got: TagsWrapper = serde_json::from_str(r#"{"tags":["bug","ops"]}"#).unwrap();
        assert_eq!(got.tags.unwrap(), vec!["bug", "ops"]);
    }

    #[test]
    fn search_tags_accept_tag_objects() {
        let got: TagsWrapper = serde_json::from_str(
            r#"{"tags":[{"id":1,"name":"bug","slug":"bug"},{"id":2,"slug":"ops"}]}"#,
        )
        .unwrap();
        assert_eq!(got.tags.unwrap(), vec!["bug", "ops"]);
    }

    #[test]
    fn encodes_spaces_as_plus() {
        assert_eq!(urlencode_form("hello world"), "hello+world");
    }

    #[test]
    fn encodes_special_chars_percent() {
        assert_eq!(urlencode_form("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn passes_alnum_unchanged() {
        assert_eq!(urlencode_form("Topic42"), "Topic42");
    }

    #[test]
    fn passes_unreserved_unchanged() {
        assert_eq!(urlencode_form("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn encodes_discourse_filter_syntax() {
        // Things like `category:foo @user` should round-trip through Discourse fine.
        assert_eq!(
            urlencode_form("hello category:foo @bob"),
            "hello+category%3Afoo+%40bob"
        );
    }
}

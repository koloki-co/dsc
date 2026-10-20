// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{DiscourseClient, PostEditOptions, PostRevision, REVISION_VISIBILITY_LIMIT};
use crate::cli::ListFormat;
use crate::commands::common::{
    emit_with, ensure_api_credentials, read_path_or_stdin, select_discourse,
};
use crate::commands::topic::topic_change_owner;
use crate::config::Config;
use crate::utils::{atomic_write, normalize_baseurl};
use anyhow::{Result, anyhow};
use serde::Serialize;
use std::io::{self, Write};
use std::path::Path;

pub fn post_pull(
    config: &Config,
    discourse_name: &str,
    post_id: u64,
    local_path: Option<&Path>,
    force: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let raw = client
        .fetch_post_raw(post_id)?
        .ok_or_else(|| anyhow!("post {} has no raw content", post_id))?;

    match local_path {
        Some(path) => {
            atomic_write(path, &raw, force)?;
            println!("Post {} pulled to {}", post_id, path.display());
        }
        None => {
            io::stdout().write_all(raw.as_bytes())?;
        }
    }
    Ok(())
}

pub fn post_edit(
    config: &Config,
    discourse_name: &str,
    post_id: u64,
    local_path: Option<&Path>,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let raw = read_body(local_path)?;
    if raw.trim().is_empty() {
        return Err(anyhow!("post body is empty"));
    }

    if dry_run {
        println!(
            "[dry-run] {}: would replace post {} with {} bytes",
            discourse.name,
            post_id,
            raw.len()
        );
        return Ok(());
    }

    client.update_post(post_id, &raw, PostEditOptions::default())?;
    println!("Post {} updated", post_id);
    Ok(())
}

pub fn post_delete(
    config: &Config,
    discourse_name: &str,
    post_id: u64,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!(
            "[dry-run] {}: would delete post {}",
            discourse.name, post_id
        );
        return Ok(());
    }

    client.delete_post(post_id)?;
    println!("Post {} deleted", post_id);
    Ok(())
}

pub fn post_move(
    config: &Config,
    discourse_name: &str,
    post_id: u64,
    to_topic: u64,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let info = client.fetch_post(post_id)?;
    if info.topic_id == to_topic {
        return Err(anyhow!("post {} is already in topic {}", post_id, to_topic));
    }

    if dry_run {
        println!(
            "[dry-run] {}: would move post {} from topic {} to topic {}",
            discourse.name, post_id, info.topic_id, to_topic
        );
        return Ok(());
    }

    let url = client.move_posts(info.topic_id, &[post_id], to_topic)?;
    println!("Moved post {} → topic {} ({})", post_id, to_topic, url);
    Ok(())
}

#[derive(Debug, Serialize)]
struct PostInfoTopicOutput {
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deleted_at: Option<String>,
}

/// Output for `dsc post info`. Deliberately its own struct rather than a
/// reuse of the API's `PostInfo`/`TopicResponse`, so that raw content,
/// author fields, and anything else those API models pick up in future can
/// never leak into this read-only lookup's output.
#[derive(Debug, Serialize)]
struct PostInfoOutput {
    id: u64,
    topic: PostInfoTopicOutput,
    post_number: u64,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    deleted_at: Option<String>,
}

pub fn post_info(
    config: &Config,
    discourse_name: &str,
    post_id: u64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let post = client.fetch_post_metadata(post_id)?;
    let topic = client.fetch_topic(post.topic_id, false)?;

    let post_number = post.post_number.unwrap_or(1);
    let url = post.post_url.clone().unwrap_or_else(|| {
        let base = normalize_baseurl(&discourse.baseurl);
        let slug = topic.slug.as_deref().unwrap_or("topic");
        format!("{}/t/{}/{}/{}", base, slug, post.topic_id, post_number)
    });

    let output = PostInfoOutput {
        id: post.id,
        topic: PostInfoTopicOutput {
            id: topic.id.unwrap_or(post.topic_id),
            title: topic.title,
            slug: topic.slug,
            category_id: topic.category_id,
            deleted_at: topic.deleted_at,
        },
        post_number,
        url,
        deleted_at: post.deleted_at,
    };

    emit_with(format, &output, || {
        println!("id:          {}", output.id);
        println!("topic_id:    {}", output.topic.id);
        if let Some(title) = &output.topic.title {
            println!("title:       {}", title);
        }
        if let Some(slug) = &output.topic.slug {
            println!("slug:        {}", slug);
        }
        if let Some(category_id) = output.topic.category_id {
            println!("category_id: {}", category_id);
        }
        println!("post_number: {}", output.post_number);
        println!("url:         {}", output.url);
        if let Some(deleted_at) = &output.deleted_at {
            println!("deleted_at:  {}", deleted_at);
        }
        if let Some(topic_deleted_at) = &output.topic.deleted_at {
            println!("topic_deleted_at: {}", topic_deleted_at);
        }
        Ok(())
    })
}

/// Reassign the visible author of a single post by ID, without needing its
/// topic ID. Resolves the post's topic and delegates to `topic change-owner`
/// scoped to just that post.
pub fn post_change_owner(
    config: &Config,
    discourse_name: &str,
    post_id: u64,
    username: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let post = client.fetch_post_metadata(post_id)?;
    topic_change_owner(
        config,
        discourse_name,
        post.topic_id,
        username,
        &[post_id],
        dry_run,
    )
}

#[derive(Debug, Serialize)]
struct RevisionSummary {
    revision: u64,
    created_at: Option<String>,
    editor: Option<String>,
    edit_reason: Option<String>,
    changed_fields: Vec<&'static str>,
}

fn summarize(rev: &PostRevision, number: u64) -> RevisionSummary {
    RevisionSummary {
        revision: number,
        created_at: rev.created_at.clone(),
        editor: rev.username.clone(),
        edit_reason: rev.edit_reason.clone().filter(|r| !r.is_empty()),
        changed_fields: rev.changed_fields(),
    }
}

/// Newest-first revision summaries, following the server-provided
/// `previous_revision` links from `revisions/latest`. Bounded by
/// Discourse's own 99-revision visibility limit.
fn collect_revisions(client: &DiscourseClient, post_id: u64) -> Result<Vec<RevisionSummary>> {
    let latest = client.fetch_latest_post_revision(post_id)?;
    let mut number = latest
        .current_revision
        .ok_or_else(|| anyhow!("post {post_id} has no visible revisions"))?;
    let mut out = vec![summarize(&latest, number)];
    let mut current = latest;
    while let Some(prev) = current.previous_revision {
        if prev >= number || out.len() >= REVISION_VISIBILITY_LIMIT {
            break;
        }
        current = client.fetch_post_revision(post_id, prev)?;
        number = prev;
        out.push(summarize(&current, number));
    }
    Ok(out)
}

pub fn post_revision_list(
    config: &Config,
    discourse_name: &str,
    post_id: u64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let revisions = collect_revisions(&client, post_id)?;
    emit_with(format, &revisions, || {
        for r in &revisions {
            println!(
                "r{:<4} {}  {}  [{}]{}",
                r.revision,
                r.created_at.as_deref().unwrap_or("-"),
                r.editor.as_deref().unwrap_or("-"),
                r.changed_fields.join(","),
                r.edit_reason
                    .as_ref()
                    .map(|e| format!("  reason: {e}"))
                    .unwrap_or_default()
            );
        }
        eprintln!(
            "note: Discourse exposes at most the latest {REVISION_VISIBILITY_LIMIT} revisions of a post; older history may not be listed"
        );
        Ok(())
    })
}

/// Reduce Discourse's HTML diff of raw Markdown to plain text, marking
/// insertions `{+...+}` and deletions `[-...-]`. Never emits cooked HTML.
fn markdown_diff_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
        let name = tag.trim_start_matches('/');
        let name = name.split(|c: char| c.is_whitespace()).next().unwrap_or("");
        let closing = tag.starts_with('/');
        match (name, closing) {
            ("ins", false) => out.push_str("{+"),
            ("ins", true) => out.push_str("+}"),
            ("del", false) => out.push_str("[-"),
            ("del", true) => out.push_str("-]"),
            ("tr" | "br", _) | ("p" | "div", true) => out.push('\n'),
            ("td", true) => out.push_str(" | "),
            _ => {}
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

pub fn post_revision_show(
    config: &Config,
    discourse_name: &str,
    post_id: u64,
    revision: u64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let rev = client.fetch_post_revision(post_id, revision)?;
    emit_with(format, &rev, || {
        let s = summarize(&rev, revision);
        println!("post:        {post_id}");
        println!("revision:    {revision}");
        println!("created_at:  {}", s.created_at.as_deref().unwrap_or("-"));
        println!("editor:      {}", s.editor.as_deref().unwrap_or("-"));
        if let Some(reason) = &s.edit_reason {
            println!("edit_reason: {reason}");
        }
        println!("changed:     {}", s.changed_fields.join(", "));
        if let Some(body) = rev.body_changes.as_ref().and_then(|b| {
            b.get("side_by_side_markdown")
                .or_else(|| b.get("inline"))
                .and_then(|v| v.as_str())
        }) {
            println!("\nbody diff (markdown; {{+added+}} [-removed-]):");
            println!("{}", markdown_diff_text(body).trim());
        }
        Ok(())
    })
}

pub fn post_revision_revert(
    config: &Config,
    discourse_name: &str,
    post_id: u64,
    revision: u64,
    yes: bool,
    dry_run: bool,
) -> Result<()> {
    if revision < 2 {
        return Err(anyhow!(
            "revision {revision} cannot be reverted: Discourse only reverts revisions numbered 2 or higher"
        ));
    }
    if !dry_run && !yes {
        return Err(anyhow!(
            "reverting a post revision changes live content; re-run with --yes (or --dry-run to preview)"
        ));
    }
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let post = client.fetch_post_metadata(post_id)?;
    let target = client.fetch_post_revision(post_id, revision)?;
    let before = client.fetch_latest_post_revision(post_id)?;
    let before_number = before.current_revision.unwrap_or(0);
    let changed = target.changed_fields();
    let path = crate::api::revert_path(post_id, revision);

    if dry_run {
        println!(
            "[dry-run] {}: would revert post {} (topic {}) by applying the prior values of revision {} (fields: {}); current revision is {}",
            discourse.name,
            post_id,
            post.topic_id,
            revision,
            if changed.is_empty() {
                "none reported".to_string()
            } else {
                changed.join(", ")
            },
            before_number
        );
        println!("[dry-run] would send: PUT {path}");
        return Ok(());
    }

    client.revert_post_revision(post_id, revision)?;
    let after = client.fetch_latest_post_revision(post_id)?;
    let after_number = after.current_revision.unwrap_or(0);
    if after_number <= before_number {
        return Err(anyhow!(
            "revert request succeeded but post {post_id} still reports revision {after_number}; verify manually"
        ));
    }
    println!(
        "Post {post_id}: applied prior values of revision {revision}; now at revision {after_number}{}",
        after
            .edit_reason
            .as_deref()
            .filter(|r| !r.is_empty())
            .map(|r| format!(" (edit reason: {r})"))
            .unwrap_or_default()
    );
    Ok(())
}

fn read_body(local_path: Option<&Path>) -> Result<String> {
    read_path_or_stdin(local_path, "post body")
}

#[cfg(test)]
mod tests {
    use super::{markdown_diff_text, read_body};
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn read_body_from_file_roundtrips_contents() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "Edited body").unwrap();
        let got = read_body(Some(f.path())).unwrap();
        assert_eq!(got.trim(), "Edited body");
    }

    #[test]
    fn read_body_missing_file_surfaces_path_in_error() {
        let bogus = std::path::Path::new("/definitely/does/not/exist.md");
        let err = read_body(Some(bogus)).unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains("/definitely/does/not/exist.md"));
    }

    #[test]
    fn markdown_diff_text_marks_changes_and_strips_html() {
        let got = markdown_diff_text("<tr><td>a <del>old &amp; x</del><ins>new</ins></td></tr>");
        assert!(got.contains("[-old & x-]{+new+}"), "{got}");
        assert!(!got.contains('<'));
    }
}

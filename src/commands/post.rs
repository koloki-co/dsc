// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{DiscourseClient, PostEditOptions};
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::commands::topic::topic_change_owner;
use crate::config::Config;
use crate::utils::{atomic_write, normalize_baseurl};
use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use std::fs;
use std::io::{self, Read, Write};
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

    match format {
        ListFormat::Text => {
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
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&output)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&output)?),
    }

    Ok(())
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

fn read_body(local_path: Option<&Path>) -> Result<String> {
    let from_stdin = match local_path {
        None => true,
        Some(p) => p.as_os_str() == "-",
    };
    if from_stdin {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .context("reading post body from stdin")?;
        Ok(buf)
    } else {
        let path = local_path.unwrap();
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::read_body;
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
}

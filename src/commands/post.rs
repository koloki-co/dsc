// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{DiscourseClient, PostEditOptions};
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::atomic_write;
use anyhow::{Context, Result, anyhow};
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

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{DiscourseClient, SearchHit};
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse, selected_discourses};
use crate::config::{Config, DiscourseConfig};
use anyhow::{Result, anyhow};
use serde::Serialize;

pub fn search(
    config: &Config,
    discourse_name: &str,
    query: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    let hits = search_one(discourse, query)?;

    match format {
        ListFormat::Text => {
            if hits.is_empty() {
                println!("No search results found.");
                return Ok(());
            }
            let id_width = hits
                .iter()
                .map(|h| h.id.to_string().len())
                .max()
                .unwrap_or(2);
            for hit in &hits {
                println!(
                    "{:>width$}  {}",
                    hit.id,
                    display_title(hit),
                    width = id_width
                );
            }
        }
        ListFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&hits)?);
        }
        ListFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&hits)?);
        }
    }

    Ok(())
}

/// Merged fan-out search: query every configured forum and print one combined,
/// forum-tagged result list. Continues past per-forum failures (missing
/// credentials, unreachable forum) so one bad entry doesn't blank the rest of
/// the fleet; fails at the end if any forum could not be searched.
pub fn search_all(
    config: &Config,
    query: &str,
    tags: Option<&str>,
    format: ListFormat,
) -> Result<()> {
    let discourses = selected_discourses(config, None, tags)?;
    if discourses.is_empty() {
        return Err(if tags.is_some() {
            anyhow!("no discourses configured matching the given tags")
        } else {
            anyhow!("no discourses configured")
        });
    }

    let mut hits: Vec<ForumHit> = Vec::new();
    let mut failed = 0usize;
    for discourse in &discourses {
        match search_one(discourse, query) {
            Ok(forum_hits) => hits.extend(forum_hits.into_iter().map(|hit| ForumHit {
                forum: discourse.name.clone(),
                hit,
            })),
            Err(e) => {
                failed += 1;
                eprintln!("{}: search failed - {e}", discourse.name);
            }
        }
    }

    match format {
        ListFormat::Text => {
            if hits.is_empty() {
                println!("No search results found.");
            } else {
                let forum_width = hits.iter().map(|h| h.forum.len()).max().unwrap_or(4);
                let id_width = hits
                    .iter()
                    .map(|h| h.hit.id.to_string().len())
                    .max()
                    .unwrap_or(2);
                for h in &hits {
                    println!(
                        "{:<forum_width$}  {:>id_width$}  {}",
                        h.forum,
                        h.hit.id,
                        display_title(&h.hit),
                        forum_width = forum_width,
                        id_width = id_width
                    );
                }
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&hits)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&hits)?),
    }

    if failed > 0 {
        return Err(anyhow!(
            "search failed on {failed} of {} forum(s)",
            discourses.len()
        ));
    }
    Ok(())
}

fn search_one(discourse: &DiscourseConfig, query: &str) -> Result<Vec<SearchHit>> {
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    client.search_topics(query)
}

fn display_title(hit: &SearchHit) -> &str {
    if hit.title.trim().is_empty() {
        hit.slug.as_str()
    } else {
        hit.title.as_str()
    }
}

#[derive(Serialize)]
struct ForumHit {
    forum: String,
    #[serde(flatten)]
    hit: SearchHit,
}

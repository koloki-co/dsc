// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{
    emit_result, ensure_api_credentials, fleet_worker_count, run_fleet, select_discourse,
    selected_discourses,
};
use crate::config::{Config, DiscourseConfig};
use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::json;

/// Report dsc's own version, one forum, or a selected fleet.
pub fn version(
    config: &Config,
    discourse: Option<&str>,
    all: bool,
    tags: Option<&str>,
    format: ListFormat,
) -> Result<()> {
    match (discourse, all, tags) {
        (Some(name), false, None) => forum_version(config, name, format),
        (None, false, None) => own_version(format),
        (None, _, tags) => fleet_version(config, tags, format),
        _ => unreachable!("clap enforces mutually exclusive version selectors"),
    }
}

/// Report dsc's own version without loading configuration or contacting a forum.
pub fn own_version(format: ListFormat) -> Result<()> {
    let ver = env!("CARGO_PKG_VERSION");
    emit_result(format, &json!({ "name": "dsc", "version": ver }), ver)
}

/// Print a configured forum's live Discourse version and git commit.
pub fn forum_version(config: &Config, discourse_name: &str, format: ListFormat) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    let (version, commit) = fetch_forum_version(discourse)?;
    let text = format!("{}: Discourse {} ({})", discourse.name, version, commit);
    emit_result(
        format,
        &json!({ "discourse": discourse.name, "version": version, "commit": commit }),
        &text,
    )
}

#[derive(Debug, Serialize)]
struct VersionRow {
    discourse: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn fleet_version(config: &Config, tags: Option<&str>, format: ListFormat) -> Result<()> {
    let discourses = selected_discourses(config, None, tags)?;
    if discourses.is_empty() {
        return Err(if tags.is_some() {
            anyhow!("no discourses configured matching the given tags")
        } else {
            anyhow!("no discourses configured")
        });
    }

    let rows: Vec<VersionRow> = run_fleet(
        &discourses,
        fleet_worker_count(None, discourses.len(), 8, false),
        |discourse| match fetch_forum_version(discourse) {
            Ok((version, commit)) => VersionRow {
                discourse: discourse.name.clone(),
                version: Some(version),
                commit: Some(commit),
                error: None,
            },
            Err(error) => VersionRow {
                discourse: discourse.name.clone(),
                version: None,
                commit: None,
                error: Some(error.to_string()),
            },
        },
        |row| {
            if let Some(error) = &row.error {
                eprintln!("{}: version lookup failed - {error}", row.discourse);
            }
        },
    );

    match format {
        ListFormat::Text => {
            for row in &rows {
                if let Some(error) = &row.error {
                    println!("{}: ERROR: {error}", row.discourse);
                } else {
                    println!(
                        "{}: Discourse {} ({})",
                        row.discourse,
                        row.version.as_deref().unwrap_or("(unknown)"),
                        row.commit.as_deref().unwrap_or("(unknown)")
                    );
                }
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&rows)?),
    }

    let failed = rows.iter().filter(|row| row.error.is_some()).count();
    if failed > 0 {
        return Err(anyhow!(
            "version lookup failed on {failed} of {} forum(s)",
            rows.len()
        ));
    }
    Ok(())
}

fn fetch_forum_version(discourse: &DiscourseConfig) -> Result<(String, String)> {
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let info = client.fetch_version_info()?;
    Ok((
        info.version.unwrap_or_else(|| "(unknown)".to_string()),
        info.commit.unwrap_or_else(|| "(unknown)".to_string()),
    ))
}

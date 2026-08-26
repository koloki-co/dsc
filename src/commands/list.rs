// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::cli::OutputFormat;
use crate::commands::common::{fetch_fullnames, open_url, parse_tags};
use crate::config::{Config, DiscourseConfig, save_config};
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::io;
use std::path::Path;

/// Public list representation. Keep this explicit so config credentials can
/// never appear merely because a new field is added to `DiscourseConfig`.
#[derive(Serialize)]
struct DiscourseListEntry<'a> {
    name: &'a str,
    baseurl: &'a str,
    fullname: Option<&'a str>,
    api_username: Option<&'a str>,
    tags: Option<&'a [String]>,
    changelog_topic_id: Option<u64>,
    ssh_host: Option<&'a str>,
    docker_rootless: Option<bool>,
}

impl<'a> From<&'a DiscourseConfig> for DiscourseListEntry<'a> {
    fn from(discourse: &'a DiscourseConfig) -> Self {
        Self {
            name: &discourse.name,
            baseurl: &discourse.baseurl,
            fullname: discourse.fullname.as_deref(),
            api_username: discourse.api_username.as_deref(),
            tags: discourse.tags.as_deref(),
            changelog_topic_id: discourse.changelog_topic_id,
            ssh_host: discourse.ssh_host.as_deref(),
            docker_rootless: discourse.docker_rootless,
        }
    }
}

pub fn list_tidy(config_path: &Path, config: &mut Config) -> Result<()> {
    // Capture missing fields based on the loaded config *before* we insert placeholders.
    // Note: `DiscourseConfig` deserializers treat empty strings/0 as None for some fields.
    let mut missing_report: HashMap<String, Vec<&'static str>> = HashMap::new();
    for d in &config.discourse {
        let mut missing = Vec::new();
        if d.baseurl.trim().is_empty() {
            missing.push("baseurl");
        }
        if d.apikey.is_none() {
            missing.push("apikey");
        }
        if d.api_username.is_none() {
            missing.push("api_username");
        }
        if d.tags.is_none() {
            missing.push("tags");
        }
        if d.ssh_host.is_none() {
            missing.push("ssh_host");
        }
        if d.changelog_topic_id.is_none() {
            missing.push("changelog_topic_id");
        }
        if !missing.is_empty() {
            missing_report.insert(d.name.clone(), missing);
        }
    }

    // Discover missing fullnames in parallel rather than one URL at a time.
    let urls_to_discover: Vec<String> = config
        .discourse
        .iter()
        .filter(|d| d.fullname.is_none() && !d.baseurl.trim().is_empty())
        .map(|d| d.baseurl.clone())
        .collect();
    let fullnames = fetch_fullnames(&urls_to_discover);
    let mut fn_idx = 0;
    for d in &mut config.discourse {
        if d.apikey.is_none() {
            d.apikey = Some("".to_string());
        }
        if d.api_username.is_none() {
            d.api_username = Some("".to_string());
        }
        if d.tags.is_none() {
            d.tags = Some(Vec::new());
        }
        if d.changelog_topic_id.is_none() {
            d.changelog_topic_id = Some(0);
        }
        if d.ssh_host.is_none() {
            d.ssh_host = Some("".to_string());
        }
        if d.fullname.is_none() && !d.baseurl.trim().is_empty() {
            d.fullname = fullnames.get(fn_idx).cloned().flatten();
            fn_idx += 1;
        }
    }

    // Sort ascending alphanumeric by name (case-insensitive, with a stable tie-break).
    config.discourse.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });

    save_config(config_path, config)?;

    // Print missing fields per discourse.
    for d in &config.discourse {
        if let Some(fields) = missing_report.get(&d.name) {
            println!("{}: missing {}", d.name, fields.join(", "));
        }
    }

    Ok(())
}

pub fn list_discourses(
    config: &Config,
    format: OutputFormat,
    tags: Option<&str>,
    open: bool,
    verbose: bool,
) -> Result<()> {
    let filter = tags.map(parse_tags).unwrap_or_default();
    let matches_filter = |disc: &DiscourseConfig| {
        if filter.is_empty() {
            return true;
        }
        let disc_tags = disc.tags.as_ref().map(|t| {
            t.iter()
                .map(|tag| tag.to_ascii_lowercase())
                .collect::<Vec<_>>()
        });
        let Some(disc_tags) = disc_tags else {
            return false;
        };
        filter.iter().any(|tag| {
            let tag = tag.to_ascii_lowercase();
            disc_tags.iter().any(|t| t == &tag)
        })
    };

    let filtered: Vec<_> = config
        .discourse
        .iter()
        .filter(|d| matches_filter(d))
        .collect();

    if open {
        open_discourse_urls(&filtered)?;
    }

    match format {
        OutputFormat::Text => {
            if filtered.is_empty() && !verbose {
                println!("No discourses found.");
                return Ok(());
            }
            for d in filtered.iter().copied() {
                let fullname = d.fullname.as_deref().unwrap_or("");
                if fullname.is_empty() {
                    println!("{} - {}", d.name, d.baseurl);
                } else {
                    println!("{} - {} - {}", d.name, fullname, d.baseurl);
                }
            }
        }
        OutputFormat::Markdown => {
            for d in filtered.iter().copied() {
                let fullname = d.fullname.as_deref().unwrap_or("");
                if fullname.is_empty() {
                    println!("- {} ({})", d.name, d.baseurl);
                } else {
                    println!("- {} ({}) - {}", d.name, fullname, d.baseurl);
                }
            }
        }
        OutputFormat::MarkdownTable => {
            println!("| Name | Full Name | Base URL |");
            println!("| --- | --- | --- |");
            for d in filtered.iter().copied() {
                let fullname = d.fullname.as_deref().unwrap_or("");
                println!("| {} | {} | {} |", d.name, fullname, d.baseurl);
            }
        }
        OutputFormat::Json => {
            let entries: Vec<DiscourseListEntry<'_>> =
                filtered.iter().copied().map(Into::into).collect();
            let raw = serde_json::to_string_pretty(&entries)?;
            println!("{}", raw);
        }
        OutputFormat::Yaml => {
            let entries: Vec<DiscourseListEntry<'_>> =
                filtered.iter().copied().map(Into::into).collect();
            let raw = serde_yaml::to_string(&entries)?;
            println!("{}", raw);
        }
        OutputFormat::Csv => {
            let mut writer = csv::Writer::from_writer(io::stdout());
            writer.write_record(["name", "fullname", "baseurl", "tags"])?;
            for d in filtered.iter().copied() {
                let tags = d.tags.as_ref().map(|t| t.join(";")).unwrap_or_default();
                let fullname = d.fullname.as_deref().unwrap_or("");
                writer.write_record([d.name.as_str(), fullname, d.baseurl.as_str(), &tags])?;
            }
            writer.flush()?;
        }
        OutputFormat::Urls => {
            for d in filtered.iter().copied() {
                println!("{}", d.baseurl);
            }
        }
    }
    Ok(())
}

fn open_discourse_urls(discourses: &[&DiscourseConfig]) -> Result<()> {
    for discourse in discourses {
        open_url(&discourse.baseurl)
            .with_context(|| format!("opening browser for '{}'", discourse.baseurl))?;
    }
    Ok(())
}

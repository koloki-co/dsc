// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::commands::common::fetch_fullname_from_url;
use crate::config::{Config, DiscourseConfig};
use anyhow::Result;
use std::io::{self, Write};

pub fn add_discourses(config: &mut Config, names: &str, interactive: bool) -> Result<()> {
    let entries = names
        .split(',')
        .map(|name| name.trim())
        .filter(|name| !name.is_empty());
    for name in entries {
        if config.discourse.iter().any(|d| d.name == name) {
            continue;
        }
        let mut entry = DiscourseConfig {
            name: name.to_string(),
            ..DiscourseConfig::default()
        };

        if !interactive {
            entry.apikey = Some("".to_string());
            entry.api_username = Some("".to_string());
            entry.tags = Some(Vec::new());
            entry.changelog_topic_id = Some(0);
            entry.ssh_host = Some("".to_string());
            entry.docker_rootless = Some(false);
        }
        if interactive {
            entry.baseurl = prompt("Base URL")?;
            entry.apikey = prompt_optional("API key")?;
            entry.api_username = prompt_optional("API username")?;
            let tags = prompt_optional("Tags (comma-separated)")?;
            entry.tags = tags.map(|t| {
                t.split(',')
                    .map(|tag| tag.trim().to_string())
                    .filter(|tag| !tag.is_empty())
                    .collect::<Vec<_>>()
            });
            entry.ssh_host = prompt_optional("SSH host (for remote updates)")?;
            let changelog_id_str = prompt_optional("Changelog topic ID (for update posts)")?;
            if let Some(id_str) = changelog_id_str
                && let Ok(id) = id_str.parse::<u64>()
            {
                entry.changelog_topic_id = Some(id);
            }
            if !entry.baseurl.trim().is_empty() {
                entry.fullname = fetch_fullname_from_url(&entry.baseurl);
            }
        }
        config.discourse.push(entry);
    }
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{}: ", label);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn prompt_optional(label: &str) -> Result<Option<String>> {
    let value = prompt(label)?;
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

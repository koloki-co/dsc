// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use anyhow::Result;

pub fn api_key_list(config: &Config, discourse_name: &str, format: ListFormat) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let keys = client.list_api_keys()?;

    match format {
        ListFormat::Text => {
            if keys.is_empty() {
                println!("No API keys found.");
                return Ok(());
            }
            let desc_width = keys
                .iter()
                .map(|k| k.description.as_deref().unwrap_or("-").len())
                .max()
                .unwrap_or(0)
                .max(11);
            for k in &keys {
                let desc = k.description.as_deref().unwrap_or("-");
                let user = k.username.as_deref().unwrap_or("(all-users)");
                let last = k.last_used_at.as_deref().unwrap_or("never");
                let status = if k.revoked_at.is_some() {
                    "revoked"
                } else {
                    "active"
                };
                println!(
                    "id:{:<5} {:<width$}  user:{:<20}  last:{:<25}  {}",
                    k.id,
                    desc,
                    user,
                    last,
                    status,
                    width = desc_width
                );
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&keys)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&keys)?),
    }
    Ok(())
}

pub fn api_key_create(
    config: &Config,
    discourse_name: &str,
    description: &str,
    username: Option<&str>,
    format: ListFormat,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!(
            "[dry-run] {}: would create api key \"{}\" for user {}",
            discourse.name,
            description,
            username.unwrap_or("(all-users)")
        );
        return Ok(());
    }

    let created = client.create_api_key(description, username)?;

    match format {
        ListFormat::Text => {
            println!("New API key (shown only this once — copy it now):");
            println!();
            println!("  {}", created.key);
            println!();
            println!("id:          {}", created.id);
            if let Some(d) = &created.description {
                println!("description: {}", d);
            }
            println!(
                "username:    {}",
                created.username.as_deref().unwrap_or("(all-users)")
            );
            if let Some(c) = &created.created_at {
                println!("created_at:  {}", c);
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&created)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&created)?),
    }
    Ok(())
}

pub fn api_key_revoke(
    config: &Config,
    discourse_name: &str,
    key_id: u64,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!(
            "[dry-run] {}: would revoke api key id:{}",
            discourse.name, key_id
        );
        return Ok(());
    }

    client.revoke_api_key(key_id)?;
    println!("Revoked api key id:{}", key_id);
    Ok(())
}

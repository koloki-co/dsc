// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::api::GroupSummary;
use crate::cli::{ListFormat, StructuredFormat};
use crate::commands::common::{ensure_api_credentials, not_found, parse_emails, select_discourse};
use crate::config::Config;
use crate::utils::{normalize_baseurl, slugify};
use anyhow::{Context, Result, anyhow};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

pub fn group_list(
    config: &Config,
    discourse_name: &str,
    format: ListFormat,
    verbose: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let groups = client.fetch_groups()?;
    match format {
        ListFormat::Text => {
            if groups.is_empty() && !verbose {
                println!("No groups found.");
                return Ok(());
            }
            for group in groups {
                let full_name = group.full_name.unwrap_or_else(|| "-".to_string());
                println!("{} - {} ({})", group.id, group.name, full_name);
            }
        }
        ListFormat::Json => {
            let raw = serde_json::to_string_pretty(&groups)?;
            println!("{}", raw);
        }
        ListFormat::Yaml => {
            let raw = serde_yaml::to_string(&groups)?;
            println!("{}", raw);
        }
    }
    Ok(())
}

pub fn group_info(
    config: &Config,
    discourse_name: &str,
    group_id: u64,
    format: StructuredFormat,
    with_defaults: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    // Try the numeric ID path first; only list all groups for a name
    // fallback if the ID route returns 404.
    let mut group = match client.fetch_group_detail_by_id(group_id)? {
        Some(detail) => detail,
        None => {
            let group_summary = find_group_summary(&client, group_id)?;
            client.fetch_group_detail(group_summary.id, Some(&group_summary.name))?
        }
    };
    if !with_defaults {
        group.clear_notification_defaults();
    }
    match format {
        StructuredFormat::Json => {
            let raw = serde_json::to_string_pretty(&group)?;
            println!("{}", raw);
        }
        StructuredFormat::Yaml => {
            let raw = serde_yaml::to_string(&group)?;
            println!("{}", raw);
        }
    }
    Ok(())
}

pub fn group_members(
    config: &Config,
    discourse_name: &str,
    group_id: u64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let members = match client.fetch_group_members_by_id(group_id)? {
        Some(members) => members,
        None => {
            let group_summary = find_group_summary(&client, group_id)?;
            client.fetch_group_members(group_summary.id, Some(&group_summary.name))?
        }
    };
    match format {
        ListFormat::Text => {
            if members.is_empty() {
                println!("No group members found.");
                return Ok(());
            }
            for member in members {
                let name = member.name.unwrap_or_else(|| "-".to_string());
                println!("{} - {} ({})", member.id, member.username, name);
            }
        }
        ListFormat::Json => {
            let raw = serde_json::to_string_pretty(&members)?;
            println!("{}", raw);
        }
        ListFormat::Yaml => {
            let raw = serde_yaml::to_string(&members)?;
            println!("{}", raw);
        }
    }
    Ok(())
}

pub fn group_copy(
    config: &Config,
    source: &str,
    target: Option<&str>,
    group_id: u64,
    dry_run: bool,
) -> Result<()> {
    let source_discourse = select_discourse(config, Some(source))?;
    let target_discourse_name = target.unwrap_or(source);
    let target_discourse = select_discourse(config, Some(target_discourse_name))?;

    ensure_api_credentials(source_discourse)?;
    ensure_api_credentials(target_discourse)?;

    let source_client = DiscourseClient::new(source_discourse)?;
    let mut group = match source_client.fetch_group_detail_by_id(group_id)? {
        Some(detail) => detail,
        None => {
            let group_summary = find_group_summary(&source_client, group_id)?;
            source_client.fetch_group_detail(group_summary.id, Some(&group_summary.name))?
        }
    };
    group.name = format!("{}-copy", slugify(&group.name));
    if let Some(full_name) = group.full_name.clone() {
        group.full_name = Some(format!("Copy of {}", full_name));
    }

    if dry_run {
        println!(
            "[dry-run] would create group \"{}\" on {}",
            group.name, target_discourse.name
        );
        return Ok(());
    }

    let target_client = DiscourseClient::new(target_discourse)?;
    let new_id = target_client.create_group(&group)?;
    let url = format!(
        "{}/g/{}/{}",
        normalize_baseurl(&target_discourse.baseurl),
        group.name,
        new_id
    );
    println!("{}", url);
    Ok(())
}

/// Try the numeric ID route for group detail first. Only list all groups
/// to recover a name for the fallback path if the ID route 404s.
fn find_group_summary(client: &DiscourseClient, group_id: u64) -> Result<GroupSummary> {
    let groups = client.fetch_groups()?;
    groups
        .into_iter()
        .find(|item| item.id == group_id)
        .ok_or_else(|| not_found("group", group_id))
}

pub fn group_add(
    config: &Config,
    discourse_name: &str,
    group_id: u64,
    local_path: Option<&Path>,
    notify: bool,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let raw = read_email_source(local_path)?;
    let emails = parse_emails(&raw);
    if emails.is_empty() {
        return Err(anyhow!("no email addresses found in input"));
    }

    if dry_run {
        println!(
            "[dry-run] {}: would add {} email(s) to group {} (notify={})",
            discourse.name,
            emails.len(),
            group_id,
            notify
        );
        for email in &emails {
            println!("  {}", email);
        }
        return Ok(());
    }

    let outcome = client.add_group_members_by_email(group_id, &emails, notify)?;
    println!(
        "Requested {} email(s); Discourse reported {} username(s) added",
        emails.len(),
        outcome.added_usernames.len()
    );
    if !outcome.added_usernames.is_empty() {
        for username in &outcome.added_usernames {
            println!("  + {}", username);
        }
    }
    if !outcome.errors.is_empty() {
        eprintln!("Server notes:");
        for msg in &outcome.errors {
            eprintln!("  - {}", msg);
        }
    }
    Ok(())
}

fn read_email_source(local_path: Option<&Path>) -> Result<String> {
    let from_stdin = match local_path {
        None => true,
        Some(p) => p.as_os_str() == "-",
    };
    if from_stdin {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .context("reading emails from stdin")?;
        Ok(buf)
    } else {
        let path = local_path.unwrap();
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
    }
}

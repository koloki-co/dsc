// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::commands::common::{ensure_api_credentials, parse_emails, select_discourse};
use crate::config::Config;
use anyhow::{Context, Result, anyhow};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

pub fn invite_one(
    config: &Config,
    discourse_name: &str,
    email: &str,
    group_ids: &[u64],
    topic_id: Option<u64>,
    message: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let email = email.trim();
    if email.is_empty() || !email.contains('@') {
        return Err(anyhow!("invalid email: {:?}", email));
    }

    if dry_run {
        println!(
            "[dry-run] {}: would invite {}{}{}{}",
            discourse.name,
            email,
            describe_groups(group_ids),
            topic_id
                .map(|t| format!(" → topic {}", t))
                .unwrap_or_default(),
            message
                .filter(|m| !m.trim().is_empty())
                .map(|m| format!(" with message ({} chars)", m.len()))
                .unwrap_or_default()
        );
        return Ok(());
    }

    let result = client.create_invite(email, group_ids, topic_id, message)?;
    if let Some(link) = &result.link {
        println!("Invited {} (id:{}) — {}", email, result.id, link);
    } else {
        println!("Invited {} (id:{})", email, result.id);
    }
    Ok(())
}

pub fn invite_bulk(
    config: &Config,
    discourse_name: &str,
    local_path: Option<&Path>,
    group_ids: &[u64],
    topic_id: Option<u64>,
    message: Option<&str>,
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
            "[dry-run] {}: would invite {} email(s){}{}",
            discourse.name,
            emails.len(),
            describe_groups(group_ids),
            topic_id
                .map(|t| format!(" → topic {}", t))
                .unwrap_or_default(),
        );
        for email in &emails {
            println!("  {}", email);
        }
        return Ok(());
    }

    let bar = ProgressBar::new(emails.len() as u64);
    bar.set_style(
        ProgressStyle::with_template("{bar:30} {pos}/{len} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_bar()),
    );

    let mut invited = 0usize;
    let mut failures: Vec<(String, String)> = Vec::new();
    for email in &emails {
        bar.set_message(email.clone());
        match client.create_invite(email, group_ids, topic_id, message) {
            Ok(_) => {
                invited += 1;
                bar.println(format!("invite  {}", email));
            }
            Err(err) => {
                bar.println(format!("FAIL    {} — {}", email, err));
                failures.push((email.clone(), err.to_string()));
            }
        }
        bar.inc(1);
    }
    bar.finish_and_clear();

    if !failures.is_empty() {
        eprintln!("Invite failures:");
        for (email, reason) in &failures {
            eprintln!("- {} => {}", email, reason);
        }
    }
    println!(
        "Invite bulk summary: invited={}, failed={}",
        invited,
        failures.len()
    );
    if !failures.is_empty() {
        return Err(anyhow!(
            "{} invites failed; see failure summary above",
            failures.len()
        ));
    }
    Ok(())
}

fn describe_groups(group_ids: &[u64]) -> String {
    if group_ids.is_empty() {
        String::new()
    } else {
        format!(
            " (groups: {})",
            group_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
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

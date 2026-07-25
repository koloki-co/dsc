// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use anyhow::{Context, Result, anyhow};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

pub fn pm_send(
    config: &Config,
    discourse_name: &str,
    recipients_csv: &str,
    title: &str,
    local_path: Option<&Path>,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if title.trim().is_empty() {
        return Err(anyhow!("PM title is empty"));
    }
    let recipients = parse_recipients(recipients_csv);
    if recipients.is_empty() {
        return Err(anyhow!("no recipients supplied"));
    }
    let raw = read_body(local_path)?;
    if raw.trim().is_empty() {
        return Err(anyhow!("PM body is empty"));
    }

    if dry_run {
        println!(
            "[dry-run] {}: would send PM titled \"{}\" to {} ({} bytes of body)",
            discourse.name,
            title,
            recipients.join(", "),
            raw.len()
        );
        return Ok(());
    }

    let topic_id = client.create_private_message(&recipients, title, &raw)?;
    println!(
        "Sent PM \"{}\" to {} (topic id {})",
        title,
        recipients.join(", "),
        topic_id
    );
    Ok(())
}

pub fn pm_list(
    config: &Config,
    discourse_name: &str,
    username: &str,
    direction: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let topics = client.list_private_messages(username, direction)?;

    match format {
        ListFormat::Text => {
            if topics.is_empty() {
                println!("No PMs found in {}.", direction);
                return Ok(());
            }
            let id_width = topics
                .iter()
                .map(|t| t.id.to_string().len())
                .max()
                .unwrap_or(2);
            for t in &topics {
                let title = t.title.as_deref().unwrap_or("(untitled)");
                let last = t.last_posted_at.as_deref().unwrap_or("");
                let from = t.last_poster_username.as_deref().unwrap_or("");
                println!(
                    "{:>width$}  {}  [{} by {}]",
                    t.id,
                    title,
                    last,
                    from,
                    width = id_width
                );
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&topics)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&topics)?),
    }
    Ok(())
}

fn parse_recipients(input: &str) -> Vec<String> {
    input
        .split([',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
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
            .context("reading PM body from stdin")?;
        Ok(buf)
    } else {
        let path = local_path.unwrap();
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_recipients;

    #[test]
    fn parses_csv() {
        let got = parse_recipients("alice,bob,charlie");
        assert_eq!(got, vec!["alice", "bob", "charlie"]);
    }

    #[test]
    fn trims_and_drops_blanks() {
        let got = parse_recipients("alice, bob , , charlie");
        assert_eq!(got, vec!["alice", "bob", "charlie"]);
    }

    #[test]
    fn accepts_semicolons() {
        let got = parse_recipients("alice;bob;charlie");
        assert_eq!(got, vec!["alice", "bob", "charlie"]);
    }

    #[test]
    fn handles_single_recipient() {
        let got = parse_recipients("alice");
        assert_eq!(got, vec!["alice"]);
    }

    #[test]
    fn empty_input_yields_empty_list() {
        let got = parse_recipients("");
        assert!(got.is_empty());
    }
}

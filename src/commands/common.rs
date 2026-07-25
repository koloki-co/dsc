// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::config::{Config, DiscourseConfig, find_discourse};
use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use std::fmt::Display;
use std::process::Command;

/// Emit a single command result honouring `--format`. Text mode prints the
/// human-readable `text`; json/yaml serialise `value`. Lets the otherwise
/// single-value commands (`setting get`, `theme duplicate`, `topic reply`/
/// `new`, …) participate in scripting pipelines without bespoke per-command
/// formatting code.
pub fn emit_result<T: Serialize>(format: ListFormat, value: &T, text: &str) -> Result<()> {
    match format {
        ListFormat::Text => println!("{}", text),
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(value)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(value)?),
    }
    Ok(())
}

pub fn select_discourse<'a>(
    config: &'a Config,
    discourse_name: Option<&str>,
) -> Result<&'a DiscourseConfig> {
    if let Some(name) = discourse_name {
        return find_discourse(config, name).ok_or_else(|| not_found("discourse", name));
    }
    Err(anyhow!("missing discourse for command"))
}

pub fn ensure_api_credentials(discourse: &DiscourseConfig) -> Result<()> {
    let apikey = discourse.apikey.as_deref().unwrap_or("").trim();
    let api_username = discourse.api_username.as_deref().unwrap_or("").trim();
    if apikey.is_empty() || api_username.is_empty() {
        return Err(missing_config(
            "apikey/api_username",
            &format!("discourse {}", discourse.name),
            "apikey and api_username",
        ));
    }
    Ok(())
}

pub fn not_found(resource: &str, identifier: impl Display) -> anyhow::Error {
    anyhow!("{} not found: {}", resource, identifier)
}

pub fn missing_config(field: &str, resource: &str, hint: &str) -> anyhow::Error {
    anyhow!(
        "missing {} for {}; please set {} or check your config",
        field,
        resource,
        hint
    )
}

pub fn parse_tags(raw: &str) -> Vec<String> {
    raw.split([';', ','])
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect()
}

pub fn fetch_fullname_from_url(baseurl: &str) -> Option<String> {
    let temp = DiscourseConfig {
        name: "temp".to_string(),
        baseurl: baseurl.to_string(),
        ..DiscourseConfig::default()
    };
    let client = match DiscourseClient::new(&temp) {
        Ok(client) => client,
        Err(err) => {
            eprintln!("Failed to query site title for {}: {}", baseurl, err);
            return None;
        }
    };
    match client.fetch_site_title() {
        Ok(title) => {
            let title = title.trim().to_string();
            if title.is_empty() { None } else { Some(title) }
        }
        Err(err) => {
            eprintln!("Failed to query site title for {}: {}", baseurl, err);
            None
        }
    }
}

pub fn open_url(url: &str) -> Result<()> {
    if url.trim().is_empty() {
        return Err(anyhow!("cannot open empty base URL"));
    }

    let mut cmd = if let Ok(opener) = std::env::var("DSC_BROWSER_OPENER") {
        let mut cmd = Command::new(opener);
        cmd.arg(url);
        cmd
    } else if cfg!(target_os = "macos") {
        let mut cmd = Command::new("open");
        cmd.arg(url);
        cmd
    } else if cfg!(target_os = "windows") {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", url]);
        cmd
    } else {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        cmd
    };

    let status = cmd.status().context("failed to launch browser opener")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("browser opener exited with status {}", status))
    }
}

/// Parse one-email-per-line input. Ignores blank lines, `#` comments
/// (full-line and inline), and leading/trailing whitespace. De-duplicates
/// while preserving the first-seen order, lowercasing as it goes.
pub fn parse_emails(input: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in input.lines() {
        let stripped = match line.find('#') {
            Some(idx) => &line[..idx],
            None => line,
        };
        let trimmed = stripped.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.contains('@') {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            out.push(lower);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::parse_emails;

    #[test]
    fn parses_one_per_line() {
        let got = parse_emails("alice@example.com\nbob@example.com\n");
        assert_eq!(got, vec!["alice@example.com", "bob@example.com"]);
    }

    #[test]
    fn skips_blanks_and_comments() {
        let got = parse_emails(
            "\
# onboarding list
alice@example.com

# new hires below
bob@example.com # bob in marketing
",
        );
        assert_eq!(got, vec!["alice@example.com", "bob@example.com"]);
    }

    #[test]
    fn dedupes_preserving_first_seen_order() {
        let got = parse_emails("alice@example.com\nbob@example.com\nALICE@example.com");
        assert_eq!(got, vec!["alice@example.com", "bob@example.com"]);
    }

    #[test]
    fn rejects_lines_without_at() {
        let got = parse_emails("not_an_email\nalice@example.com");
        assert_eq!(got, vec!["alice@example.com"]);
    }

    #[test]
    fn lowercases_emails() {
        let got = parse_emails("Alice@Example.com");
        assert_eq!(got, vec!["alice@example.com"]);
    }

    #[test]
    fn handles_crlf_line_endings() {
        let got = parse_emails("alice@example.com\r\nbob@example.com\r\n");
        assert_eq!(got, vec!["alice@example.com", "bob@example.com"]);
    }
}

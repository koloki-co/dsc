// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! `dsc webhook list|create|delete|ping` — basic outbound webhook
//! administration (`/admin/api/web_hooks.json`).

use crate::api::{DiscourseClient, WebhookSummary, redact_webhook_url};
use crate::cli::ListFormat;
use crate::commands::common::{emit_result, ensure_api_credentials, select_discourse};
use crate::config::Config;
use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use std::io::{self, Read};

fn bool_str(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}

pub fn webhook_list(config: &Config, discourse_name: &str, format: ListFormat) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let hooks = client.list_webhooks()?;

    match format {
        ListFormat::Text => {
            if hooks.is_empty() {
                println!("No webhooks found.");
                return Ok(());
            }
            let url_width = hooks
                .iter()
                .map(|h| webhook_url(h).len())
                .max()
                .unwrap_or(0)
                .max(11);
            for h in &hooks {
                let url = webhook_url(h);
                let active = match h.active {
                    Some(true) => "active",
                    Some(false) => "inactive",
                    None => "unknown",
                };
                let events = match h.wildcard_web_hook {
                    Some(true) => "all",
                    Some(false) => "selected",
                    None => "-",
                };
                println!(
                    "id:{:<5} {:<width$}  events:{:<9}  {}",
                    h.id,
                    url,
                    events,
                    active,
                    width = url_width
                );
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&hooks)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&hooks)?),
    }
    Ok(())
}

pub struct WebhookCreateOptions<'a> {
    pub payload_url: &'a str,
    pub content_type: u8,
    pub secret_from_stdin: bool,
    pub active: bool,
    pub verify_certificate: bool,
    pub format: ListFormat,
    pub dry_run: bool,
}

pub fn webhook_create(
    config: &Config,
    discourse_name: &str,
    options: WebhookCreateOptions<'_>,
) -> Result<()> {
    let secret = read_webhook_secret(options.secret_from_stdin)?;
    validate_create_input(options.payload_url, secret.as_deref())?;
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let event_type_ids = client.default_webhook_event_type_ids()?;

    if options.dry_run {
        println!(
            "{}",
            create_plan(
                &discourse.name,
                options.payload_url,
                options.content_type,
                secret.is_some(),
                options.active,
                options.verify_certificate,
                &event_type_ids
            )
        );
        return Ok(());
    }

    let created = client.create_webhook(
        options.payload_url,
        options.content_type,
        secret.as_deref(),
        options.active,
        options.verify_certificate,
        &event_type_ids,
    )?;

    match options.format {
        ListFormat::Text => {
            println!("Created webhook id:{}", created.id);
            println!("payload_url:        {}", webhook_url(&created));
            println!("active:             {}", bool_str(created.active));
            println!(
                "wildcard_web_hook:  {}",
                bool_str(created.wildcard_web_hook)
            );
            println!(
                "verify_certificate: {}",
                bool_str(created.verify_certificate)
            );
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&created)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&created)?),
    }
    Ok(())
}

pub fn webhook_delete(
    config: &Config,
    discourse_name: &str,
    webhook_id: u64,
    format: ListFormat,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!(
            "[dry-run] {}: would delete webhook id:{}",
            discourse.name, webhook_id
        );
        return Ok(());
    }

    client.delete_webhook(webhook_id)?;
    emit_action(format, webhook_id, "deleted")
}

/// Enqueue a test delivery for a webhook. Honours `--dry-run` like the other
/// state-changing webhook subcommands, since a ping enqueues a real delivery
/// job on the forum (mirrors `dsc notification read`'s guard on its
/// side-effecting mark-read calls).
pub fn webhook_ping(
    config: &Config,
    discourse_name: &str,
    webhook_id: u64,
    format: ListFormat,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!(
            "[dry-run] {}: would ping webhook id:{}",
            discourse.name, webhook_id
        );
        return Ok(());
    }

    client.ping_webhook(webhook_id)?;
    emit_action(format, webhook_id, "pinged")
}

fn webhook_url(webhook: &WebhookSummary) -> &str {
    if webhook.payload_url.is_empty() {
        "-"
    } else {
        &webhook.payload_url
    }
}

fn validate_create_input(payload_url: &str, secret: Option<&str>) -> Result<()> {
    let parsed = reqwest::Url::parse(payload_url)
        .map_err(|_| anyhow!("webhook payload URL must be an absolute HTTP(S) URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(anyhow!(
            "webhook payload URL must be an absolute HTTP(S) URL"
        ));
    }
    if let Some(secret) = secret
        && (secret.trim().is_empty() || secret.chars().count() < 12)
    {
        return Err(anyhow!(
            "webhook secret must contain at least 12 characters and cannot be blank"
        ));
    }
    Ok(())
}

fn read_webhook_secret(secret_from_stdin: bool) -> Result<Option<String>> {
    if !secret_from_stdin {
        return Ok(None);
    }
    let mut secret = String::new();
    io::stdin()
        .read_to_string(&mut secret)
        .context("reading webhook secret from stdin")?;
    Ok(Some(normalize_webhook_secret(&secret)?))
}

fn normalize_webhook_secret(secret: &str) -> Result<String> {
    let secret = secret.trim_end_matches(['\r', '\n']).to_string();
    if secret.is_empty() {
        return Err(anyhow!("--secret-stdin set but stdin was empty"));
    }
    Ok(secret)
}

fn create_plan(
    discourse_name: &str,
    payload_url: &str,
    content_type: u8,
    secret_provided: bool,
    active: bool,
    verify_certificate: bool,
    event_type_ids: &[u64],
) -> String {
    let content_type = match content_type {
        1 => "json",
        2 => "form",
        _ => "unknown",
    };
    let event_type_ids = event_type_ids
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "[dry-run] {discourse_name}: would create webhook for {} (content_type:{content_type}, wildcard:true, event_types:{event_type_ids}, active:{active}, verify_certificate:{verify_certificate}, secret:{})",
        redact_webhook_url(payload_url),
        if secret_provided {
            "provided"
        } else {
            "omitted"
        }
    )
}

#[derive(Serialize)]
struct WebhookActionResult {
    id: u64,
    action: &'static str,
}

fn emit_action(format: ListFormat, webhook_id: u64, action: &'static str) -> Result<()> {
    let result = WebhookActionResult {
        id: webhook_id,
        action,
    };
    let text = match action {
        "deleted" => format!("Deleted webhook id:{webhook_id}"),
        "pinged" => format!("Pinged webhook id:{webhook_id} (test delivery enqueued)"),
        _ => format!("Webhook id:{webhook_id}: {action}"),
    };
    emit_result(format, &result, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_input_requires_an_absolute_http_url() {
        assert!(validate_create_input("https://hooks.example.test/incoming", None).is_ok());
        assert!(validate_create_input("ftp://hooks.example.test/incoming", None).is_err());
        assert!(validate_create_input("/relative", None).is_err());
        assert!(validate_create_input("https://", None).is_err());
    }

    #[test]
    fn create_input_rejects_short_or_blank_secrets() {
        assert!(validate_create_input("https://hooks.example.test", Some("short")).is_err());
        assert!(validate_create_input("https://hooks.example.test", Some("            ")).is_err());
        assert!(validate_create_input("https://hooks.example.test", Some("0123456789ab")).is_ok());
    }

    #[test]
    fn stdin_secret_strips_only_its_line_ending() {
        assert_eq!(
            normalize_webhook_secret("0123456789ab\r\n").unwrap(),
            "0123456789ab"
        );
        assert_eq!(
            normalize_webhook_secret("  secret value  ").unwrap(),
            "  secret value  "
        );
        assert!(normalize_webhook_secret("\n").is_err());
    }

    #[test]
    fn dry_run_plan_redacts_credentials_and_secret() {
        let plan = create_plan(
            "forum",
            "https://user:url-canary@example.test/hook",
            1,
            true,
            true,
            true,
            &[201, 202, 203, 204],
        );
        assert!(plan.contains("https://***@example.test/hook"));
        assert!(plan.contains("secret:provided"));
        assert!(plan.contains("event_types:201,202,203,204"));
        assert!(!plan.contains("url-canary"));
        assert!(!plan.contains("secret-canary"));
    }
}

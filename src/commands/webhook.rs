// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! `dsc webhook list|create|delete|ping` — basic outbound webhook
//! administration (`/admin/api/web_hooks.json`).

use crate::api::{DiscourseClient, WebhookSummary, redact_webhook_url};
use crate::cli::ListFormat;
use crate::commands::common::{emit_result, ensure_api_credentials, select_discourse};
use crate::config::Config;
use anyhow::{Result, anyhow};
use serde::Serialize;

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

pub fn webhook_create(
    config: &Config,
    discourse_name: &str,
    payload_url: &str,
    content_type: u8,
    secret: Option<&str>,
    active: bool,
    verify_certificate: bool,
    format: ListFormat,
    dry_run: bool,
) -> Result<()> {
    validate_create_input(payload_url, secret)?;
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;

    if dry_run {
        println!(
            "{}",
            create_plan(
                &discourse.name,
                payload_url,
                content_type,
                secret,
                active,
                verify_certificate
            )
        );
        return Ok(());
    }

    let client = DiscourseClient::new(discourse)?;
    let created = client.create_webhook(
        payload_url,
        content_type,
        secret,
        active,
        verify_certificate,
    )?;

    match format {
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
        return Err(anyhow!("webhook payload URL must be an absolute HTTP(S) URL"));
    }
    if let Some(secret) = secret
        && (secret.trim().is_empty() || secret.len() < 12)
    {
        return Err(anyhow!(
            "webhook secret must contain at least 12 non-whitespace characters"
        ));
    }
    Ok(())
}

fn create_plan(
    discourse_name: &str,
    payload_url: &str,
    content_type: u8,
    secret: Option<&str>,
    active: bool,
    verify_certificate: bool,
) -> String {
    let content_type = match content_type {
        1 => "json",
        2 => "form",
        _ => "unknown",
    };
    format!(
        "[dry-run] {discourse_name}: would create webhook for {} (content_type:{content_type}, wildcard:true, event_types:Discourse defaults, active:{active}, verify_certificate:{verify_certificate}, secret:{})",
        redact_webhook_url(payload_url),
        if secret.is_some() { "provided" } else { "omitted" }
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
        assert!(validate_create_input("https:///missing-host", None).is_err());
    }

    #[test]
    fn create_input_rejects_short_or_blank_secrets() {
        assert!(validate_create_input("https://hooks.example.test", Some("short")).is_err());
        assert!(validate_create_input("https://hooks.example.test", Some("            ")).is_err());
        assert!(validate_create_input("https://hooks.example.test", Some("0123456789ab")).is_ok());
    }

    #[test]
    fn dry_run_plan_redacts_credentials_and_secret() {
        let plan = create_plan(
            "forum",
            "https://user:url-canary@example.test/hook",
            1,
            Some("secret-canary"),
            true,
            true,
        );
        assert!(plan.contains("https://***@example.test/hook"));
        assert!(plan.contains("secret:provided"));
        assert!(!plan.contains("url-canary"));
        assert!(!plan.contains("secret-canary"));
    }
}

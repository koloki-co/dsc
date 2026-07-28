// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::DiscourseClient;
use super::error::http_error;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const WEBHOOKS_PATH: &str = "/admin/api/web_hooks.json";
const WEBHOOK_PAGE_SIZE: usize = 50;
const MAX_WEBHOOK_PAGES: usize = 1_000;

/// A webhook event type returned by Discourse.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct WebhookEventType {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub group: String,
}

/// A tag scope returned by Discourse for a webhook.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct WebhookTag {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
}

/// A safe representation of a webhook for CLI output.
///
/// Discourse includes the signing secret in its admin serializer. Keep this
/// type explicit so a future server field cannot leak through JSON or YAML.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct WebhookSummary {
    pub id: u64,
    #[serde(default)]
    pub payload_url: String,
    /// Discourse's integer encoding: 1 = `application/json`,
    /// 2 = `application/x-www-form-urlencoded`.
    #[serde(default)]
    pub content_type: Option<u8>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub wildcard_web_hook: Option<bool>,
    #[serde(default)]
    pub verify_certificate: Option<bool>,
    #[serde(default)]
    pub last_delivery_status: Option<u8>,
    #[serde(default)]
    pub category_ids: Vec<u64>,
    #[serde(default)]
    pub group_ids: Vec<u64>,
    #[serde(default)]
    pub tags: Vec<WebhookTag>,
    #[serde(default)]
    pub web_hook_event_types: Vec<WebhookEventType>,
}

/// The wire representation from Discourse's admin serializer. This is kept
/// private deliberately: its `secret` field must never become CLI output.
#[derive(Debug, Deserialize)]
struct WebhookWire {
    id: u64,
    #[serde(default)]
    payload_url: String,
    #[serde(default)]
    content_type: Option<u8>,
    #[serde(default)]
    active: Option<bool>,
    #[serde(default)]
    wildcard_web_hook: Option<bool>,
    #[serde(default)]
    verify_certificate: Option<bool>,
    #[serde(default)]
    last_delivery_status: Option<u8>,
    #[serde(default)]
    category_ids: Vec<u64>,
    #[serde(default)]
    group_ids: Vec<u64>,
    #[serde(default)]
    tags: Vec<WebhookTag>,
    #[serde(default)]
    web_hook_event_types: Vec<WebhookEventType>,
}

impl From<WebhookWire> for WebhookSummary {
    fn from(webhook: WebhookWire) -> Self {
        Self {
            id: webhook.id,
            payload_url: redact_webhook_url(&webhook.payload_url),
            content_type: webhook.content_type,
            active: webhook.active,
            wildcard_web_hook: webhook.wildcard_web_hook,
            verify_certificate: webhook.verify_certificate,
            last_delivery_status: webhook.last_delivery_status,
            category_ids: webhook.category_ids,
            group_ids: webhook.group_ids,
            tags: webhook.tags,
            web_hook_event_types: webhook.web_hook_event_types,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct WebhookExtras {
    #[serde(default)]
    default_event_types: Vec<u64>,
}

#[derive(Debug, Deserialize)]
struct WebhookPage {
    #[serde(default)]
    web_hooks: Vec<WebhookWire>,
    #[serde(default)]
    extras: WebhookExtras,
    #[serde(default)]
    total_rows_web_hooks: Option<usize>,
}

impl DiscourseClient {
    pub fn list_webhooks(&self) -> Result<Vec<WebhookSummary>> {
        let mut hooks = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut offset = 0;

        for _ in 0..MAX_WEBHOOK_PAGES {
            let page = self.webhook_page(offset)?;
            let page_len = page.web_hooks.len();
            append_webhook_page(&mut hooks, &mut seen_ids, page.web_hooks)?;

            if !has_more_webhook_pages(page_len, page.total_rows_web_hooks, hooks.len()) {
                return Ok(hooks);
            }
            offset = offset
                .checked_add(WEBHOOK_PAGE_SIZE)
                .context("webhook pagination offset overflow")?;
        }

        Err(anyhow!(
            "webhook pagination exceeded {MAX_WEBHOOK_PAGES} pages"
        ))
    }

    fn webhook_page(&self, offset: usize) -> Result<WebhookPage> {
        let response = self.get(&format!("{WEBHOOKS_PATH}?offset={offset}"))?;
        let status = response.status();
        let text = response
            .text()
            .context("reading webhook list response")?;
        if !status.is_success() {
            return Err(http_error("webhook list request", status, &text));
        }
        serde_json::from_str(&text).context("parsing webhook list response")
    }

    /// Create a webhook. `content_type` is Discourse's integer encoding:
    /// 1 = `application/json`, 2 = `application/x-www-form-urlencoded`.
    pub fn create_webhook(
        &self,
        payload_url: &str,
        content_type: u8,
        secret: Option<&str>,
        active: bool,
        verify_certificate: bool,
    ) -> Result<WebhookSummary> {
        let event_type_ids = self.default_webhook_event_type_ids()?;
        let payload = webhook_create_payload(
            payload_url,
            content_type,
            secret,
            active,
            verify_certificate,
            &event_type_ids,
        )?;
        let response =
            self.send_retrying(|| Ok(self.post("/admin/api/web_hooks.json")?.form(&payload)))?;
        let status = response.status();
        let text = response.text().context("reading webhook create response")?;
        if !status.is_success() {
            return Err(http_error("webhook create request", status, &text));
        }
        let value: serde_json::Value =
            serde_json::from_str(&text).context("parsing webhook create response")?;
        let hook_obj = value.get("web_hook").unwrap_or(&value);
        let created: WebhookWire =
            serde_json::from_value(hook_obj.clone()).context("deserialising created webhook")?;
        Ok(created.into())
    }

    fn default_webhook_event_type_ids(&self) -> Result<Vec<u64>> {
        let mut event_type_ids = self.webhook_page(0)?.extras.default_event_types;
        event_type_ids.retain(|id| *id > 0);
        event_type_ids.sort_unstable();
        event_type_ids.dedup();
        if event_type_ids.is_empty() {
            return Err(anyhow!(
                "Discourse returned no default webhook event types; refusing to create a webhook that would receive no events"
            ));
        }
        Ok(event_type_ids)
    }

    pub fn delete_webhook(&self, webhook_id: u64) -> Result<()> {
        let path = format!("/admin/api/web_hooks/{}.json", webhook_id);
        let response = self.send_retrying(|| self.delete_builder(&path))?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            return Err(http_error("webhook delete request", status, &text));
        }
        Ok(())
    }

    /// Enqueue a test delivery. Discourse's ping route likely returns 200
    /// with no meaningful body (or a small ack JSON), so only the status
    /// code is checked — any 2xx counts as success.
    pub fn ping_webhook(&self, webhook_id: u64) -> Result<()> {
        let path = format!("/admin/api/web_hooks/{}/ping.json", webhook_id);
        let response = self.send_retrying(|| self.post(&path))?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            return Err(http_error("webhook ping request", status, &text));
        }
        Ok(())
    }
}

fn append_webhook_page(
    hooks: &mut Vec<WebhookSummary>,
    seen_ids: &mut HashSet<u64>,
    page: Vec<WebhookWire>,
) -> Result<()> {
    for webhook in page {
        if !seen_ids.insert(webhook.id) {
            return Err(anyhow!(
                "webhook pagination returned duplicate webhook id {}",
                webhook.id
            ));
        }
        hooks.push(webhook.into());
    }
    Ok(())
}

fn has_more_webhook_pages(
    page_len: usize,
    total_rows: Option<usize>,
    collected_rows: usize,
) -> bool {
    page_len == WEBHOOK_PAGE_SIZE && total_rows.is_none_or(|total| collected_rows < total)
}

fn webhook_create_payload(
    payload_url: &str,
    content_type: u8,
    secret: Option<&str>,
    active: bool,
    verify_certificate: bool,
    event_type_ids: &[u64],
) -> Result<Vec<(String, String)>> {
    if !matches!(content_type, 1 | 2) {
        return Err(anyhow!("unsupported webhook content type: {content_type}"));
    }
    let mut payload = vec![
        ("web_hook[payload_url]".to_string(), payload_url.to_string()),
        ("web_hook[content_type]".to_string(), content_type.to_string()),
        ("web_hook[wildcard_web_hook]".to_string(), "true".to_string()),
        ("web_hook[active]".to_string(), active.to_string()),
        (
            "web_hook[verify_certificate]".to_string(),
            verify_certificate.to_string(),
        ),
    ];
    if let Some(secret) = secret {
        payload.push(("web_hook[secret]".to_string(), secret.to_string()));
    }
    for event_type_id in event_type_ids {
        payload.push((
            "web_hook[web_hook_event_type_ids][]".to_string(),
            event_type_id.to_string(),
        ));
    }
    Ok(payload)
}

/// Remove URL userinfo before it can reach a CLI output format or dry-run plan.
pub(crate) fn redact_webhook_url(payload_url: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(payload_url) else {
        return payload_url.to_string();
    };
    if parsed.username().is_empty() && parsed.password().is_none() {
        return payload_url.to_string();
    }
    if parsed.set_username("***").is_err() {
        return "<redacted URL>".to_string();
    }
    let _ = parsed.set_password(None);
    parsed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wire_webhook(id: u64) -> WebhookWire {
        serde_json::from_value(json!({
            "id": id,
            "payload_url": format!("https://example.test/hooks/{id}"),
            "content_type": 1,
            "active": true,
            "wildcard_web_hook": true,
            "verify_certificate": true,
            "last_delivery_status": 3,
            "category_ids": [],
            "group_ids": [],
            "tags": [],
            "web_hook_event_types": []
        }))
        .expect("wire webhook")
    }

    #[test]
    fn public_webhook_output_omits_secrets_and_url_credentials() {
        let webhook: WebhookWire = serde_json::from_value(json!({
            "id": 7,
            "payload_url": "https://user:url-canary@example.test/hooks/7",
            "secret": "secret-canary",
            "active": true,
            "wildcard_web_hook": true,
            "verify_certificate": true
        }))
        .expect("wire webhook");
        let output: WebhookSummary = webhook.into();

        assert_eq!(output.payload_url, "https://***@example.test/hooks/7");
        for rendered in [
            serde_json::to_string(&output).expect("json"),
            serde_yaml::to_string(&output).expect("yaml"),
        ] {
            assert!(!rendered.contains("secret-canary"));
            assert!(!rendered.contains("url-canary"));
        }
    }

    #[test]
    fn webhook_create_payload_uses_discord_defaults_for_wildcard_delivery() {
        let payload = webhook_create_payload(
            "https://example.test/hook",
            1,
            Some("0123456789ab"),
            true,
            true,
            &[201, 202],
        )
        .expect("payload");

        assert!(payload.contains(&(
            "web_hook[wildcard_web_hook]".to_string(),
            "true".to_string()
        )));
        assert!(payload.contains(&(
            "web_hook[web_hook_event_type_ids][]".to_string(),
            "201".to_string()
        )));
        assert!(payload.contains(&(
            "web_hook[web_hook_event_type_ids][]".to_string(),
            "202".to_string()
        )));
    }

    #[test]
    fn pagination_collects_each_webhook_once() {
        let mut hooks = Vec::new();
        let mut seen_ids = HashSet::new();
        let first = (1..=WEBHOOK_PAGE_SIZE)
            .map(|id| wire_webhook(id as u64))
            .collect();
        append_webhook_page(&mut hooks, &mut seen_ids, first).expect("first page");
        assert!(has_more_webhook_pages(
            WEBHOOK_PAGE_SIZE,
            Some(WEBHOOK_PAGE_SIZE + 1),
            hooks.len()
        ));

        append_webhook_page(&mut hooks, &mut seen_ids, vec![wire_webhook(51)])
            .expect("second page");
        assert_eq!(hooks.len(), 51);
        assert!(!has_more_webhook_pages(1, Some(51), hooks.len()));
    }

    #[test]
    fn pagination_rejects_duplicate_webhook_ids() {
        let mut hooks = Vec::new();
        let mut seen_ids = HashSet::new();
        append_webhook_page(&mut hooks, &mut seen_ids, vec![wire_webhook(7)])
            .expect("first page");
        assert!(append_webhook_page(&mut hooks, &mut seen_ids, vec![wire_webhook(7)]).is_err());
    }
}

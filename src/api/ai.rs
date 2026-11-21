// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Safe read-only client for Discourse AI LLM definitions. The server serializer
//! includes raw custom-model API keys, so wire records must never reach output
//! directly. See `spec/commands/ai-llm-management.md` (R88).

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;
use crate::api::redact_webhook_url;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

const AI_LLMS_PATH: &str = "/admin/plugins/discourse-ai/ai-llms.json";

/// One non-secret LLM definition suitable for CLI output.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AiLlmSummary {
    pub id: i64,
    pub display_name: String,
    pub name: String,
    pub provider: String,
    pub tokenizer: Option<String>,
    pub endpoint: Option<String>,
    pub api_key_configured: bool,
    pub ai_secret_id: Option<i64>,
    pub seeded: bool,
    pub max_prompt_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AiLlmListWire {
    ai_llms: Vec<AiLlmWire>,
}

/// Deliberately private: the current Discourse AI serializer exposes `api_key`
/// for custom models. Only its presence is projected into `AiLlmSummary`.
#[derive(Debug, Deserialize)]
struct AiLlmWire {
    id: i64,
    display_name: String,
    name: String,
    provider: String,
    #[serde(default)]
    tokenizer: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    ai_secret_id: Option<i64>,
    #[serde(default)]
    seeded: bool,
    #[serde(default)]
    max_prompt_tokens: Option<i64>,
    #[serde(default)]
    max_output_tokens: Option<i64>,
}

impl From<AiLlmWire> for AiLlmSummary {
    fn from(value: AiLlmWire) -> Self {
        let endpoint = value.url.map(|url| {
            if url == "********" {
                "<masked>".to_string()
            } else {
                redact_webhook_url(&url)
            }
        });
        Self {
            id: value.id,
            display_name: value.display_name,
            name: value.name,
            provider: value.provider,
            tokenizer: value.tokenizer,
            endpoint,
            api_key_configured: value.api_key.is_some_and(|key| !key.is_empty()),
            ai_secret_id: value.ai_secret_id,
            seeded: value.seeded,
            max_prompt_tokens: value.max_prompt_tokens,
            max_output_tokens: value.max_output_tokens,
        }
    }
}

impl DiscourseClient {
    /// List LLM definitions through the documented Discourse AI admin API.
    ///
    /// The route's response includes provider metadata and may include raw API
    /// keys. This method intentionally parses only definition rows and returns
    /// an allowlisted public representation.
    pub fn list_ai_llms(&self) -> Result<Vec<AiLlmSummary>> {
        let response = self.get(AI_LLMS_PATH)?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading Discourse AI LLM response")?;
        if !status.is_success() {
            return Err(ai_llms_http_error(status));
        }
        let response: AiLlmListWire =
            serde_json::from_str(&text).context("parsing Discourse AI LLM response")?;
        Ok(response.ai_llms.into_iter().map(Into::into).collect())
    }
}

fn ai_llms_http_error(status: reqwest::StatusCode) -> anyhow::Error {
    let base = http_error(
        "Discourse AI LLM list request",
        status,
        "response body redacted because this API can contain credentials",
    );
    if status == reqwest::StatusCode::NOT_FOUND {
        return anyhow!(
            "{}; Discourse AI may be disabled, unavailable on this Discourse version, or its admin API may have changed",
            base
        );
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn public_output_omits_api_key_and_provider_params() {
        let wire: AiLlmWire = serde_json::from_value(json!({
            "id": 7,
            "display_name": "Safe output test",
            "name": "model-canary",
            "provider": "open_ai",
            "url": "https://user:url-canary@example.test/v1?token=query-canary",
            "api_key": "api-key-canary",
            "provider_params": {"access_key_id": "provider-secret-canary"}
        }))
        .expect("wire LLM");
        let output: AiLlmSummary = wire.into();

        assert!(output.api_key_configured);
        for rendered in [
            serde_json::to_string(&output).expect("JSON"),
            serde_yaml::to_string(&output).expect("YAML"),
        ] {
            assert!(!rendered.contains("api-key-canary"));
            assert!(!rendered.contains("provider-secret-canary"));
            assert!(!rendered.contains("url-canary"));
            assert!(!rendered.contains("query-canary"));
        }
    }

    #[test]
    fn seeded_endpoint_stays_masked() {
        let wire: AiLlmWire = serde_json::from_value(json!({
            "id": -200,
            "display_name": "Seeded",
            "name": "seeded-model",
            "provider": "CDCK",
            "url": "********",
            "api_key": "********",
            "seeded": true
        }))
        .expect("wire seeded LLM");
        let output: AiLlmSummary = wire.into();

        assert_eq!(output.id, -200);
        assert_eq!(output.endpoint.as_deref(), Some("<masked>"));
        assert!(output.api_key_configured);
    }

    #[test]
    fn unavailable_api_has_a_safe_discovery_hint() {
        let error = ai_llms_http_error(reqwest::StatusCode::NOT_FOUND).to_string();
        assert!(error.contains("Discourse AI may be disabled"));
        assert!(!error.contains("api-key-canary"));
    }
}

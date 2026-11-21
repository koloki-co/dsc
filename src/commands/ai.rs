// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{emit_with, ensure_api_credentials, not_found, select_discourse};
use crate::config::Config;
use anyhow::Result;

/// List the configured forum's non-secret Discourse AI LLM definitions.
pub fn ai_llm_list(config: &Config, discourse_name: &str, format: ListFormat) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let mut llms = client.list_ai_llms()?;
    llms.sort_by(|left, right| left.display_name.cmp(&right.display_name));

    emit_with(format, &llms, || {
        if llms.is_empty() {
            println!("No AI LLM definitions found.");
            return Ok(());
        }
        for llm in &llms {
            println!(
                "{} - {} - {} ({}, API key: {})",
                llm.id,
                llm.display_name,
                llm.name,
                llm.provider,
                if llm.api_key_configured {
                    "configured"
                } else {
                    "absent"
                }
            );
        }
        Ok(())
    })
}

/// Show one definition, selected from the documented list response rather than
/// the browser-oriented edit route.
pub fn ai_llm_show(
    config: &Config,
    discourse_name: &str,
    llm_id: i64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let llm = client
        .list_ai_llms()?
        .into_iter()
        .find(|llm| llm.id == llm_id)
        .ok_or_else(|| not_found("AI LLM definition", llm_id))?;

    emit_with(format, &llm, || {
        println!("{} - {} - {}", llm.id, llm.display_name, llm.name);
        println!("provider: {}", llm.provider);
        println!(
            "API key: {}",
            if llm.api_key_configured {
                "configured"
            } else {
                "absent"
            }
        );
        if let Some(endpoint) = &llm.endpoint {
            println!("endpoint: {endpoint}");
        }
        Ok(())
    })
}

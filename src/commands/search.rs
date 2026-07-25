// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use anyhow::Result;

pub fn search(
    config: &Config,
    discourse_name: &str,
    query: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let hits = client.search_topics(query)?;

    match format {
        ListFormat::Text => {
            if hits.is_empty() {
                println!("No search results found.");
                return Ok(());
            }
            let id_width = hits
                .iter()
                .map(|h| h.id.to_string().len())
                .max()
                .unwrap_or(2);
            for hit in &hits {
                let title = if hit.title.trim().is_empty() {
                    hit.slug.as_str()
                } else {
                    hit.title.as_str()
                };
                println!("{:>width$}  {}", hit.id, title, width = id_width);
            }
        }
        ListFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&hits)?);
        }
        ListFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&hits)?);
        }
    }

    Ok(())
}

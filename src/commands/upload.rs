// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use anyhow::{Result, anyhow};
use std::path::Path;

pub fn upload(
    config: &Config,
    discourse_name: &str,
    file_path: &Path,
    upload_type: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if !file_path.is_file() {
        return Err(anyhow!("not a file: {}", file_path.display()));
    }

    let info = client.upload_file(file_path, upload_type)?;

    match format {
        ListFormat::Text => {
            // Default text output prints just the short URL — that's what
            // gets pasted into post bodies. Single line, pipe-friendly.
            if let Some(short) = &info.short_url {
                println!("{}", short);
            } else {
                println!("{}", info.url);
            }
        }
        ListFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        ListFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&info)?);
        }
    }

    Ok(())
}

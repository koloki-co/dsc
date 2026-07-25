// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{emit_result, ensure_api_credentials, select_discourse};
use crate::config::Config;
use anyhow::Result;
use serde_json::json;

/// `dsc version [discourse] [--format]`. Without a discourse, report dsc's own
/// version; with one, report that forum's live Discourse version + commit.
pub fn version(config: &Config, discourse: Option<&str>, format: ListFormat) -> Result<()> {
    match discourse {
        Some(name) => forum_version(config, name, format),
        None => own_version(format),
    }
}

/// Report dsc's own version without loading configuration or contacting a forum.
pub fn own_version(format: ListFormat) -> Result<()> {
    let ver = env!("CARGO_PKG_VERSION");
    emit_result(format, &json!({ "name": "dsc", "version": ver }), ver)
}

/// Print a configured forum's live Discourse version and git commit, read from
/// `/about.json`. Uses the configured API key, so it works even on
/// login-required forums where an anonymous request would be rejected.
pub fn forum_version(config: &Config, discourse_name: &str, format: ListFormat) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let info = client.fetch_version_info()?;
    let version = info.version.as_deref().unwrap_or("(unknown)");
    let commit = info.commit.as_deref().unwrap_or("(unknown)");
    let text = format!("{}: Discourse {} ({})", discourse.name, version, commit);
    emit_result(
        format,
        &json!({ "discourse": discourse.name, "version": version, "commit": commit }),
        &text,
    )
}

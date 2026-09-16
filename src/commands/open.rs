// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::commands::common::{open_url, open_url_detached, selected_discourses};
use crate::config::Config;
use anyhow::{Context, Result};

/// Open one forum's base URL, or fan out across a fleet selection. A single
/// exact name keeps `dsc open`'s checked, interactive behaviour; a fleet
/// selection (`--all`, `--tags`, or an explicit name glob) launches every
/// opener detached so one slow or hung opener cannot serialize the rest.
pub fn open_discourse(
    config: &Config,
    discourse_name: Option<&str>,
    all: Option<()>,
    tags: Option<&str>,
) -> Result<()> {
    let _ = all;
    // A glob is always a fleet selection even when it matches exactly one
    // forum: the caller expressed "open every match", not "open this one and
    // tell me when the browser exits".
    let fleet = all.is_some()
        || tags.is_some()
        || discourse_name.is_some_and(|name| name.contains('*') || name.contains('?'));
    let discourses = selected_discourses(config, discourse_name, tags)?;
    if fleet {
        for discourse in discourses {
            open_url_detached(&discourse.baseurl)
                .with_context(|| format!("opening browser for '{}'", discourse.baseurl))?;
        }
        Ok(())
    } else {
        // Preserve the single-forum contract: inherit stdio, check the
        // opener's exit status.
        open_url(&discourses[0].baseurl)
            .with_context(|| format!("opening browser for '{}'", discourses[0].baseurl))
    }
}

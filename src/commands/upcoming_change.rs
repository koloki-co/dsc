// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{DiscourseClient, UpcomingChange};
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use anyhow::Result;

/// List every Upcoming Change the forum exposes.
pub fn upcoming_change_list(
    config: &Config,
    discourse_name: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let changes = client.list_upcoming_changes()?;
    match format {
        ListFormat::Text => print_list(&changes),
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&changes)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&changes)?),
    }
    Ok(())
}

/// Show one Upcoming Change by its exact setting name.
pub fn upcoming_change_show(
    config: &Config,
    discourse_name: &str,
    setting_name: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let change = client.show_upcoming_change(setting_name)?;
    match format {
        ListFormat::Text => print_detail(&change),
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&change)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&change)?),
    }
    Ok(())
}

fn print_list(changes: &[UpcomingChange]) {
    if changes.is_empty() {
        println!("No Upcoming Changes found.");
        return;
    }
    for change in changes {
        println!(
            "{}  value:{}  {}",
            change.setting,
            change.value,
            change_summary(change)
        );
    }
}

fn print_detail(change: &UpcomingChange) {
    println!("setting:     {}", change.setting);
    println!(
        "name:        {}",
        change.humanized_name.as_deref().unwrap_or("-")
    );
    println!(
        "description: {}",
        change.description.as_deref().unwrap_or("-")
    );
    println!("value:       {}", change.value);
    println!("{}", change_summary(change));
    println!(
        "depends_on_met: {}",
        change
            .depends_on_met
            .map(|met| met.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "overriding_defaults: {}",
        change
            .overriding_defaults
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
}

fn change_summary(change: &UpcomingChange) -> String {
    let Some(detail) = change.upcoming_change.as_ref() else {
        return "(not an Upcoming Change; a promoted or ordinary setting)".to_string();
    };
    format!(
        "status:{} enabled_for:{}",
        detail.status.as_deref().unwrap_or("-"),
        detail.enabled_for.as_deref().unwrap_or("-")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::UpcomingChangeDetail;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn sample() -> UpcomingChange {
        UpcomingChange {
            setting: "enable_generated_llms_txt".to_string(),
            humanized_name: Some("Enable generated llms txt".to_string()),
            description: Some("Generates a concise /llms.txt.".to_string()),
            value: json!(true),
            upcoming_change: Some(UpcomingChangeDetail {
                status: Some("beta".to_string()),
                impact: None,
                impact_type: None,
                impact_role: None,
                enabled_for: Some("everyone".to_string()),
                extra: BTreeMap::new(),
            }),
            plugin: None,
            depends_on: None,
            depends_on_humanized_names: None,
            dependents: vec![],
            depends_on_met: Some(true),
            overriding_defaults: Some(true),
            groups: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn change_summary_reports_status_and_scope() {
        assert_eq!(
            change_summary(&sample()),
            "status:beta enabled_for:everyone"
        );
    }

    #[test]
    fn change_summary_flags_a_non_upcoming_setting() {
        let mut change = sample();
        change.upcoming_change = None;
        assert!(change_summary(&change).contains("not an Upcoming Change"));
    }
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::commands::update::run_ssh_command;
use crate::config::{Config, DiscourseConfig};
use anyhow::{Result, anyhow};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct PluginListEntry {
    name: String,
    version: String,
    status: String,
}

pub fn plugin_list(
    config: &Config,
    discourse_name: &str,
    format: ListFormat,
    verbose: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.list_plugins()?;
    let plugins = response
        .get("plugins")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let entries: Vec<PluginListEntry> = plugins
        .into_iter()
        .map(|plugin| {
            let name = plugin
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let version = plugin
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let status = plugin
                .get("enabled")
                .and_then(|v| v.as_bool())
                .or_else(|| plugin.get("active").and_then(|v| v.as_bool()))
                .map(|value| {
                    if value {
                        "enabled".to_string()
                    } else {
                        "disabled".to_string()
                    }
                })
                .unwrap_or_else(|| "unknown".to_string());
            PluginListEntry {
                name,
                version,
                status,
            }
        })
        .collect();

    match format {
        ListFormat::Text => {
            if entries.is_empty() && !verbose {
                println!("No plugins found.");
                return Ok(());
            }
            for plugin in entries {
                println!("{} - {} - {}", plugin.name, plugin.version, plugin.status);
            }
        }
        ListFormat::Json => {
            let raw = serde_json::to_string_pretty(&entries)?;
            println!("{}", raw);
        }
        ListFormat::Yaml => {
            let raw = serde_yaml::to_string(&entries)?;
            println!("{}", raw);
        }
    }
    Ok(())
}

pub fn plugin_install(
    config: &Config,
    discourse_name: &str,
    url: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    let target = ssh_target(discourse);
    let template = std::env::var("DSC_SSH_PLUGIN_INSTALL_CMD")
        .map_err(|_| {
            anyhow!(
                "missing DSC_SSH_PLUGIN_INSTALL_CMD for plugin install; set DSC_SSH_PLUGIN_INSTALL_CMD to your install command"
            )
        })?;
    let command = render_template(&template, &[("url", url), ("name", url)]);
    if dry_run {
        println!("[dry-run] would run on {}: {}", target, command);
        return Ok(());
    }
    let output = run_ssh_command(&target, &command)?;
    println!("Plugin install completed: {}", url);
    if !output.trim().is_empty() {
        println!("{}", output.trim());
    }
    Ok(())
}

pub fn plugin_remove(
    config: &Config,
    discourse_name: &str,
    name: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    let target = ssh_target(discourse);
    let template = std::env::var("DSC_SSH_PLUGIN_REMOVE_CMD")
        .map_err(|_| {
            anyhow!(
                "missing DSC_SSH_PLUGIN_REMOVE_CMD for plugin remove; set DSC_SSH_PLUGIN_REMOVE_CMD to your remove command"
            )
        })?;
    let command = render_template(&template, &[("name", name), ("url", name)]);
    if dry_run {
        println!("[dry-run] would run on {}: {}", target, command);
        return Ok(());
    }
    let output = run_ssh_command(&target, &command)?;
    println!("Plugin removal completed: {}", name);
    if !output.trim().is_empty() {
        println!("{}", output.trim());
    }
    Ok(())
}

fn ssh_target(discourse: &DiscourseConfig) -> String {
    discourse
        .ssh_host
        .clone()
        .unwrap_or_else(|| discourse.name.clone())
}

fn render_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in replacements {
        out = out.replace(&format!("{{{}}}", key), value);
    }
    out
}

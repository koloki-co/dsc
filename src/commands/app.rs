// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::cli::ListFormat;
use crate::commands::common::{
    ensure_api_credentials, fleet_worker_count, run_fleet, select_discourse, selected_discourses,
    shell_quote,
};
use crate::commands::ssh::run_ssh_text;
use crate::config::{Config, DiscourseConfig};
use anyhow::{Result, anyhow};
use serde::Serialize;
use std::collections::BTreeMap;

const DEFAULT_APP_YML_PATH: &str = "/var/discourse/containers/app.yml";
const REBUILD_CHECK_CMD: &str =
    "pgrep -f '[l]auncher rebuild' >/dev/null 2>&1 && echo REBUILDING || echo IDLE";

pub struct AppEnvChangeOptions {
    pub rebuild: bool,
    pub backup: bool,
    pub dry_run: bool,
    pub yes: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct AppEnvEntry {
    key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redacted: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
struct AppEnvAuditRow {
    discourse: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redacted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// List the non-secret variable names in an app.yml `env:` block.
pub fn app_env_list(config: &Config, discourse_name: &str, format: ListFormat) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    let env = fetch_app_env(discourse)?;
    let entries: Vec<AppEnvEntry> = env
        .keys()
        .filter(|key| !is_secret_key(key))
        .map(|key| AppEnvEntry {
            key: key.clone(),
            value: None,
            redacted: None,
        })
        .collect();
    match format {
        ListFormat::Text => {
            if entries.is_empty() {
                println!("No non-secret app environment variables found.");
            } else {
                for entry in entries {
                    println!("{}", entry.key);
                }
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&entries)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&entries)?),
    }
    Ok(())
}

/// Read one app.yml environment variable, redacting likely secrets unless the
/// caller explicitly requests the value for a single forum.
pub fn app_env_get(
    config: &Config,
    discourse_name: &str,
    key: &str,
    show_secret: bool,
    format: ListFormat,
) -> Result<()> {
    validate_env_key(key)?;
    let discourse = select_discourse(config, Some(discourse_name))?;
    let env = fetch_app_env(discourse)?;
    let value = env
        .get(key)
        .ok_or_else(|| anyhow!("environment variable not found: {key}"))?;
    let secret = is_secret_key(key);
    let entry = AppEnvEntry {
        key: key.to_string(),
        value: (!secret || show_secret).then(|| value.clone()),
        redacted: secret.then_some(true),
    };
    match format {
        ListFormat::Text => {
            if secret && !show_secret {
                println!("{} = [REDACTED] (use --show-secret to reveal)", entry.key);
            } else {
                println!("{} = {}", entry.key, value);
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&entry)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&entry)?),
    }
    Ok(())
}

/// Compare one environment variable across matching forums. Secret values are
/// always redacted because audit output is inherently multi-target.
pub fn app_env_audit(
    config: &Config,
    key: &str,
    tags: Option<&str>,
    format: ListFormat,
) -> Result<()> {
    validate_env_key(key)?;
    let discourses = selected_discourses(config, None, tags)?;
    let key_owned = key.to_string();

    let rows: Vec<AppEnvAuditRow> = run_fleet(
        &discourses,
        fleet_worker_count(None, discourses.len(), 8, false),
        move |discourse| match fetch_app_env(discourse) {
            Ok(env) => {
                let value = env.get(&key_owned).cloned();
                let secret = is_secret_key(&key_owned) && value.is_some();
                AppEnvAuditRow {
                    discourse: discourse.name.clone(),
                    value: (!secret).then_some(value).flatten(),
                    redacted: secret.then_some(true),
                    error: None,
                }
            }
            Err(error) => AppEnvAuditRow {
                discourse: discourse.name.clone(),
                value: None,
                redacted: None,
                error: Some(error.to_string()),
            },
        },
        |_| {},
    );

    match format {
        ListFormat::Text => {
            if rows.is_empty() {
                println!("No Discourses selected.");
            } else {
                for row in &rows {
                    let state = if let Some(error) = &row.error {
                        format!("ERROR: {error}")
                    } else if row.redacted == Some(true) {
                        "[REDACTED]".to_string()
                    } else {
                        row.value.clone().unwrap_or_else(|| "[UNSET]".to_string())
                    };
                    println!("{}: {}", row.discourse, state);
                }
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&rows)?),
    }
    Ok(())
}

/// Set a single scalar value in a plain `env:` mapping without reserialising
/// the document and risking comments or unrelated app.yml sections.
pub fn app_env_set(
    config: &Config,
    discourse_name: &str,
    key: &str,
    value: &str,
    options: AppEnvChangeOptions,
) -> Result<()> {
    change_app_env(config, discourse_name, key, Some(value), options)
}

/// Remove one scalar value from a plain `env:` mapping.
pub fn app_env_unset(
    config: &Config,
    discourse_name: &str,
    key: &str,
    options: AppEnvChangeOptions,
) -> Result<()> {
    change_app_env(config, discourse_name, key, None, options)
}

fn change_app_env(
    config: &Config,
    discourse_name: &str,
    key: &str,
    new_value: Option<&str>,
    options: AppEnvChangeOptions,
) -> Result<()> {
    validate_env_key(key)?;
    validate_env_value(new_value)?;
    let discourse = select_discourse(config, Some(discourse_name))?;
    let target = ssh_target(discourse)?;
    let path = app_yml_path(discourse);
    let original = run_ssh_text(target, &format!("cat -- {}", shell_quote(&path)))?;
    let (updated, previous) = edit_app_env(&original, key, new_value)?;
    if updated == original {
        println!("{} is already unchanged on {}.", key, discourse.name);
        return Ok(());
    }
    print_change_plan(
        discourse,
        key,
        previous.as_deref(),
        new_value,
        options.backup,
        options.rebuild,
    );
    if options.dry_run {
        println!("Nothing was changed (--dry-run).");
        return Ok(());
    }
    if !options.yes {
        return Err(anyhow!(
            "refusing to change {} on {} without --yes; review with --dry-run first",
            key,
            discourse.name
        ));
    }
    if options.rebuild && run_ssh_text(target, REBUILD_CHECK_CMD)?.contains("REBUILDING") {
        return Err(anyhow!(
            "a launcher rebuild is already running on {}; no changes were made",
            discourse.name
        ));
    }
    write_app_env(target, &path, &updated, options.backup)?;
    let verified = run_ssh_text(target, &format!("cat -- {}", shell_quote(&path)))?;
    let parsed = parse_app_env(&verified)?;
    if parsed.get(key).map(String::as_str) != new_value {
        return Err(anyhow!("post-write verification failed for {key}"));
    }
    println!("Changed {} on {}.", key, discourse.name);
    if options.rebuild {
        let install_dir = path
            .strip_suffix("/containers/app.yml")
            .unwrap_or("/var/discourse");
        run_ssh_text(
            target,
            &format!(
                "cd -- {} && ./launcher rebuild app",
                shell_quote(install_dir)
            ),
        )?;
        println!("Rebuilt app on {}.", discourse.name);
    } else {
        println!(
            "Run `dsc app env set {} {} … --rebuild --yes` or rebuild manually for the change to take effect.",
            discourse.name, key
        );
    }
    Ok(())
}

fn ssh_target(discourse: &DiscourseConfig) -> Result<&str> {
    discourse
        .ssh_host
        .as_deref()
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| anyhow!("missing ssh_host for discourse {}", discourse.name))
}

fn validate_env_value(value: Option<&str>) -> Result<()> {
    if let Some(value) = value
        && (value.contains('\n') || value.contains('\r') || value.contains('\0'))
    {
        return Err(anyhow!("environment variable value must be a single line"));
    }
    Ok(())
}

fn print_change_plan(
    discourse: &DiscourseConfig,
    key: &str,
    previous: Option<&str>,
    new_value: Option<&str>,
    backup: bool,
    rebuild: bool,
) {
    let display = |value: Option<&str>| match value {
        Some(_) if is_secret_key(key) => "[REDACTED]".to_string(),
        Some(value) => format!("{value:?}"),
        None => "[UNSET]".to_string(),
    };
    println!(
        "{}: {} {} -> {}",
        discourse.name,
        key,
        display(previous),
        display(new_value)
    );
    println!(
        "remote backup: {}",
        if backup { "enabled" } else { "disabled" }
    );
    println!(
        "rebuild: {}",
        if rebuild { "enabled" } else { "not requested" }
    );
}

fn write_app_env(target: &str, path: &str, content: &str, backup: bool) -> Result<()> {
    let encoded = base64_encode(content);
    let backup_command = if backup {
        format!(
            "backup={}.dsc-$(date -u +%Y%m%dT%H%M%SZ).bak && cp -- {} \"$backup\" && ",
            shell_quote(path),
            shell_quote(path)
        )
    } else {
        String::new()
    };
    let command = format!(
        "set -eu; tmp=$(mktemp {dir}/.dsc-app-env.XXXXXX); {backup}printf '%s' {encoded} | base64 -d > \"$tmp\"; chmod --reference={path} \"$tmp\"; chown --reference={path} \"$tmp\"; mv -f \"$tmp\" {path}",
        dir = shell_quote(path.rsplit_once('/').map_or(".", |(dir, _)| dir)),
        backup = backup_command,
        encoded = shell_quote(&encoded),
        path = shell_quote(path),
    );
    run_ssh_text(target, &command).map(|_| ())
}

fn base64_encode(value: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(value)
}

fn edit_app_env(raw: &str, key: &str, new_value: Option<&str>) -> Result<(String, Option<String>)> {
    let mut lines: Vec<String> = raw.split_inclusive('\n').map(str::to_string).collect();
    if lines.is_empty() {
        return Err(anyhow!("app.yml is empty"));
    }
    let env_line = lines
        .iter()
        .position(|line| line.trim_end_matches(['\r', '\n']) == "env:")
        .ok_or_else(|| anyhow!("app.yml is missing a plain top-level env: mapping"))?;
    let end = lines[env_line + 1..]
        .iter()
        .position(|line| {
            let trimmed = line.trim_start();
            !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !line.starts_with(' ')
                && !line.starts_with('\t')
        })
        .map_or(lines.len(), |offset| env_line + 1 + offset);
    let mut existing = None;
    for (index, line) in lines.iter().enumerate().take(end).skip(env_line + 1) {
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.starts_with('-') || trimmed.starts_with('&') || trimmed.starts_with('*') {
            return Err(anyhow!("app.yml env mapping uses unsupported YAML syntax"));
        }
        let Some((candidate, raw_value)) = trimmed.trim_end_matches(['\r', '\n']).split_once(':')
        else {
            if !trimmed.trim().is_empty() && !trimmed.trim_start().starts_with('#') {
                return Err(anyhow!("app.yml env mapping contains an unsupported entry"));
            }
            continue;
        };
        if candidate.trim() == key {
            if existing.is_some() {
                return Err(anyhow!("app.yml contains duplicate env key {key}"));
            }
            let value = raw_value.trim_start();
            if value.contains('#')
                || value.contains('&')
                || value.contains('*')
                || value.starts_with('|')
                || value.starts_with('>')
            {
                return Err(anyhow!(
                    "app.yml env value for {key} uses unsupported YAML syntax"
                ));
            }
            existing = Some((index, value.trim_end().to_string()));
        }
    }
    let previous = existing.as_ref().map(|(_, value)| value.clone());
    match (existing, new_value) {
        (Some((index, _)), Some(value)) => {
            let indent = lines[index].len() - lines[index].trim_start_matches([' ', '\t']).len();
            lines[index] = format!("{}{}: {}\n", " ".repeat(indent), key, yaml_scalar(value));
        }
        (Some((index, _)), None) => {
            lines.remove(index);
        }
        (None, Some(value)) => {
            let insert_at = end;
            lines.insert(insert_at, format!("  {key}: {}\n", yaml_scalar(value)));
        }
        (None, None) => return Err(anyhow!("environment variable not found: {key}")),
    }
    Ok((lines.concat(), previous))
}

fn yaml_scalar(value: &str) -> String {
    if !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | '/' | ':')
        })
        && !matches!(value, "true" | "false" | "null" | "~")
    {
        return value.to_string();
    }
    serde_yaml::to_string(value)
        .expect("serializing a string cannot fail")
        .trim_end()
        .to_string()
}

fn fetch_app_env(discourse: &DiscourseConfig) -> Result<BTreeMap<String, String>> {
    ensure_api_credentials(discourse)?;
    let target = discourse
        .ssh_host
        .as_deref()
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| anyhow!("missing ssh_host for discourse {}", discourse.name))?;
    let path = app_yml_path(discourse);
    let command = format!("cat -- {}", shell_quote(&path));
    let raw = run_ssh_text(target, &command)?;
    parse_app_env(&raw)
}

fn app_yml_path(discourse: &DiscourseConfig) -> String {
    discourse
        .app_yml_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .unwrap_or(DEFAULT_APP_YML_PATH)
        .to_string()
}

fn parse_app_env(raw: &str) -> Result<BTreeMap<String, String>> {
    let value: serde_yaml::Value =
        serde_yaml::from_str(raw).map_err(|error| anyhow!("parsing app.yml: {error}"))?;
    let env = value
        .get("env")
        .and_then(serde_yaml::Value::as_mapping)
        .ok_or_else(|| anyhow!("app.yml is missing an env mapping"))?;
    let mut entries = BTreeMap::new();
    for (key, value) in env {
        let key = key
            .as_str()
            .ok_or_else(|| anyhow!("app.yml env key is not a string"))?;
        validate_env_key(key)?;
        let value = match value {
            serde_yaml::Value::String(value) => value.clone(),
            serde_yaml::Value::Number(value) => value.to_string(),
            serde_yaml::Value::Bool(value) => value.to_string(),
            _ => return Err(anyhow!("app.yml env value for {key} is not a scalar")),
        };
        entries.insert(key.to_string(), value);
    }
    Ok(entries)
}

fn validate_env_key(key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty()
        || !key.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
    {
        return Err(anyhow!("invalid environment variable name: {key}"));
    }
    Ok(())
}

fn is_secret_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    [
        "SECRET",
        "PASSWORD",
        "TOKEN",
        "API_KEY",
        "ACCESS_KEY",
        "PRIVATE_KEY",
        "SMTP",
        "DATABASE_URL",
    ]
    .iter()
    .any(|needle| upper.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalar_env_mapping() {
        let env = parse_app_env(
            r#"
env:
  DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE: 120
  DISCOURSE_HOSTNAME: forum.example.com
"#,
        )
        .unwrap();
        assert_eq!(env["DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE"], "120");
        assert_eq!(env["DISCOURSE_HOSTNAME"], "forum.example.com");
    }

    #[test]
    fn rejects_missing_or_non_scalar_env_values() {
        assert!(parse_app_env("templates: []").is_err());
        assert!(parse_app_env("env:\n  THING: [one]").is_err());
    }

    #[test]
    fn secret_detection_is_conservative() {
        assert!(is_secret_key("DISCOURSE_SMTP_PASSWORD"));
        assert!(is_secret_key("S3_ACCESS_KEY_ID"));
        assert!(!is_secret_key("DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE"));
    }

    #[test]
    fn validates_env_variable_names() {
        assert!(validate_env_key("DISCOURSE_HOSTNAME").is_ok());
        assert!(validate_env_key("bad-name").is_err());
        assert!(validate_env_key("VALUE; rm -rf /").is_err());
    }

    #[test]
    fn edits_one_env_value_without_reserializing_other_content() {
        let raw = "templates:\n  - \"templates/web.template.yml\"\nenv:\n  DISCOURSE_HOSTNAME: forum.example.com\n  DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE: 60\nparams:\n  - \"db_default_text_search_config=english\"\n";
        let (updated, previous) =
            edit_app_env(raw, "DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE", Some("120")).unwrap();
        assert_eq!(previous.as_deref(), Some("60"));
        assert_eq!(
            updated,
            "templates:\n  - \"templates/web.template.yml\"\nenv:\n  DISCOURSE_HOSTNAME: forum.example.com\n  DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE: 120\nparams:\n  - \"db_default_text_search_config=english\"\n"
        );
    }

    #[test]
    fn adds_and_removes_env_values() {
        let raw = "env:\n  DISCOURSE_HOSTNAME: forum.example.com\n";
        let (added, previous) =
            edit_app_env(raw, "DISCOURSE_CDN_URL", Some("https://cdn.example")).unwrap();
        assert_eq!(previous, None);
        assert!(added.contains("  DISCOURSE_CDN_URL: https://cdn.example\n"));
        let (removed, previous) = edit_app_env(&added, "DISCOURSE_CDN_URL", None).unwrap();
        assert_eq!(previous.as_deref(), Some("https://cdn.example"));
        assert_eq!(removed, raw);
    }

    #[test]
    fn rejects_unsupported_env_syntax() {
        assert!(edit_app_env("env:\n  THING: value # comment\n", "THING", Some("new")).is_err());
        assert!(edit_app_env("env:\n  - THING=value\n", "THING", Some("new")).is_err());
    }

    #[test]
    fn rejects_multiline_values() {
        assert!(validate_env_value(Some("one\ntwo")).is_err());
    }
}

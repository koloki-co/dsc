// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{
    emit_result, ensure_api_credentials, not_found, render_shell_template, select_discourse,
};
use crate::commands::ssh::run_ssh_text;
use crate::config::{Config, DiscourseConfig};
use crate::utils::{atomic_write_private, slugify};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Serialize)]
struct ThemeListEntry {
    id: u64,
    name: String,
    status: String,
}

pub fn theme_list(
    config: &Config,
    discourse_name: &str,
    format: ListFormat,
    verbose: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.list_themes()?;
    let themes = response
        .get("themes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let entries: Vec<ThemeListEntry> = themes
        .into_iter()
        .map(|theme| {
            let id = theme.get("id").and_then(|v| v.as_u64()).unwrap_or_default();
            let name = theme
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let status = theme
                .get("enabled")
                .and_then(|v| v.as_bool())
                .map(|value| {
                    if value {
                        "enabled".to_string()
                    } else {
                        "disabled".to_string()
                    }
                })
                .unwrap_or_else(|| "unknown".to_string());
            ThemeListEntry { id, name, status }
        })
        .collect();

    match format {
        ListFormat::Text => {
            if entries.is_empty() && !verbose {
                println!("No themes found.");
                return Ok(());
            }
            for theme in entries {
                println!("{} - {} - {}", theme.id, theme.name, theme.status);
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

/// Install a theme/component via the admin import API, from either a git repo
/// (a URL, optionally with embedded credentials for a private repo) or a local
/// bundle file (a `.tar.gz`/zip theme export).
pub fn theme_install(
    config: &Config,
    discourse_name: &str,
    source: &str,
    branch: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let remote = looks_like_git_url(source);

    if dry_run {
        if remote {
            let branch_note = branch
                .filter(|b| !b.is_empty())
                .map(|b| format!(" (branch {})", b))
                .unwrap_or_default();
            println!(
                "[dry-run] {}: would import theme from {}{}",
                discourse.name,
                redact_url(source),
                branch_note
            );
        } else {
            println!(
                "[dry-run] {}: would import theme from local bundle {}",
                discourse.name, source
            );
        }
        return Ok(());
    }

    let result = if remote {
        client.import_theme_remote(source, branch)?
    } else {
        let path = Path::new(source);
        if !path.is_file() {
            return Err(anyhow!(
                "`{}` is neither a git URL nor an existing local bundle file",
                source
            ));
        }
        client.import_theme_bundle(path)?
    };

    let theme = extract_theme(&result);
    let name = theme
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown)");
    match theme.get("id").and_then(|v| v.as_u64()) {
        Some(id) => println!("{}: installed \"{}\" (theme {})", discourse.name, name, id),
        None => println!("{}: theme import completed", discourse.name),
    }
    Ok(())
}

/// Heuristic: does this install source look like a git URL (vs a local path)?
fn looks_like_git_url(s: &str) -> bool {
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("git@")
        || s.starts_with("ssh://")
        || s.ends_with(".git")
}

/// Redact `user:token@` credentials from a URL before printing it.
fn redact_url(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let rest = &url[scheme_end + 3..];
        if let Some(at) = rest.find('@') {
            return format!("{}://***@{}", &url[..scheme_end], &rest[at + 1..]);
        }
    }
    url.to_string()
}

/// Delete a theme/component by id via the admin API. Refuses the site default.
pub fn theme_delete(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);
    let name = theme
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown)")
        .to_string();
    if theme
        .get("default")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err(anyhow!(
            "theme {} (\"{}\") is the site default; set another theme as default before deleting it",
            theme_id,
            name
        ));
    }
    if dry_run {
        println!(
            "[dry-run] {}: would delete theme {} (\"{}\")",
            discourse.name, theme_id, name
        );
        return Ok(());
    }
    client.delete_theme(theme_id)?;
    println!(
        "{}: deleted theme {} (\"{}\")",
        discourse.name, theme_id, name
    );
    Ok(())
}

pub fn theme_remove(
    config: &Config,
    discourse_name: &str,
    name: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    let target = ssh_target(discourse);
    let template = std::env::var("DSC_SSH_THEME_REMOVE_CMD")
        .map_err(|_| {
            anyhow!(
                "missing DSC_SSH_THEME_REMOVE_CMD for theme remove; set DSC_SSH_THEME_REMOVE_CMD to your remove command"
            )
        })?;
    let command = render_shell_template(&template, &[("name", name), ("url", name)]);
    if dry_run {
        println!("[dry-run] would run on {}: {}", target, command);
        return Ok(());
    }
    let output = run_ssh_text(&target, &command)?;
    println!("Theme removal completed: {}", name);
    if !output.trim().is_empty() {
        println!("{}", output.trim());
    }
    Ok(())
}

/// Pull a theme to a local JSON file.
pub fn theme_pull(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    local_path: Option<&Path>,
    force: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;

    // Unwrap {"theme": {...}} envelope if present
    let theme = response.get("theme").unwrap_or(&response);

    let path = match local_path {
        Some(p) => p.to_path_buf(),
        None => {
            let name_slug = theme
                .get("name")
                .and_then(|v| v.as_str())
                .map(slugify)
                .unwrap_or_else(|| format!("theme-{}", theme_id));
            let filename = format!("{}.json", name_slug);
            std::env::current_dir()
                .context("getting current directory")?
                .join(filename)
        }
    };

    let content = serde_json::to_string_pretty(theme).context("serializing theme to JSON")?;
    atomic_write_private(&path, content, force)?;
    println!("{}", path.display());
    Ok(())
}

/// Push a local JSON file to create or update a theme.
pub fn theme_push(
    config: &Config,
    discourse_name: &str,
    json_path: &Path,
    theme_id: Option<u64>,
    dry_run: bool,
    assume_yes: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let raw = std::fs::read_to_string(json_path)
        .with_context(|| format!("reading {}", json_path.display()))?;
    let parsed: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing JSON from {}", json_path.display()))?;

    // Unwrap {"theme": {...}} envelope if present
    let theme = if let Some(inner) = parsed.get("theme") {
        inner.clone()
    } else {
        parsed
    };

    let push_data = build_push_payload(&theme);

    if let Some(id) = theme_id {
        let current = client.fetch_theme(id)?;
        let current = extract_theme(&current);
        let name = current
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if dry_run {
            println!(
                "[dry-run] {}: would update theme {} (\"{}\") from {}",
                discourse.name,
                id,
                name,
                json_path.display()
            );
            return Ok(());
        }
        if !assume_yes {
            return Err(anyhow!(
                "refusing to update live theme {} (\"{}\") without --yes; review with --dry-run first",
                id,
                name
            ));
        }
        client.update_theme(id, &push_data)?;
        println!("{}", id);
    } else {
        if let Some(file_id) = theme.get("id").and_then(Value::as_u64) {
            return Err(anyhow!(
                "theme file contains id {}; pass that ID explicitly to update, or remove id to create a new theme",
                file_id
            ));
        }
        if push_data
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
        {
            return Err(anyhow!(
                "missing name in theme file; set name or pass a theme ID to update"
            ));
        }
        if dry_run {
            let name = push_data
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            println!(
                "[dry-run] {}: would create theme \"{}\" from {}",
                discourse.name,
                name,
                json_path.display()
            );
            return Ok(());
        }
        let new_id = client.create_theme(&push_data)?;
        println!("{}", new_id);
    }

    Ok(())
}

/// Duplicate a theme and print the new theme ID.
pub fn theme_duplicate(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let response = client.fetch_theme(theme_id)?;
    let theme = response.get("theme").unwrap_or(&response);

    let original_name = theme
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let new_name = format!("Copy of {}", original_name);

    let mut push_data = build_push_payload(theme);
    push_data["name"] = Value::String(new_name);
    // Never copy the default status to the duplicate
    push_data["default"] = Value::Bool(false);

    let new_id = client.create_theme(&push_data)?;
    emit_result(format, &json!({ "id": new_id }), &new_id.to_string())
}

/// Build a payload suitable for creating or updating a theme.
/// Strips server-generated and read-only fields.
fn build_push_payload(theme: &Value) -> Value {
    let mut map = serde_json::Map::new();
    for key in &[
        "name",
        "enabled",
        "user_selectable",
        "color_scheme_id",
        "theme_fields",
        "component",
    ] {
        if let Some(val) = theme.get(key) {
            map.insert(key.to_string(), val.clone());
        }
    }
    Value::Object(map)
}

fn ssh_target(discourse: &DiscourseConfig) -> String {
    discourse
        .ssh_host
        .clone()
        .unwrap_or_else(|| discourse.name.clone())
}

// ---------------------------------------------------------------------------
// Phase 1: component settings + enable/disable + attach/detach
// (spec/theme-management.md). Themes are handled as raw JSON values, matching
// the rest of this module.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ThemeSettingEntry {
    setting: String,
    #[serde(rename = "type")]
    kind: String,
    value: Value,
    default: Value,
}

/// Unwrap the `{ "theme": { … } }` envelope returned by some endpoints,
/// falling back to the bare object.
fn extract_theme(value: &Value) -> &Value {
    value.get("theme").unwrap_or(value)
}

/// Render a setting value for human-readable (text) output: strings bare,
/// null as empty, everything else as compact JSON.
fn value_display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn theme_setting_entries(theme: &Value) -> Vec<ThemeSettingEntry> {
    theme
        .get("settings")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|s| ThemeSettingEntry {
                    setting: s
                        .get("setting")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    kind: s
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    value: s.get("value").cloned().unwrap_or(Value::Null),
                    default: s.get("default").cloned().unwrap_or(Value::Null),
                })
                .collect()
        })
        .unwrap_or_default()
}

// ─── theme setting pull/push file format ───────────────────────────────────

/// On-disk snapshot of a theme/component's settings. `version` gates the
/// schema; the rest is a header plus the editable settings list.
#[derive(Debug, Serialize, Deserialize)]
struct ThemeSettingsFile {
    version: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    discourse_version: Option<String>,
    theme_id: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    theme_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pulled_at: Option<String>,
    settings: Vec<ThemeSettingsFileEntry>,
}

/// One setting in the snapshot. `type`/`default` are informational context for
/// the human editor and are ignored on push; only `setting` + `value` matter.
#[derive(Debug, Serialize, Deserialize)]
struct ThemeSettingsFileEntry {
    setting: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none", default)]
    kind: Option<String>,
    value: Value,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    default: Option<Value>,
}

/// JSON-schema list settings (e.g. `header_links`) arrive as a string whose
/// content is a JSON array/object. Expand that to the real structure so it is
/// editable as a list, not one escaped line. Anything else passes through
/// unchanged (plain strings like `var(--primary)` are left alone).
fn expand_json_list(v: &Value) -> Value {
    if let Value::String(s) = v
        && matches!(s.trim_start().as_bytes().first(), Some(b'[') | Some(b'{'))
        && let Ok(parsed) = serde_json::from_str::<Value>(s)
        && (parsed.is_array() || parsed.is_object())
    {
        return parsed;
    }
    v.clone()
}

/// Serialise a snapshot value to the string Discourse expects on
/// `PUT /admin/themes/:id/setting.json`. Arrays/objects (JSON-schema list
/// settings) become compact JSON text; scalars become their plain form. This
/// is deliberately NOT the site-settings `value_to_send_string`, which
/// pipe-joins arrays - theme list settings are JSON, not pipe-delimited.
fn theme_value_to_send(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Compare two wire-strings for equality. A JSON-list setting round-trips as
/// compact JSON from the file but the server stores it spaced, so compare the
/// parsed JSON when both sides parse; otherwise compare literally.
fn json_equal(a: &str, b: &str) -> bool {
    match (
        serde_json::from_str::<Value>(a),
        serde_json::from_str::<Value>(b),
    ) {
        (Ok(va), Ok(vb)) => va == vb,
        _ => a == b,
    }
}

/// Render a change for the `--dry-run` plan: short values inline, long ones
/// (the big link lists) summarised by length so the terminal isn't flooded.
/// Both sides are normalised first so a list's size delta reflects the real
/// edit, not the compact-vs-spaced JSON serialisation difference between the
/// file and the server.
fn describe_change(from: &str, to: &str) -> String {
    const MAX: usize = 80;
    let from = normalize_for_display(from);
    let to = normalize_for_display(to);
    if from.chars().count() <= MAX && to.chars().count() <= MAX {
        format!("{} -> {}", from, to)
    } else {
        format!("changed ({} -> {} chars)", from.len(), to.len())
    }
}

/// Re-serialise JSON arrays/objects to a canonical compact form so two sides
/// of a diff are measured alike; leave everything else untouched.
fn normalize_for_display(s: &str) -> String {
    match serde_json::from_str::<Value>(s) {
        Ok(v) if v.is_array() || v.is_object() => v.to_string(),
        _ => s.to_string(),
    }
}

fn is_json_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

/// List a theme/component's settings (distinct from site settings).
pub fn theme_setting_list(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);
    let entries = theme_setting_entries(theme);
    match format {
        ListFormat::Text => {
            if entries.is_empty() {
                println!("No settings found for theme {}.", theme_id);
                return Ok(());
            }
            for entry in &entries {
                println!("{} = {}", entry.setting, value_display(&entry.value));
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&entries)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&entries)?),
    }
    Ok(())
}

/// Print a single theme/component setting's current value.
pub fn theme_setting_get(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    key: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);
    let setting = theme
        .get("settings")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|s| s.get("setting").and_then(|v| v.as_str()) == Some(key))
        })
        .ok_or_else(|| not_found("theme setting", key))?;
    let value = setting.get("value").cloned().unwrap_or(Value::Null);
    emit_result(
        format,
        &json!({ "setting": key, "value": value }),
        &value_display(&value),
    )
}

/// Set a single theme/component setting. The value is sent verbatim, so a
/// JSON-schema list setting takes its JSON text directly.
pub fn theme_setting_set(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    key: &str,
    value: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    if dry_run {
        println!(
            "[dry-run] {}: would set theme {} setting {} = {}",
            discourse.name, theme_id, key, value
        );
        return Ok(());
    }
    client.set_theme_setting(theme_id, key, value)?;
    println!("{}: set theme {} setting {}", discourse.name, theme_id, key);
    Ok(())
}

/// Pull a theme/component's settings to a local file for offline editing.
///
/// JSON-schema list settings (e.g. `header_links`, `dropdown_links`) arrive
/// from Discourse as a single string of escaped JSON; this expands them to
/// real arrays so they can be edited by hand rather than as one escaped line.
/// YAML by default; a `.json` destination writes JSON.
pub fn theme_setting_pull(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    local_path: Option<&Path>,
    force: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);
    let theme_name = theme
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let settings: Vec<ThemeSettingsFileEntry> = theme_setting_entries(theme)
        .into_iter()
        .map(|e| ThemeSettingsFileEntry {
            setting: e.setting,
            kind: if e.kind.is_empty() {
                None
            } else {
                Some(e.kind)
            },
            value: expand_json_list(&e.value),
            default: match &e.default {
                Value::Null => None,
                Value::String(s) if s.is_empty() => None,
                other => Some(expand_json_list(other)),
            },
        })
        .collect();

    let path = match local_path {
        Some(p) => p.to_path_buf(),
        None => {
            let slug = theme_name
                .as_deref()
                .map(slugify)
                .unwrap_or_else(|| format!("theme-{}", theme_id));
            std::env::current_dir()
                .context("getting current directory")?
                .join(format!("{}-settings.yml", slug))
        }
    };

    let file = ThemeSettingsFile {
        version: 1,
        discourse_version: client.fetch_version().ok().flatten(),
        theme_id,
        theme_name,
        pulled_at: Some(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        settings,
    };

    let content = if is_json_path(&path) {
        serde_json::to_string_pretty(&file).context("serializing theme settings as JSON")?
    } else {
        serde_yaml::to_string(&file).context("serializing theme settings as YAML")?
    };
    atomic_write_private(&path, &content, force)?;

    let n = file.settings.len();
    println!(
        "Wrote {} setting{} to {}",
        n,
        if n == 1 { "" } else { "s" },
        path.display()
    );
    Ok(())
}

/// Push a settings file back to a theme/component, PUTting only the settings
/// whose value differs from the server (idempotent). Re-serialises expanded
/// JSON-list settings back to the escaped-string form Discourse expects.
/// Honours `--dry-run`.
pub fn theme_setting_push(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    local_path: &Path,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let raw = std::fs::read_to_string(local_path)
        .with_context(|| format!("reading {}", local_path.display()))?;
    let file: ThemeSettingsFile = if is_json_path(local_path) {
        serde_json::from_str(&raw).context("parsing theme settings file as JSON")?
    } else {
        serde_yaml::from_str(&raw).context("parsing theme settings file as YAML")?
    };
    if file.version != 1 {
        return Err(anyhow!(
            "unsupported theme settings file schema version {} (expected 1)",
            file.version
        ));
    }

    // Current server values, to PUT only what actually changed.
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);
    let server = theme_setting_entries(theme);
    let current_by_name: std::collections::HashMap<&str, &Value> = server
        .iter()
        .map(|e| (e.setting.as_str(), &e.value))
        .collect();

    let mut changes: Vec<(String, String, String)> = Vec::new();
    let mut unchanged = 0usize;
    for entry in &file.settings {
        let desired = theme_value_to_send(&entry.value);
        match current_by_name.get(entry.setting.as_str()) {
            None => eprintln!(
                "warning: setting `{}` not found on theme {}; skipping",
                entry.setting, theme_id
            ),
            Some(current_value) => {
                let current = theme_value_to_send(current_value);
                if json_equal(&desired, &current) {
                    unchanged += 1;
                } else {
                    changes.push((entry.setting.clone(), current, desired));
                }
            }
        }
    }

    if changes.is_empty() {
        println!(
            "{}: theme {} already up to date ({} setting{} checked)",
            discourse.name,
            theme_id,
            unchanged,
            if unchanged == 1 { "" } else { "s" }
        );
        return Ok(());
    }

    if dry_run {
        println!(
            "[dry-run] {}: would update {} setting{} on theme {}:",
            discourse.name,
            changes.len(),
            if changes.len() == 1 { "" } else { "s" },
            theme_id
        );
        for (name, from, to) in &changes {
            println!("  {}: {}", name, describe_change(from, to));
        }
        return Ok(());
    }
    for (name, _from, to) in &changes {
        client.set_theme_setting(theme_id, name, to)?;
        println!("  set {}", name);
    }
    println!(
        "{}: updated {} setting{} on theme {}",
        discourse.name,
        changes.len(),
        if changes.len() == 1 { "" } else { "s" },
        theme_id
    );
    Ok(())
}

// ─── theme field (raw theme_fields: SCSS, head_tag, ...) ───────────────────

#[derive(Debug, Serialize)]
struct ThemeFieldEntry {
    field: String,
    #[serde(rename = "type")]
    kind: String,
    bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_url: Option<String>,
}

/// Discourse `ThemeField` type ids (see `ThemeField.types`).
fn field_type_label(type_id: i64) -> &'static str {
    match type_id {
        0 => "html",
        1 => "scss",
        2 => "upload",
        3 => "yaml",
        4 => "js",
        _ => "other",
    }
}

/// Sensible file extension for a pulled field body.
fn field_extension(type_id: i64) -> &'static str {
    match type_id {
        1 => "scss",
        0 => "html",
        3 => "yaml",
        4 => "js",
        _ => "txt",
    }
}

/// Best-effort `type_id` for a *new* field from its name (only used when the
/// field doesn't already exist server-side; the normal edit path reuses the
/// existing field's type).
fn infer_type_id(name: &str) -> i64 {
    if name.contains("scss") || name == "color_definitions" {
        1
    } else if name.ends_with("js") {
        4
    } else if name == "yaml" || name == "settings" {
        3
    } else {
        0
    }
}

/// Split a `target/name` field spec. A spec with no `/` is treated as a bare
/// name with an empty target (some fields have no target).
fn split_target_name(spec: &str) -> (String, String) {
    match spec.split_once('/') {
        Some((t, n)) => (t.to_string(), n.to_string()),
        None => (String::new(), spec.to_string()),
    }
}

fn find_theme_field<'a>(theme: &'a Value, target: &str, name: &str) -> Option<&'a Value> {
    theme
        .get("theme_fields")
        .and_then(|v| v.as_array())?
        .iter()
        .find(|f| {
            f.get("name").and_then(|v| v.as_str()) == Some(name)
                && f.get("target").and_then(|v| v.as_str()).unwrap_or("") == target
        })
}

/// The `remote_theme` object, but only when it's a git-backed remote (the case
/// where the DB is not the source of truth and where `theme update` applies).
fn git_remote_theme(theme: &Value) -> Option<&Value> {
    let rt = theme.get("remote_theme").filter(|v| !v.is_null())?;
    rt.get("is_git")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        .then_some(rt)
}

fn short_hash(h: &str) -> String {
    h.chars().take(8).collect()
}

fn theme_field_entries(theme: &Value) -> Vec<ThemeFieldEntry> {
    theme
        .get("theme_fields")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|f| {
                    let name = f.get("name").and_then(|v| v.as_str())?;
                    let target = f.get("target").and_then(|v| v.as_str()).unwrap_or("");
                    let type_id = f.get("type_id").and_then(|v| v.as_i64()).unwrap_or(-1);
                    let value = f.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    let field = if target.is_empty() {
                        name.to_string()
                    } else {
                        format!("{}/{}", target, name)
                    };
                    Some(ThemeFieldEntry {
                        field,
                        kind: field_type_label(type_id).to_string(),
                        bytes: value.len(),
                        upload_url: f
                            .get("url")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// List a theme's editable fields (`target/name`, type, size).
pub fn theme_field_list(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);
    let entries = theme_field_entries(theme);
    match format {
        ListFormat::Text => {
            if entries.is_empty() {
                println!("No editable fields for theme {}.", theme_id);
                return Ok(());
            }
            for e in &entries {
                match &e.upload_url {
                    Some(url) => println!("{}  ({}, upload -> {})", e.field, e.kind, url),
                    None => println!("{}  ({}, {} bytes)", e.field, e.kind, e.bytes),
                }
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&entries)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&entries)?),
    }
    Ok(())
}

/// Pull one field's body (e.g. `common/scss`) to a local file.
pub fn theme_field_pull(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    field_spec: &str,
    local_path: Option<&Path>,
    force: bool,
) -> Result<()> {
    let (target, name) = split_target_name(field_spec);
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);
    let field = find_theme_field(theme, &target, &name).ok_or_else(|| {
        anyhow!(
            "theme {} has no field `{}` (see `dsc theme field list {}`)",
            theme_id,
            field_spec,
            discourse_name
        )
    })?;
    let type_id = field.get("type_id").and_then(|v| v.as_i64()).unwrap_or(-1);
    if type_id == 2 {
        return Err(anyhow!(
            "`{}` is an upload var, not a text field; use `dsc theme asset`",
            field_spec
        ));
    }
    let value = field.get("value").and_then(|v| v.as_str()).unwrap_or("");

    let path = match local_path {
        Some(p) => p.to_path_buf(),
        None => {
            let base = if target.is_empty() {
                name.clone()
            } else {
                format!("{}-{}", target, name)
            };
            std::env::current_dir()
                .context("getting current directory")?
                .join(format!("{}.{}", base, field_extension(type_id)))
        }
    };
    atomic_write_private(&path, value, force)?;
    println!(
        "Wrote {} ({} bytes) to {}",
        field_spec,
        value.len(),
        path.display()
    );
    Ok(())
}

/// Push a local file back to one field. Refuses git-backed remote themes, where
/// the repo (not the DB) owns the field.
pub fn theme_field_push(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    field_spec: &str,
    local_path: &Path,
    dry_run: bool,
    assume_yes: bool,
) -> Result<()> {
    let (target, name) = split_target_name(field_spec);
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);

    if let Some(rt) = git_remote_theme(theme) {
        let url = rt
            .get("remote_url")
            .and_then(|v| v.as_str())
            .unwrap_or("its git repo");
        return Err(anyhow!(
            "theme {} is a git-backed remote component (from {}); its fields are owned by the \
             repo, not the site. Edit upstream and `dsc theme update`, or `dsc theme duplicate` \
             it first to get an editable copy.",
            theme_id,
            url
        ));
    }

    let existing = find_theme_field(theme, &target, &name);
    let type_id = existing
        .and_then(|f| f.get("type_id").and_then(|v| v.as_i64()))
        .unwrap_or_else(|| infer_type_id(&name));
    let old_value = existing
        .and_then(|f| f.get("value").and_then(|v| v.as_str()))
        .unwrap_or("");
    let new_value = std::fs::read_to_string(local_path)
        .with_context(|| format!("reading {}", local_path.display()))?;

    if new_value == old_value {
        println!(
            "{}: theme {} field {} unchanged",
            discourse.name, theme_id, field_spec
        );
        return Ok(());
    }
    if dry_run {
        let verb = if existing.is_some() {
            "update"
        } else {
            "create"
        };
        println!(
            "[dry-run] {}: would {} theme {} field {} ({} -> {} bytes)",
            discourse.name,
            verb,
            theme_id,
            field_spec,
            old_value.len(),
            new_value.len()
        );
        return Ok(());
    }
    if !assume_yes {
        return Err(anyhow!(
            "refusing to update live theme {} field {} without --yes; review with --dry-run first",
            theme_id,
            field_spec
        ));
    }

    // A single-entry `theme_fields` array upserts just this field, leaving the
    // theme's other fields untouched.
    let body = json!({
        "theme_fields": [{ "target": target, "name": name, "value": new_value, "type_id": type_id }]
    });
    client.update_theme(theme_id, &body)?;
    println!(
        "{}: updated theme {} field {} ({} bytes)",
        discourse.name,
        theme_id,
        field_spec,
        new_value.len()
    );
    Ok(())
}

// ─── theme asset (upload + bind a theme_upload_var) ────────────────────────

#[derive(Debug, Serialize)]
struct ThemeAssetEntry {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

/// List a theme's bound upload assets (the `$var` uploads).
pub fn theme_asset_list(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);
    let assets: Vec<ThemeAssetEntry> = theme
        .get("theme_fields")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|f| f.get("type_id").and_then(|v| v.as_i64()) == Some(2))
                .filter_map(|f| {
                    let name = f.get("name").and_then(|v| v.as_str())?.to_string();
                    Some(ThemeAssetEntry {
                        name,
                        filename: f
                            .get("filename")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        url: f.get("url").and_then(|v| v.as_str()).map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    match format {
        ListFormat::Text => {
            if assets.is_empty() {
                println!("No upload assets bound to theme {}.", theme_id);
                return Ok(());
            }
            for a in &assets {
                println!(
                    "${}  {}  {}",
                    a.name,
                    a.filename.as_deref().unwrap_or(""),
                    a.url.as_deref().unwrap_or("")
                );
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&assets)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&assets)?),
    }
    Ok(())
}

/// Upload a file and bind it to a theme upload var (`$name`) in one step.
pub fn theme_asset_set(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    var_name: &str,
    file: &Path,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    if dry_run {
        println!(
            "[dry-run] {}: would upload {} and bind it to theme {} as ${}",
            discourse.name,
            file.display(),
            theme_id,
            var_name
        );
        return Ok(());
    }
    let info = client.upload_file(file, "theme")?;
    // Upload vars live on the `common` target.
    let body = json!({
        "theme_fields": [{
            "target": "common",
            "name": var_name,
            "type_id": 2,
            "upload_id": info.id,
            "value": ""
        }]
    });
    client.update_theme(theme_id, &body)?;
    println!(
        "{}: bound ${} on theme {} -> {} (upload {})",
        discourse.name, var_name, theme_id, info.url, info.id
    );
    Ok(())
}

/// Remove a theme upload var binding. Clearing the field's value and upload
/// deletes the `theme_upload_var` field server-side.
pub fn theme_asset_unset(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    var_name: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);
    if find_theme_field(theme, "common", var_name).is_none() {
        return Err(anyhow!(
            "theme {} has no asset ${} (see `dsc theme asset list {}`)",
            theme_id,
            var_name,
            discourse_name
        ));
    }
    if dry_run {
        println!(
            "[dry-run] {}: would unbind ${} from theme {}",
            discourse.name, var_name, theme_id
        );
        return Ok(());
    }
    let body = json!({
        "theme_fields": [{
            "target": "common",
            "name": var_name,
            "type_id": 2,
            "value": "",
            "upload_id": null
        }]
    });
    client.update_theme(theme_id, &body)?;
    println!(
        "{}: unbound ${} from theme {}",
        discourse.name, var_name, theme_id
    );
    Ok(())
}

// ─── theme update (remote/git-backed component refresh) ────────────────────

/// Pull a git-backed remote component to its latest upstream commit. With
/// `check`, only report how far behind it is without pulling.
pub fn theme_update(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    check: bool,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);
    let rt = git_remote_theme(theme).ok_or_else(|| {
        anyhow!(
            "theme {} is not a git-backed remote component; nothing to update \
             (locally-authored themes have no upstream to pull from)",
            theme_id
        )
    })?;
    let remote_url = rt
        .get("remote_url")
        .and_then(|v| v.as_str())
        .unwrap_or("its upstream")
        .to_string();
    let before = rt
        .get("local_version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if check || dry_run {
        // Refresh the upstream comparison without pulling.
        let resp = client.put_theme_flag(theme_id, "remote_check")?;
        let behind = git_remote_theme(extract_theme(&resp))
            .and_then(|r| r.get("commits_behind").and_then(|v| v.as_i64()))
            .unwrap_or(0);
        if behind > 0 {
            println!(
                "{}: theme {} is {} commit{} behind {} (run `dsc theme update {} {}` to pull)",
                discourse.name,
                theme_id,
                behind,
                if behind == 1 { "" } else { "s" },
                remote_url,
                discourse_name,
                theme_id
            );
        } else {
            println!(
                "{}: theme {} is up to date with {}",
                discourse.name, theme_id, remote_url
            );
        }
        return Ok(());
    }

    let resp = client.put_theme_flag(theme_id, "remote_update")?;
    let after = git_remote_theme(extract_theme(&resp))
        .and_then(|r| r.get("local_version").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    if !after.is_empty() && after != before {
        println!(
            "{}: updated theme {} {} -> {}",
            discourse.name,
            theme_id,
            short_hash(&before),
            short_hash(&after)
        );
    } else {
        println!(
            "{}: theme {} already up to date ({})",
            discourse.name,
            theme_id,
            short_hash(&after)
        );
    }
    Ok(())
}

/// Enable or disable a theme/component (`PUT /admin/themes/:id.json` toggling
/// the `enabled` boolean).
pub fn theme_set_enabled(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    enabled: bool,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let action = if enabled { "enable" } else { "disable" };
    if dry_run {
        println!(
            "[dry-run] {}: would {} theme {}",
            discourse.name, action, theme_id
        );
        return Ok(());
    }
    client.update_theme(theme_id, &json!({ "enabled": enabled }))?;
    println!("{}: {}d theme {}", discourse.name, action, theme_id);
    Ok(())
}

/// Attach or detach a component to/from a parent theme. Reads the parent's
/// current `child_themes`, adds/removes the component id, and PUTs the full
/// replacement `child_theme_ids` set (disabled components stay in the list).
pub fn theme_set_child(
    config: &Config,
    discourse_name: &str,
    parent_id: u64,
    component_id: u64,
    attach: bool,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(parent_id)?;
    let theme = extract_theme(&response);
    let mut child_ids: Vec<u64> = theme
        .get("child_themes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("id").and_then(|v| v.as_u64()))
                .collect()
        })
        .unwrap_or_default();

    let present = child_ids.contains(&component_id);
    if attach && present {
        println!(
            "{}: component {} already attached to theme {}",
            discourse.name, component_id, parent_id
        );
        return Ok(());
    }
    if !attach && !present {
        println!(
            "{}: component {} is not attached to theme {}",
            discourse.name, component_id, parent_id
        );
        return Ok(());
    }
    if attach {
        child_ids.push(component_id);
    } else {
        child_ids.retain(|&id| id != component_id);
    }

    let (verb, prep) = if attach {
        ("attach", "to")
    } else {
        ("detach", "from")
    };
    if dry_run {
        println!(
            "[dry-run] {}: would {} component {} {} theme {} (child_theme_ids -> {:?})",
            discourse.name, verb, component_id, prep, parent_id, child_ids
        );
        return Ok(());
    }
    client.update_theme(parent_id, &json!({ "child_theme_ids": child_ids }))?;
    println!(
        "{}: {}ed component {} {} theme {}",
        discourse.name, verb, component_id, prep, parent_id
    );
    Ok(())
}

#[derive(Debug, Serialize)]
struct ThemeRelation {
    id: u64,
    name: String,
}

#[derive(Debug, Serialize)]
struct ThemeShow {
    id: u64,
    name: String,
    component: bool,
    enabled: bool,
    default: bool,
    user_selectable: bool,
    color_scheme_id: Option<u64>,
    parent_themes: Vec<ThemeRelation>,
    child_themes: Vec<ThemeRelation>,
    settings_count: usize,
    fields: Vec<String>,
}

/// Parse an array of `{id, name}` theme relations (child/parent themes),
/// skipping entries missing an id.
fn theme_relations(theme: &Value, key: &str) -> Vec<ThemeRelation> {
    theme
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    let id = r.get("id").and_then(|v| v.as_u64())?;
                    let name = r
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    Some(ThemeRelation { id, name })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Inventory of editable `theme_fields` as `target/name` strings (e.g.
/// `common/scss`). Parsed defensively so an unexpected entry shape just
/// contributes nothing rather than erroring.
fn theme_field_inventory(theme: &Value) -> Vec<String> {
    theme
        .get("theme_fields")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|f| {
                    let name = f.get("name").and_then(|v| v.as_str())?;
                    let target = f.get("target").and_then(|v| v.as_str()).unwrap_or("");
                    if target.is_empty() {
                        Some(name.to_string())
                    } else {
                        Some(format!("{}/{}", target, name))
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn build_theme_show(theme: &Value, theme_id: u64) -> ThemeShow {
    ThemeShow {
        id: theme.get("id").and_then(|v| v.as_u64()).unwrap_or(theme_id),
        name: theme
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        component: theme
            .get("component")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        enabled: theme
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        default: theme
            .get("default")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        user_selectable: theme
            .get("user_selectable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        color_scheme_id: theme.get("color_scheme_id").and_then(|v| v.as_u64()),
        parent_themes: theme_relations(theme, "parent_themes"),
        child_themes: theme_relations(theme, "child_themes"),
        settings_count: theme_setting_entries(theme).len(),
        fields: theme_field_inventory(theme),
    }
}

fn format_relations(rels: &[ThemeRelation]) -> String {
    if rels.is_empty() {
        "(none)".to_string()
    } else {
        rels.iter()
            .map(|r| format!("{} - {}", r.id, r.name))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Show a richer view of one theme/component than `theme list`: type, enabled
/// and default flags, parents, attached children, settings count, and the
/// editable field inventory.
pub fn theme_show(
    config: &Config,
    discourse_name: &str,
    theme_id: u64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.fetch_theme(theme_id)?;
    let theme = extract_theme(&response);
    let show = build_theme_show(theme, theme_id);
    match format {
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&show)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&show)?),
        ListFormat::Text => {
            println!("{} - {}", show.id, show.name);
            println!(
                "  type:            {}",
                if show.component { "component" } else { "theme" }
            );
            println!("  enabled:         {}", show.enabled);
            println!("  default:         {}", show.default);
            println!("  user-selectable: {}", show.user_selectable);
            if let Some(cs) = show.color_scheme_id {
                println!("  color scheme:    {}", cs);
            }
            println!(
                "  parents:         {}",
                format_relations(&show.parent_themes)
            );
            println!(
                "  children:        {}",
                format_relations(&show.child_themes)
            );
            println!("  settings:        {}", show.settings_count);
            let fields = if show.fields.is_empty() {
                "(none)".to_string()
            } else {
                show.fields.join(", ")
            };
            println!("  fields:          {}", fields);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_theme_unwraps_envelope_and_passes_bare() {
        let wrapped = json!({ "theme": { "id": 11, "name": "kitchen" } });
        assert_eq!(
            extract_theme(&wrapped).get("id").and_then(|v| v.as_u64()),
            Some(11)
        );
        let bare = json!({ "id": 7, "name": "bare" });
        assert_eq!(
            extract_theme(&bare).get("id").and_then(|v| v.as_u64()),
            Some(7)
        );
    }

    #[test]
    fn value_display_renders_each_json_kind() {
        assert_eq!(value_display(&json!("right")), "right");
        assert_eq!(value_display(&Value::Null), "");
        assert_eq!(value_display(&json!(true)), "true");
        assert_eq!(value_display(&json!(42)), "42");
        // json-schema list settings arrive as a JSON string already; an actual
        // array still renders as compact JSON for text output.
        assert_eq!(value_display(&json!(["a", "b"])), "[\"a\",\"b\"]");
    }

    #[test]
    fn theme_setting_entries_parses_settings_array() {
        let theme = json!({
            "settings": [
                { "setting": "links_position", "type": "enum", "default": "right", "value": "left" },
                { "setting": "header_links", "type": "string", "default": "[]", "value": "[{\"id\":1}]" }
            ]
        });
        let entries = theme_setting_entries(&theme);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].setting, "links_position");
        assert_eq!(entries[0].kind, "enum");
        assert_eq!(value_display(&entries[0].value), "left");
        assert_eq!(entries[1].setting, "header_links");
        assert_eq!(value_display(&entries[1].value), "[{\"id\":1}]");
    }

    #[test]
    fn theme_setting_entries_empty_when_absent() {
        assert!(theme_setting_entries(&json!({ "name": "no settings" })).is_empty());
    }

    #[test]
    fn expand_json_list_expands_only_json_arrays_and_objects() {
        // The header_links shape: a string holding a JSON array -> real array.
        let v = expand_json_list(&json!("[{\"id\": 1, \"title\": \"A\"}]"));
        assert!(v.is_array());
        assert_eq!(v[0]["title"], json!("A"));
        // A JSON object string -> object.
        assert!(expand_json_list(&json!("{\"a\": 1}")).is_object());
        // Plain strings (CSS vars, enums) are left alone.
        assert_eq!(
            expand_json_list(&json!("var(--primary)")),
            json!("var(--primary)")
        );
        assert_eq!(expand_json_list(&json!("left")), json!("left"));
        // Non-strings pass through.
        assert_eq!(expand_json_list(&json!(true)), json!(true));
        // Starts with '[' but isn't valid JSON -> stays a string.
        assert_eq!(expand_json_list(&json!("[not json")), json!("[not json"));
    }

    #[test]
    fn theme_value_to_send_serialises_lists_as_json_text() {
        assert_eq!(theme_value_to_send(&json!([{"id": 1}])), "[{\"id\":1}]");
        assert_eq!(theme_value_to_send(&json!("left")), "left");
        assert_eq!(theme_value_to_send(&json!(true)), "true");
        assert_eq!(theme_value_to_send(&Value::Null), "");
    }

    #[test]
    fn json_equal_ignores_whitespace_for_lists() {
        // Server stores spaced JSON; the file round-trips to compact JSON.
        assert!(json_equal("[{\"id\": 1}]", "[{\"id\":1}]"));
        assert!(json_equal("left", "left"));
        assert!(!json_equal("[{\"id\": 1}]", "[{\"id\":2}]"));
        assert!(!json_equal("split", "left"));
    }

    #[test]
    fn header_links_round_trips_idempotently() {
        // A realistic server value: a spaced JSON string, as Discourse returns it.
        let server = json!("[{\"id\": 1, \"title\": \"Conference\", \"newTab\": true}]");
        // pull: expand to an editable array.
        let expanded = expand_json_list(&server);
        assert!(expanded.is_array());
        // push (unedited): the array serialises back and compares equal to the
        // server's spaced form, so an untouched list is never needlessly PUT.
        let current = theme_value_to_send(&server);
        assert!(
            json_equal(&theme_value_to_send(&expanded), &current),
            "an untouched list must be a no-op on push"
        );
        // Edit one title -> it now differs and would be pushed.
        let mut edited = expanded.clone();
        edited[0]["title"] = json!("Conference 2027");
        assert!(!json_equal(&theme_value_to_send(&edited), &current));
    }

    #[test]
    fn theme_settings_file_round_trips_through_yaml() {
        let file = ThemeSettingsFile {
            version: 1,
            discourse_version: Some("3.x".into()),
            theme_id: 17,
            theme_name: Some("Dropdown Header".into()),
            pulled_at: None,
            settings: vec![ThemeSettingsFileEntry {
                setting: "header_links".into(),
                kind: Some("string".into()),
                value: json!([{"id": 1, "title": "A"}]),
                default: None,
            }],
        };
        let yaml = serde_yaml::to_string(&file).unwrap();
        let back: ThemeSettingsFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back.version, 1);
        assert_eq!(back.theme_id, 17);
        assert_eq!(back.settings.len(), 1);
        assert_eq!(back.settings[0].setting, "header_links");
        assert!(back.settings[0].value.is_array());
        assert_eq!(back.settings[0].value[0]["title"], json!("A"));
    }

    #[test]
    fn describe_change_summarises_long_values() {
        assert_eq!(describe_change("split", "left"), "split -> left");
        let long = "x".repeat(200);
        assert!(describe_change(&long, &long).starts_with("changed ("));
    }

    #[test]
    fn split_target_name_handles_slash_and_bare() {
        assert_eq!(
            split_target_name("common/scss"),
            ("common".into(), "scss".into())
        );
        assert_eq!(
            split_target_name("settings/yaml"),
            ("settings".into(), "yaml".into())
        );
        // No slash -> empty target, whole thing is the name.
        assert_eq!(split_target_name("scss"), (String::new(), "scss".into()));
    }

    #[test]
    fn field_type_and_extension_map_ids() {
        assert_eq!(field_type_label(1), "scss");
        assert_eq!(field_type_label(0), "html");
        assert_eq!(field_type_label(2), "upload");
        assert_eq!(field_extension(1), "scss");
        assert_eq!(field_extension(0), "html");
        assert_eq!(field_extension(2), "txt");
    }

    #[test]
    fn infer_type_id_from_name() {
        assert_eq!(infer_type_id("scss"), 1);
        assert_eq!(infer_type_id("embedded_scss"), 1);
        assert_eq!(infer_type_id("extra_js"), 4);
        assert_eq!(infer_type_id("head_tag"), 0);
    }

    #[test]
    fn git_remote_theme_only_matches_git_backed() {
        // Locally authored: remote_theme is null.
        assert!(git_remote_theme(&json!({ "remote_theme": null })).is_none());
        assert!(git_remote_theme(&json!({ "name": "local" })).is_none());
        // Git-backed remote component.
        let git = json!({ "remote_theme": { "is_git": true, "remote_url": "https://x/y.git" } });
        assert!(git_remote_theme(&git).is_some());
        // A non-git remote (e.g. zip import) is not updatable.
        let zip = json!({ "remote_theme": { "is_git": false } });
        assert!(git_remote_theme(&zip).is_none());
    }

    #[test]
    fn theme_field_entries_parses_shape() {
        let theme = json!({
            "theme_fields": [
                { "target": "common", "name": "scss", "type_id": 1, "value": "body{}" },
                { "target": "common", "name": "logo", "type_id": 2, "value": "",
                  "url": "/uploads/logo.png", "filename": "logo.png" }
            ]
        });
        let entries = theme_field_entries(&theme);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].field, "common/scss");
        assert_eq!(entries[0].kind, "scss");
        assert_eq!(entries[0].bytes, 6);
        assert!(entries[0].upload_url.is_none());
        assert_eq!(entries[1].kind, "upload");
        assert_eq!(entries[1].upload_url.as_deref(), Some("/uploads/logo.png"));
    }

    #[test]
    fn find_theme_field_matches_target_and_name() {
        let theme = json!({
            "theme_fields": [
                { "target": "common", "name": "scss", "type_id": 1, "value": "a" },
                { "target": "desktop", "name": "scss", "type_id": 1, "value": "b" }
            ]
        });
        assert_eq!(
            find_theme_field(&theme, "desktop", "scss")
                .and_then(|f| f.get("value"))
                .and_then(|v| v.as_str()),
            Some("b")
        );
        assert!(find_theme_field(&theme, "mobile", "scss").is_none());
    }

    #[test]
    fn short_hash_takes_eight() {
        assert_eq!(short_hash("0f474e72e256f4dfcd6685"), "0f474e72");
        assert_eq!(short_hash("abc"), "abc");
    }

    #[test]
    fn looks_like_git_url_distinguishes_urls_from_paths() {
        assert!(looks_like_git_url("https://github.com/org/theme"));
        assert!(looks_like_git_url("http://x/y"));
        assert!(looks_like_git_url("git@github.com:org/theme.git"));
        assert!(looks_like_git_url("ssh://git@host/repo"));
        assert!(looks_like_git_url("/tmp/theme.git")); // .git suffix
        assert!(!looks_like_git_url("./my-theme.tar.gz"));
        assert!(!looks_like_git_url("/home/me/theme.zip"));
    }

    #[test]
    fn redact_url_hides_credentials() {
        assert_eq!(
            redact_url("https://user:token@github.com/org/private.git"),
            "https://***@github.com/org/private.git"
        );
        // No credentials -> unchanged.
        assert_eq!(
            redact_url("https://github.com/org/public"),
            "https://github.com/org/public"
        );
        assert_eq!(redact_url("./local.tar.gz"), "./local.tar.gz");
    }

    #[test]
    fn theme_relations_parses_id_name_pairs() {
        let theme = json!({
            "child_themes": [
                { "id": 8, "name": "Header Submenus" },
                { "id": 14, "name": "Dropdown Header" },
                { "name": "no id, skipped" }
            ]
        });
        let rels = theme_relations(&theme, "child_themes");
        assert_eq!(rels.len(), 2);
        assert_eq!(rels[0].id, 8);
        assert_eq!(rels[1].name, "Dropdown Header");
        assert!(theme_relations(&theme, "parent_themes").is_empty());
    }

    #[test]
    fn theme_field_inventory_joins_target_and_name() {
        let theme = json!({
            "theme_fields": [
                { "target": "common", "name": "scss", "value": "body{}" },
                { "target": "desktop", "name": "scss", "value": "" },
                { "target": "", "name": "extra_js", "value": "" },
                { "value": "no name, skipped" }
            ]
        });
        let fields = theme_field_inventory(&theme);
        assert_eq!(fields, vec!["common/scss", "desktop/scss", "extra_js"]);
    }

    #[test]
    fn build_theme_show_summarises_core_fields() {
        let theme = json!({
            "id": 11,
            "name": "kitchen-customisations",
            "component": false,
            "enabled": true,
            "default": false,
            "user_selectable": true,
            "child_themes": [{ "id": 14, "name": "Dropdown Header" }],
            "settings": [{ "setting": "links_position", "value": "left" }],
            "theme_fields": [{ "target": "common", "name": "scss", "value": "x" }]
        });
        let show = build_theme_show(&theme, 11);
        assert_eq!(show.id, 11);
        assert!(!show.component);
        assert!(show.enabled);
        assert_eq!(show.child_themes.len(), 1);
        assert_eq!(show.settings_count, 1);
        assert_eq!(show.fields, vec!["common/scss"]);
    }
}

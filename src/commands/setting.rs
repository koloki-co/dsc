// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{DiscourseClient, SiteSettingDetail};
use crate::cli::ListFormat;
use crate::commands::common::{
    emit_result, ensure_api_credentials, fleet_worker_count, parse_tags, run_fleet,
    select_discourse, selected_discourses,
};
use crate::config::{Config, DiscourseConfig};
use crate::utils::atomic_write_private;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Set a site setting. If `discourse_name` is given, only that discourse is updated.
/// Otherwise all discourses matching `tags` are updated.
pub fn set_site_setting(
    config: &Config,
    discourse_name: Option<&str>,
    setting: &str,
    value: &str,
    tags: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    if let Some(name) = discourse_name {
        let discourse = select_discourse(config, Some(name))?;
        ensure_api_credentials(discourse)?;
        if dry_run {
            println!(
                "[dry-run] {}: would set {} = {}",
                discourse.name, setting, value
            );
            return Ok(());
        }
        let client = DiscourseClient::new(discourse)?;
        client.update_site_setting(setting, value)?;
        println!("{}: updated {}", discourse.name, setting);
        return Ok(());
    }

    // No specific discourse - use tag filter across all discourses.
    let filter = tags.map(parse_tags).unwrap_or_default();

    let mut matched = 0;
    for discourse in config
        .discourse
        .iter()
        .filter(|d| matches_tag_filter(d, &filter))
    {
        matched += 1;
        ensure_api_credentials(discourse)?;
        if dry_run {
            println!(
                "[dry-run] {}: would set {} = {}",
                discourse.name, setting, value
            );
            continue;
        }
        let client = DiscourseClient::new(discourse)?;
        client.update_site_setting(setting, value)?;
        println!("{}: updated {}", discourse.name, setting);
    }

    if matched == 0 {
        return Err(anyhow!("no discourses matched the tag filter"));
    }

    Ok(())
}

/// Get the current value of a single site setting.
pub fn get_site_setting(
    config: &Config,
    discourse_name: &str,
    setting: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let value = client.fetch_site_setting(setting)?;
    emit_result(
        format,
        &serde_json::json!({ "setting": setting, "value": value }),
        &value,
    )
}

/// Does a discourse carry at least one of the tags in `filter`? An empty
/// filter matches every discourse. Shared by `setting set --tags` and
/// `setting audit --tags`.
fn matches_tag_filter(disc: &DiscourseConfig, filter: &[String]) -> bool {
    if filter.is_empty() {
        return true;
    }
    let Some(disc_tags) = disc.tags.as_ref() else {
        return false;
    };
    let disc_tags: Vec<String> = disc_tags.iter().map(|t| t.to_ascii_lowercase()).collect();
    filter.iter().any(|tag| {
        let tag = tag.to_ascii_lowercase();
        disc_tags.iter().any(|t| t == &tag)
    })
}

#[derive(Debug, Serialize)]
struct AuditRow {
    discourse: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Show the value of one setting across every configured forum (filtered by
/// `tags`). One unreachable or unauthenticated forum is reported inline rather
/// than aborting the whole audit. Distinct from `setting diff`, which compares
/// two specific sources across all settings.
pub fn audit_site_setting(
    config: &Config,
    setting: &str,
    tags: Option<&str>,
    format: ListFormat,
) -> Result<()> {
    let discourses = selected_discourses(config, None, tags)?;
    let setting_owned = setting.to_string();

    let rows: Vec<AuditRow> = run_fleet(
        &discourses,
        fleet_worker_count(None, discourses.len(), 8, false),
        move |d| match fetch_one_setting(d, &setting_owned) {
            Ok(value) => AuditRow {
                discourse: d.name.clone(),
                value: Some(value),
                error: None,
            },
            Err(e) => AuditRow {
                discourse: d.name.clone(),
                value: None,
                error: Some(e.to_string()),
            },
        },
        |_| {},
    );

    match format {
        ListFormat::Text => print!("{}", render_audit_text(setting, &rows)),
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&rows)?),
    }
    Ok(())
}

fn fetch_one_setting(discourse: &DiscourseConfig, setting: &str) -> Result<String> {
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    client.fetch_site_setting(setting)
}

/// Render an audit as an aligned `name  value` table followed by an agreement
/// summary (all agree / N distinct values / none returned). Pure, so it is
/// unit-tested without touching the network.
fn render_audit_text(setting: &str, rows: &[AuditRow]) -> String {
    if rows.is_empty() {
        return format!("No forums matched for setting '{}'.\n", setting);
    }
    let width = rows.iter().map(|r| r.discourse.len()).max().unwrap_or(0);
    let mut out = String::new();
    for row in rows {
        let cell = match (&row.value, &row.error) {
            (Some(v), _) => v.clone(),
            (None, Some(e)) => format!("<error: {}>", e),
            (None, None) => String::new(),
        };
        out.push_str(&format!(
            "{:width$}  {}\n",
            row.discourse,
            cell,
            width = width
        ));
    }
    let values: Vec<&String> = rows.iter().filter_map(|r| r.value.as_ref()).collect();
    let distinct: std::collections::BTreeSet<&str> = values.iter().map(|v| v.as_str()).collect();
    let summary = match (values.len(), distinct.len()) {
        (0, _) => format!("no forum returned a value for '{}'", setting),
        (n, 1) => format!("all {} forum(s) agree on '{}'", n, setting),
        (n, d) => format!(
            "{} distinct values for '{}' across {} forum(s)",
            d, setting, n
        ),
    };
    out.push_str(&format!("\n{}\n", summary));
    out
}

#[derive(Debug, Serialize)]
struct SettingEntry {
    setting: String,
    value: String,
    category: String,
}

/// List all site settings.
pub fn list_site_settings(
    config: &Config,
    discourse_name: &str,
    format: ListFormat,
    verbose: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let raw = client.list_site_settings()?;

    let settings_arr = raw
        .get("site_settings")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let entries: Vec<SettingEntry> = settings_arr
        .into_iter()
        .map(|entry| {
            let setting = entry
                .get("setting")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let value = match entry
                .get("value")
                .cloned()
                .unwrap_or(serde_json::Value::Null)
            {
                serde_json::Value::String(s) => s,
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            };
            let category = entry
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("uncategorized")
                .to_string();
            SettingEntry {
                setting,
                value,
                category,
            }
        })
        .collect();

    match format {
        ListFormat::Text => {
            if entries.is_empty() && !verbose {
                println!("No settings found.");
                return Ok(());
            }
            for e in &entries {
                println!("{} = {}", e.setting, e.value);
            }
        }
        ListFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
        ListFormat::Yaml => {
            print!("{}", serde_yaml::to_string(&entries)?);
        }
    }

    Ok(())
}

// ─── Pull (snapshot) ──────────────────────────────────────────────────────────

/// On-disk settings snapshot file (schema version 1).
///
/// Spec: `spec/setting-sync.md`. The file is self-documenting: it carries
/// each setting's default, type, category, and description so that a human
/// (or LLM) reading the file can understand each entry without consulting
/// the API. On `push`, only `name` and `value` are honoured.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SettingsFile {
    pub version: u32,
    /// True only when the pull contained the complete settings catalogue.
    /// Filtered snapshots must never be used with `--reset-unlisted`.
    #[serde(default)]
    pub complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discourse_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulled_at: Option<String>,
    #[serde(default)]
    pub settings: Vec<SettingsEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SettingsEntry {
    pub name: String,
    pub value: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// Renamed to avoid the Rust keyword. Serializes as `type`.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub setting_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Settings excluded from `pull` because they are computed/read-only on the
/// server. Keep the list small; unknown read-only settings are handled
/// gracefully by `push` (server returns 422, we warn and continue).
const READONLY_SETTINGS: &[&str] = &[];

/// Pull all site settings to a local file.
pub fn pull_settings(
    config: &Config,
    discourse_name: &str,
    local_path: &Path,
    changed_only: bool,
    category: Option<&str>,
    force: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let server = client.list_site_settings_detailed()?;
    let discourse_version = client.fetch_version().ok().flatten();

    let mut entries: Vec<SettingsEntry> = server
        .into_iter()
        .filter(|s| !READONLY_SETTINGS.contains(&s.setting.as_str()))
        .filter(|s| match category {
            Some(cat) => s.category.eq_ignore_ascii_case(cat),
            None => true,
        })
        .filter(|s| {
            if !changed_only {
                return true;
            }
            !values_equal(&s.value, &s.default)
        })
        .map(detail_to_entry)
        .collect();

    // Sort by category, then by name for stable diffs.
    entries.sort_by(|a, b| {
        let ca = a.category.as_deref().unwrap_or("");
        let cb = b.category.as_deref().unwrap_or("");
        ca.cmp(cb).then_with(|| a.name.cmp(&b.name))
    });

    let pulled_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let file = SettingsFile {
        version: 1,
        complete: !changed_only && category.is_none(),
        discourse_version,
        pulled_at: Some(pulled_at),
        settings: entries,
    };

    let content = if is_json_path(local_path) {
        serde_json::to_string_pretty(&file).context("serializing settings as JSON")?
    } else {
        serde_yaml::to_string(&file).context("serializing settings as YAML")?
    };

    atomic_write_private(local_path, &content, force)?;

    println!(
        "Wrote {} setting{} to {}",
        file.settings.len(),
        if file.settings.len() == 1 { "" } else { "s" },
        local_path.display()
    );
    Ok(())
}

fn detail_to_entry(d: SiteSettingDetail) -> SettingsEntry {
    SettingsEntry {
        name: d.setting,
        value: d.value,
        default: if d.default.is_null() {
            None
        } else {
            Some(d.default)
        },
        setting_type: empty_to_none(d.setting_type),
        category: empty_to_none(d.category),
        description: empty_to_none(d.description),
    }
}

fn empty_to_none(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

fn values_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    // Discourse returns numeric and boolean settings as JSON values; comparing
    // the parsed Value handles all simple types. For string-typed values that
    // happen to differ only in whitespace, leave the strict comparison; users
    // who hit edge cases can edit the snapshot directly.
    a == b
}

fn is_json_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

// ─── Push (apply) ─────────────────────────────────────────────────────────────

/// Apply a settings snapshot file to a Discourse.
///
/// Idempotent: only PUTs values that differ from the server. Settings present
/// in the file but unknown on the server are skipped with a warning. With
/// `--reset-unlisted`, settings present on the server but absent from the
/// file are reset to their `default` value.
pub fn push_settings(
    config: &Config,
    discourse_name: &str,
    local_path: &Path,
    reset_unlisted: bool,
    dry_run: bool,
    assume_yes: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let raw = fs::read_to_string(local_path)
        .with_context(|| format!("reading {}", local_path.display()))?;
    let file: SettingsFile = if is_json_path(local_path) {
        serde_json::from_str(&raw).context("parsing settings file as JSON")?
    } else {
        serde_yaml::from_str(&raw).context("parsing settings file as YAML")?
    };

    if file.version != 1 {
        return Err(anyhow!(
            "unsupported settings file schema version {} (expected 1)",
            file.version
        ));
    }
    if reset_unlisted && !file.complete {
        return Err(anyhow!(
            "--reset-unlisted requires a complete snapshot produced without --changed-only or --category"
        ));
    }

    let server = client.list_site_settings_detailed()?;
    let server_by_name: std::collections::HashMap<&str, &SiteSettingDetail> =
        server.iter().map(|s| (s.setting.as_str(), s)).collect();

    let mut plan: Vec<PushAction> = Vec::new();

    // File → server: change or unchanged.
    for entry in &file.settings {
        let Some(srv) = server_by_name.get(entry.name.as_str()) else {
            plan.push(PushAction::UnknownOnServer(entry.name.clone()));
            continue;
        };
        let desired = value_to_send_string(&entry.value);
        let current = value_to_send_string(&srv.value);
        if desired == current {
            plan.push(PushAction::Unchanged(entry.name.clone()));
        } else {
            plan.push(PushAction::Change {
                name: entry.name.clone(),
                from: current,
                to: desired,
            });
        }
    }

    // Server → file: reset_unlisted.
    if reset_unlisted {
        let in_file: std::collections::HashSet<&str> =
            file.settings.iter().map(|e| e.name.as_str()).collect();
        for srv in &server {
            if in_file.contains(srv.setting.as_str()) {
                continue;
            }
            if READONLY_SETTINGS.contains(&srv.setting.as_str()) {
                continue;
            }
            let current = value_to_send_string(&srv.value);
            let default = value_to_send_string(&srv.default);
            if current == default {
                continue;
            }
            plan.push(PushAction::Reset {
                name: srv.setting.clone(),
                from: current,
                to: default,
            });
        }
    }

    // Stable order for display.
    plan.sort_by(|a, b| a.name().cmp(b.name()));

    print_plan(&plan, &discourse.name, dry_run);

    if dry_run {
        return Ok(());
    }

    let reset_count = plan
        .iter()
        .filter(|action| matches!(action, PushAction::Reset { .. }))
        .count();
    if reset_count > 0 && !assume_yes {
        return Err(anyhow!(
            "refusing to reset {} unlisted setting{} without --yes; review the plan with --dry-run first",
            reset_count,
            if reset_count == 1 { "" } else { "s" }
        ));
    }

    // Apply.
    let mut applied = 0;
    let mut failed = 0;
    for action in &plan {
        match action {
            PushAction::Change { name, to, .. } | PushAction::Reset { name, to, .. } => {
                match client.update_site_setting(name, to) {
                    Ok(()) => {
                        applied += 1;
                    }
                    Err(err) => {
                        failed += 1;
                        eprintln!("  ! {}: failed: {}", name, err);
                    }
                }
            }
            PushAction::Unchanged(_) | PushAction::UnknownOnServer(_) => {}
        }
    }

    println!(
        "{}: applied {} setting{}{}",
        discourse.name,
        applied,
        if applied == 1 { "" } else { "s" },
        if failed > 0 {
            format!(", {} failed", failed)
        } else {
            String::new()
        }
    );
    if failed > 0 {
        return Err(anyhow!("{} setting(s) failed to apply", failed));
    }
    Ok(())
}

#[derive(Debug)]
enum PushAction {
    Change {
        name: String,
        from: String,
        to: String,
    },
    Reset {
        name: String,
        from: String,
        to: String,
    },
    Unchanged(String),
    UnknownOnServer(String),
}

impl PushAction {
    fn name(&self) -> &str {
        match self {
            PushAction::Change { name, .. }
            | PushAction::Reset { name, .. }
            | PushAction::Unchanged(name)
            | PushAction::UnknownOnServer(name) => name,
        }
    }
}

fn print_plan(plan: &[PushAction], discourse: &str, dry_run: bool) {
    let prefix = if dry_run { "[dry-run] " } else { "" };
    let changes = plan
        .iter()
        .filter(|a| matches!(a, PushAction::Change { .. } | PushAction::Reset { .. }))
        .count();
    let unchanged = plan
        .iter()
        .filter(|a| matches!(a, PushAction::Unchanged(_)))
        .count();
    let unknown = plan
        .iter()
        .filter(|a| matches!(a, PushAction::UnknownOnServer(_)))
        .count();

    println!(
        "{}Setting push plan for {}: {} change{}, {} unchanged, {} unknown",
        prefix,
        discourse,
        changes,
        if changes == 1 { "" } else { "s" },
        unchanged,
        unknown,
    );
    for action in plan {
        match action {
            PushAction::Change { name, from, to } => {
                println!("  ~ {}: {} → {}", name, quote(from), quote(to));
            }
            PushAction::Reset { name, from, to } => {
                println!(
                    "  - {}: {} → {} (reset to default)",
                    name,
                    quote(from),
                    quote(to)
                );
            }
            PushAction::Unchanged(name) => {
                println!("  = {}: (unchanged)", name);
            }
            PushAction::UnknownOnServer(name) => {
                println!("  ? {}: skipped (not found on server)", name);
            }
        }
    }
}

fn quote(s: &str) -> String {
    if s.is_empty() {
        "\"\"".to_string()
    } else {
        format!("\"{}\"", s)
    }
}

/// Convert a `serde_json::Value` to the string form expected by Discourse's
/// `PUT /admin/site_settings/{name}.json` endpoint. Discourse accepts strings
/// and coerces internally.
fn value_to_send_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        // Discourse list-type settings are pipe-separated strings on the wire.
        // If a user wrote a YAML/JSON array, join with "|" for compatibility.
        serde_json::Value::Array(arr) => arr
            .iter()
            .map(value_to_send_string)
            .collect::<Vec<_>>()
            .join("|"),
        serde_json::Value::Object(_) => v.to_string(),
    }
}

// ─── Diff (compare two sources) ───────────────────────────────────────────────

/// A canonical, source-agnostic snapshot of settings used by `diff_settings`.
struct DiffSource {
    label: String,
    entries: std::collections::HashMap<String, SettingsEntry>,
}

/// Compare site settings between two sources. Each source can be a Discourse
/// name (live fetch) or a path to a snapshot file produced by `pull`.
pub fn diff_settings(
    config: &Config,
    source: &str,
    target: &str,
    changed_only: bool,
    category: Option<&str>,
    format: ListFormat,
) -> Result<()> {
    let a = load_diff_source(config, source)?;
    let b = load_diff_source(config, target)?;

    // Union of keys.
    let mut names: std::collections::BTreeSet<String> = a.entries.keys().cloned().collect();
    names.extend(b.entries.keys().cloned());

    let mut rows: Vec<DiffRow> = Vec::new();
    for name in names {
        let ea = a.entries.get(&name);
        let eb = b.entries.get(&name);
        let va = ea.map(|e| value_to_send_string(&e.value));
        let vb = eb.map(|e| value_to_send_string(&e.value));
        if va == vb {
            continue;
        }
        // Category filter (uses whichever side has metadata).
        if let Some(cat) = category {
            let row_cat = ea
                .and_then(|e| e.category.as_deref())
                .or_else(|| eb.and_then(|e| e.category.as_deref()))
                .unwrap_or("");
            if !row_cat.eq_ignore_ascii_case(cat) {
                continue;
            }
        }
        // changed-only: filter to rows where at least one side has a value
        // that differs from its default. Treat an absent setting as "at
        // default" - this avoids drowning the diff in entries that one side
        // simply omitted because the snapshot was --changed-only.
        if changed_only {
            // Borrow the default from whichever side has metadata.
            let shared_default = ea
                .and_then(|e| e.default.as_ref())
                .or_else(|| eb.and_then(|e| e.default.as_ref()));
            let a_changed = match ea {
                Some(e) => shared_default.map(|d| &e.value != d).unwrap_or(true),
                None => false,
            };
            let b_changed = match eb {
                Some(e) => shared_default.map(|d| &e.value != d).unwrap_or(true),
                None => false,
            };
            if !a_changed && !b_changed {
                continue;
            }
        }
        rows.push(DiffRow {
            name,
            value_a: va,
            value_b: vb,
        });
    }

    print_diff(&rows, &a.label, &b.label, format)
}

#[derive(Debug, Serialize)]
struct DiffRow {
    name: String,
    #[serde(rename = "a")]
    value_a: Option<String>,
    #[serde(rename = "b")]
    value_b: Option<String>,
}

/// Resolve a source string to a canonical settings snapshot. Treats the
/// argument as a file path if it points to an existing file or has a
/// `.yaml`/`.yml`/`.json` extension; otherwise treats it as a Discourse name.
fn load_diff_source(config: &Config, src: &str) -> Result<DiffSource> {
    let path = Path::new(src);
    let looks_like_file = path.is_file()
        || matches!(
            path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase),
            Some(ref ext) if ext == "yaml" || ext == "yml" || ext == "json"
        );
    if looks_like_file {
        let raw =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let file: SettingsFile = if is_json_path(path) {
            serde_json::from_str(&raw).context("parsing settings file as JSON")?
        } else {
            serde_yaml::from_str(&raw).context("parsing settings file as YAML")?
        };
        let entries: std::collections::HashMap<String, SettingsEntry> = file
            .settings
            .into_iter()
            .map(|e| (e.name.clone(), e))
            .collect();
        return Ok(DiffSource {
            label: path.display().to_string(),
            entries,
        });
    }
    // Treat as discourse name.
    let discourse = select_discourse(config, Some(src))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let server = client.list_site_settings_detailed()?;
    let entries: std::collections::HashMap<String, SettingsEntry> = server
        .into_iter()
        .map(|d| {
            let entry = detail_to_entry(d);
            (entry.name.clone(), entry)
        })
        .collect();
    Ok(DiffSource {
        label: discourse.name.clone(),
        entries,
    })
}

fn print_diff(rows: &[DiffRow], label_a: &str, label_b: &str, format: ListFormat) -> Result<()> {
    match format {
        ListFormat::Text => {
            if rows.is_empty() {
                println!("{} and {}: no differences.", label_a, label_b);
                return Ok(());
            }
            println!(
                "{} differing setting{} between {} and {}:",
                rows.len(),
                if rows.len() == 1 { "" } else { "s" },
                label_a,
                label_b
            );
            for row in rows {
                println!("  {}", row.name);
                println!("    {}: {}", label_a, fmt_diff_value(&row.value_a));
                println!("    {}: {}", label_b, fmt_diff_value(&row.value_b));
            }
        }
        ListFormat::Json => {
            let payload = serde_json::json!({
                "a": label_a,
                "b": label_b,
                "differences": rows,
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        ListFormat::Yaml => {
            let payload = serde_json::json!({
                "a": label_a,
                "b": label_b,
                "differences": rows,
            });
            print!("{}", serde_yaml::to_string(&payload)?);
        }
    }
    Ok(())
}

fn fmt_diff_value(v: &Option<String>) -> String {
    match v {
        Some(s) if s.is_empty() => "\"\"".to_string(),
        Some(s) => format!("\"{}\"", s),
        None => "(absent)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(name: &str, tags: Option<Vec<&str>>) -> DiscourseConfig {
        DiscourseConfig {
            name: name.to_string(),
            tags: tags.map(|t| t.into_iter().map(String::from).collect()),
            ..DiscourseConfig::default()
        }
    }

    fn row(name: &str, value: Option<&str>, error: Option<&str>) -> AuditRow {
        AuditRow {
            discourse: name.to_string(),
            value: value.map(String::from),
            error: error.map(String::from),
        }
    }

    #[test]
    fn tag_filter_empty_matches_all() {
        assert!(matches_tag_filter(&disc("a", None), &[]));
        assert!(matches_tag_filter(&disc("a", Some(vec!["prod"])), &[]));
    }

    #[test]
    fn tag_filter_matches_case_insensitively() {
        let filter = vec!["Prod".to_string()];
        assert!(matches_tag_filter(
            &disc("a", Some(vec!["prod", "eu"])),
            &filter
        ));
    }

    #[test]
    fn tag_filter_rejects_untagged_or_nonmatching() {
        let filter = vec!["prod".to_string()];
        assert!(!matches_tag_filter(&disc("a", None), &filter));
        assert!(!matches_tag_filter(
            &disc("a", Some(vec!["staging"])),
            &filter
        ));
    }

    #[test]
    fn audit_text_reports_agreement() {
        let rows = vec![
            row("forum-a", Some("My Forum"), None),
            row("forum-b", Some("My Forum"), None),
        ];
        let out = render_audit_text("title", &rows);
        assert!(out.contains("forum-a  My Forum"));
        assert!(
            out.contains("all 2 forum(s) agree on 'title'"),
            "got: {out}"
        );
    }

    #[test]
    fn audit_text_reports_distinct_values() {
        let rows = vec![row("a", Some("X"), None), row("b", Some("Y"), None)];
        let out = render_audit_text("title", &rows);
        assert!(
            out.contains("2 distinct values for 'title' across 2 forum(s)"),
            "got: {out}"
        );
    }

    #[test]
    fn audit_text_renders_errors_and_excludes_them_from_agreement() {
        let rows = vec![
            row("a", Some("X"), None),
            row("b", None, Some("auth failed")),
        ];
        let out = render_audit_text("title", &rows);
        assert!(out.contains("<error: auth failed>"));
        // Only one forum returned a value, so they trivially "agree".
        assert!(out.contains("all 1 forum(s) agree"), "got: {out}");
    }

    #[test]
    fn audit_text_empty_when_no_forums_match() {
        let out = render_audit_text("title", &[]);
        assert!(out.contains("No forums matched"));
    }
}

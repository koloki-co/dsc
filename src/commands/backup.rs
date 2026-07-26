// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::OutputFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::create_atomic_output;
use anyhow::{Context, Result, anyhow};
use std::io;
use std::path::Path;

pub fn backup_create(config: &Config, discourse_name: &str) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    client.create_backup()?;
    Ok(())
}

pub fn backup_list(
    config: &Config,
    discourse_name: &str,
    format: OutputFormat,
    verbose: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let response = client.list_backups()?;
    let mut backups = extract_backups(&response);
    backups.sort_by(|a, b| backup_created_at(b).cmp(&backup_created_at(a)));
    // The list endpoint doesn't report where backups live; that's the global
    // `backup_location` site setting (local vs s3). Best-effort and only when
    // there's something to label - a read failure just blanks the column
    // rather than failing the listing, and we skip the (heavy) settings fetch
    // entirely when there are no backups.
    let global_location = if backups.is_empty() {
        None
    } else {
        client
            .fetch_site_setting("backup_location")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| backup_location_response(&response))
    };

    match format {
        OutputFormat::Text => {
            if backups.is_empty() && !verbose {
                println!("No backups found.");
                return Ok(());
            }
            if let Some(latest) = backups.first() {
                let filename = backup_filename(latest);
                let created_at = backup_created_at(latest).unwrap_or("unknown");
                let location = backup_location(latest, global_location.as_deref());
                println!(
                    "Latest backup: {} - {} - {}",
                    filename, created_at, location
                );
            }
            for backup in &backups {
                let filename = backup_filename(backup);
                let created_at = backup_created_at(backup).unwrap_or("unknown");
                let size = backup_size(backup);
                let location = backup_location(backup, global_location.as_deref());
                println!("{} - {} - {} - {}", filename, created_at, size, location);
            }
        }
        OutputFormat::Markdown => {
            if let Some(latest) = backups.first() {
                let filename = backup_filename(latest);
                let created_at = backup_created_at(latest).unwrap_or("unknown");
                let location = backup_location(latest, global_location.as_deref());
                println!(
                    "Latest backup: {} ({}) - {}",
                    filename, created_at, location
                );
            }
            for backup in &backups {
                let filename = backup_filename(backup);
                let created_at = backup_created_at(backup).unwrap_or("unknown");
                let size = backup_size(backup);
                let location = backup_location(backup, global_location.as_deref());
                println!("- {} ({}) - {} - {}", filename, created_at, size, location);
            }
        }
        OutputFormat::MarkdownTable => {
            println!("| Filename | Created At | Size | Location |");
            println!("| --- | --- | --- | --- |");
            for backup in &backups {
                let filename = backup_filename(backup);
                let created_at = backup_created_at(backup).unwrap_or("unknown");
                let size = backup_size(backup);
                let location = backup_location(backup, global_location.as_deref());
                println!(
                    "| {} | {} | {} | {} |",
                    filename, created_at, size, location
                );
            }
        }
        OutputFormat::Json => {
            let raw = serde_json::to_string_pretty(&response)?;
            println!("{}", raw);
        }
        OutputFormat::Yaml => {
            let raw = serde_yaml::to_string(&response)?;
            println!("{}", raw);
        }
        OutputFormat::Csv => {
            let mut writer = csv::Writer::from_writer(io::stdout());
            writer.write_record(["filename", "created_at", "size", "location"])?;
            for backup in &backups {
                let filename = backup_filename(backup);
                let created_at = backup_created_at(backup).unwrap_or("");
                // Raw byte count for machine consumption.
                let size = backup
                    .get("size")
                    .and_then(|v| v.as_u64())
                    .or_else(|| backup.get("size_bytes").and_then(|v| v.as_u64()))
                    .map(|v| v.to_string())
                    .or_else(|| {
                        backup
                            .get("size")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_default();
                let location = backup_location(backup, global_location.as_deref());
                writer.write_record([filename, created_at, &size, &location])?;
            }
            writer.flush()?;
        }
        OutputFormat::Urls => {
            return Err(anyhow!(
                "'backup list' does not support '--format urls'; use text/markdown/json/yaml/csv"
            ));
        }
    }
    Ok(())
}

pub fn backup_restore(
    config: &Config,
    discourse_name: &str,
    backup_path: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    if dry_run {
        println!(
            "[dry-run] {}: would restore backup {}",
            discourse.name, backup_path
        );
        return Ok(());
    }
    let client = DiscourseClient::new(discourse)?;
    client.restore_backup(backup_path)?;
    Ok(())
}

pub fn backup_pull(
    config: &Config,
    discourse_name: &str,
    backup_filename: &str,
    local_path: Option<&Path>,
    force: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let backup_filename = validate_backup_filename(backup_filename)?;
    let url = format!("{}/admin/backups/{}", client.baseurl(), backup_filename);
    // Backup downloads can legitimately take minutes for large archives;
    // bypass the standard per-request timeout by using the raw client.
    let mut response = client
        .raw_client()
        .get(&url)
        .send()
        .context("downloading backup")?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!(
            "failed to download backup {} (HTTP {})",
            backup_filename,
            status
        ));
    }

    let dest = match local_path {
        Some(p) => p.to_path_buf(),
        None => Path::new(backup_filename).to_path_buf(),
    };
    let mut output = create_atomic_output(&dest, force, true)?;
    let bytes = response
        .copy_to(output.file_mut())
        .with_context(|| format!("streaming backup response from {}", url))?;
    output.commit()?;
    println!(
        "Backup {} pulled to {} ({} bytes)",
        backup_filename,
        dest.display(),
        bytes
    );
    Ok(())
}

fn validate_backup_filename(filename: &str) -> Result<&str> {
    let filename = filename.trim();
    if filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')))
    {
        return Err(anyhow!("invalid backup filename: {}", filename));
    }
    Ok(filename)
}

/// Pull the backup array out of the list response. `GET /admin/backups.json`
/// renders a bare array of backup files (`render_serialized(store.files,
/// BackupFileSerializer)`); an earlier assumption of a `{ "backups": [...] }`
/// wrapper meant the list was always empty against a real forum. Accept both.
fn extract_backups(response: &serde_json::Value) -> Vec<serde_json::Value> {
    response
        .as_array()
        .or_else(|| response.get("backups").and_then(|v| v.as_array()))
        .cloned()
        .unwrap_or_default()
}

fn backup_filename(backup: &serde_json::Value) -> &str {
    backup
        .get("filename")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
}

fn backup_created_at(backup: &serde_json::Value) -> Option<&str> {
    // Discourse's BackupFileSerializer exposes `last_modified`; tolerate a
    // `created_at` shape too.
    backup
        .get("last_modified")
        .and_then(|v| v.as_str())
        .or_else(|| backup.get("created_at").and_then(|v| v.as_str()))
}

/// Human-readable backup size. The serializer gives `size` as an integer byte
/// count; tolerate a pre-formatted string and a `size_bytes` alias.
fn backup_size(backup: &serde_json::Value) -> String {
    if let Some(bytes) = backup
        .get("size")
        .and_then(|v| v.as_u64())
        .or_else(|| backup.get("size_bytes").and_then(|v| v.as_u64()))
    {
        return format_bytes(bytes);
    }
    backup
        .get("size")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Format a byte count as B / KB / MB / GB / TB (base-1024, one decimal place
/// above a kilobyte).
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

fn backup_location_response(response: &serde_json::Value) -> Option<String> {
    let keys = [
        "backup_location",
        "location",
        "storage_location",
        "backup_store",
        "upload_destination",
    ];
    for key in keys {
        if let Some(value) = response.get(key).and_then(|v| v.as_str()) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn backup_location(backup: &serde_json::Value, global: Option<&str>) -> String {
    if let Some(global) = global {
        return global.to_string();
    }
    if let Some(location) = backup
        .get("location")
        .and_then(|v| v.as_str())
        .or_else(|| backup.get("backup_location").and_then(|v| v.as_str()))
        .or_else(|| backup.get("storage_location").and_then(|v| v.as_str()))
        .or_else(|| backup.get("upload_destination").and_then(|v| v.as_str()))
    {
        return location.to_string();
    }
    if let Some(url) = backup
        .get("url")
        .and_then(|v| v.as_str())
        .or_else(|| backup.get("path").and_then(|v| v.as_str()))
    {
        return location_from_url(url);
    }
    "unknown".to_string()
}

fn location_from_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with('/') {
        return "local".to_string();
    }
    if let Some(rest) = trimmed.split("//").nth(1) {
        return rest.split('/').next().unwrap_or(trimmed).to_string();
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // The authoritative shape: `GET /admin/backups.json` returns a bare array
    // of `{ filename, size, last_modified }` (BackupFileSerializer).
    fn discourse_response() -> serde_json::Value {
        json!([
            {
                "filename": "accm-2026-06-26-120005-v20260601000000.tar.gz",
                "size": 2_147_483_648u64,
                "last_modified": "2026-06-26T12:00:05.000Z"
            }
        ])
    }

    #[test]
    fn extracts_bare_array_response() {
        let backups = extract_backups(&discourse_response());
        assert_eq!(backups.len(), 1, "bare array must yield the backup");
        let b = &backups[0];
        assert_eq!(
            backup_filename(b),
            "accm-2026-06-26-120005-v20260601000000.tar.gz"
        );
        assert_eq!(backup_created_at(b), Some("2026-06-26T12:00:05.000Z"));
        assert_eq!(backup_size(b), "2.0 GB");
    }

    #[test]
    fn extracts_wrapped_array_response() {
        // Defensive: tolerate a `{ "backups": [...] }` wrapper too.
        let wrapped = json!({ "backups": discourse_response() });
        assert_eq!(extract_backups(&wrapped).len(), 1);
    }

    #[test]
    fn empty_response_yields_no_backups() {
        assert!(extract_backups(&json!([])).is_empty());
        assert!(extract_backups(&json!({})).is_empty());
    }

    #[test]
    fn created_at_is_used_when_last_modified_absent() {
        let b = json!({ "filename": "x.tar.gz", "created_at": "2026-01-01T00:00:00Z" });
        assert_eq!(backup_created_at(&b), Some("2026-01-01T00:00:00Z"));
    }

    #[test]
    fn size_tolerates_string_and_alias() {
        assert_eq!(backup_size(&json!({ "size_bytes": 1024u64 })), "1.0 KB");
        assert_eq!(backup_size(&json!({ "size": "42 MB" })), "42 MB");
        assert_eq!(backup_size(&json!({})), "unknown");
    }

    #[test]
    fn format_bytes_scales_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_bytes(2_147_483_648), "2.0 GB");
        assert_eq!(format_bytes(3 * 1024u64.pow(4)), "3.0 TB");
    }
}

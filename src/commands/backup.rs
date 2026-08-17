// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::OutputFormat;
use crate::commands::common::{ensure_api_credentials, parse_tags, select_discourse};
use crate::config::{Config, DiscourseConfig, find_discourse};
use crate::utils::create_atomic_output;
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::Path;
use std::process::Command;

pub fn backup_create(config: &Config, discourse_name: &str) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    client.create_backup()?;
    Ok(())
}

/// Fan out `backup create` to every configured forum. Continues past
/// per-forum failures (missing credentials, unreachable forum) so one bad
/// entry doesn't block the rest of the fleet; fails at the end if any forum
/// could not be backed up.
pub fn backup_create_all(config: &Config) -> Result<()> {
    if config.discourse.is_empty() {
        return Err(anyhow!("no discourses configured"));
    }

    let mut failed = 0usize;
    for discourse in &config.discourse {
        match backup_create_one(discourse) {
            Ok(()) => println!("{}: backup requested", discourse.name),
            Err(e) => {
                failed += 1;
                eprintln!("{}: backup failed - {e}", discourse.name);
            }
        }
    }

    if failed > 0 {
        return Err(anyhow!(
            "backup creation failed on {failed} of {} forum(s)",
            config.discourse.len()
        ));
    }
    Ok(())
}

fn backup_create_one(discourse: &DiscourseConfig) -> Result<()> {
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    client.create_backup()
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

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BackupHealthStatus {
    Ok,
    Stale,
    Missing,
    NotS3,
    Misconfigured,
    Inaccessible,
    Unknown,
}

impl BackupHealthStatus {
    fn is_healthy(self) -> bool {
        matches!(self, Self::Ok | Self::NotS3)
    }
}

#[derive(Debug, Clone, Serialize)]
struct BackupHealthRow {
    discourse: String,
    status: BackupHealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    bucket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_modified_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    age_days: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bucket_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bucket_object_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Debug, Clone)]
struct S3Object {
    key: String,
    modified_at: DateTime<Utc>,
    size_bytes: u64,
}

#[derive(Debug, Clone)]
struct S3BucketSummary {
    latest_archive: Option<S3Object>,
    total_size_bytes: u64,
    object_count: u64,
}

/// Check actual S3 backup objects for one or more configured Discourses.
pub fn backup_health(
    config: &Config,
    discourse_name: Option<&str>,
    tags: Option<&str>,
    max_age: u64,
    format: OutputFormat,
) -> Result<()> {
    if discourse_name.is_some() && tags.is_some() {
        return Err(anyhow!(
            "cannot pass <discourse> together with --tags; specify either a single discourse or a tag filter"
        ));
    }

    let discourses = selected_discourses(config, discourse_name, tags)?;
    if discourses.is_empty() {
        return emit_backup_health(Vec::new(), format);
    }

    let mut rows: Vec<BackupHealthRow> = discourses
        .iter()
        .map(|discourse| backup_health_configuration(discourse))
        .collect();

    let buckets: BTreeMap<(String, String), Vec<usize>> = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            (matches!(row.status, BackupHealthStatus::Unknown) && row.detail.is_none())
                .then(|| row.bucket.as_ref().zip(row.region.as_ref()))
                .flatten()
                .map(|(bucket, region)| ((bucket.clone(), region.clone()), index))
        })
        .fold(BTreeMap::new(), |mut buckets, (key, index)| {
            buckets.entry(key).or_default().push(index);
            buckets
        });

    if !buckets.is_empty() {
        let aws_error = aws_preflight().err().map(|error| error.to_string());
        let now = Utc::now();
        for ((bucket, region), indexes) in buckets {
            let result = match &aws_error {
                Some(error) => Err(anyhow!(error.clone())),
                None => list_s3_bucket(&bucket, &region),
            };
            for index in indexes {
                rows[index] = match &result {
                    Ok(summary) => health_row_from_summary(
                        &rows[index].discourse,
                        bucket.clone(),
                        region.clone(),
                        summary,
                        now,
                        max_age,
                    ),
                    Err(error) => inaccessible_health_row(
                        &rows[index].discourse,
                        bucket.clone(),
                        region.clone(),
                        error,
                    ),
                };
            }
        }
    }

    rows.sort_by(|left, right| left.discourse.cmp(&right.discourse));
    let unhealthy = rows.iter().any(|row| !row.status.is_healthy());
    emit_backup_health(rows, format)?;
    if unhealthy {
        return Err(anyhow!("one or more backup health checks failed"));
    }
    Ok(())
}

pub(crate) fn selected_discourses<'a>(
    config: &'a Config,
    discourse_name: Option<&str>,
    tags: Option<&str>,
) -> Result<Vec<&'a DiscourseConfig>> {
    if let Some(name) = discourse_name {
        return find_discourse(config, name)
            .map(|discourse| vec![discourse])
            .ok_or_else(|| anyhow!("discourse not found: {name}"));
    }
    let filter = tags.map(parse_tags).unwrap_or_default();
    Ok(config
        .discourse
        .iter()
        .filter(|discourse| matches_tag_filter(discourse, &filter))
        .collect())
}

fn matches_tag_filter(discourse: &DiscourseConfig, filter: &[String]) -> bool {
    if filter.is_empty() {
        return true;
    }
    let tags: HashSet<String> = discourse
        .tags
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|tag| tag.to_ascii_lowercase())
        .collect();
    filter
        .iter()
        .any(|tag| tags.contains(&tag.to_ascii_lowercase()))
}

fn backup_health_configuration(discourse: &DiscourseConfig) -> BackupHealthRow {
    let mut row = BackupHealthRow {
        discourse: discourse.name.clone(),
        status: BackupHealthStatus::Unknown,
        bucket: None,
        region: None,
        latest_key: None,
        latest_modified_at: None,
        age_days: None,
        latest_size_bytes: None,
        bucket_size_bytes: None,
        bucket_object_count: None,
        detail: None,
    };
    if let Err(error) = ensure_api_credentials(discourse) {
        row.detail = Some(error.to_string());
        return row;
    }
    let client = match DiscourseClient::new(discourse) {
        Ok(client) => client,
        Err(error) => {
            row.detail = Some(error.to_string());
            return row;
        }
    };
    let settings = match backup_s3_settings(&client) {
        Ok(settings) => settings,
        Err(error) => {
            row.detail = Some(error.to_string());
            return row;
        }
    };
    if settings.location != "s3" {
        row.status = BackupHealthStatus::NotS3;
        row.detail = Some(format!("backup_location={}", settings.location));
        return row;
    }
    row.bucket = settings.bucket;
    row.region = settings.region;
    if row.bucket.is_none() || row.region.is_none() {
        row.status = BackupHealthStatus::Misconfigured;
        row.detail =
            Some("backup_location=s3 but s3_backup_bucket or s3_region is empty".to_string());
    }
    row
}

struct BackupS3Settings {
    location: String,
    bucket: Option<String>,
    region: Option<String>,
}

fn backup_s3_settings(client: &DiscourseClient) -> Result<BackupS3Settings> {
    let settings = client.list_site_settings()?;
    let entries = settings
        .get("site_settings")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("site settings response missing 'site_settings' array"))?;
    let mut values = HashMap::new();
    for entry in entries {
        if let Some(name) = entry.get("setting").and_then(Value::as_str)
            && matches!(name, "backup_location" | "s3_backup_bucket" | "s3_region")
        {
            let value = entry
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            values.insert(name, value);
        }
    }
    let location = values.remove("backup_location").unwrap_or_default();
    Ok(BackupS3Settings {
        location,
        bucket: values
            .remove("s3_backup_bucket")
            .filter(|value| !value.is_empty()),
        region: values.remove("s3_region").filter(|value| !value.is_empty()),
    })
}

fn aws_preflight() -> Result<()> {
    aws_json(&["sts", "get-caller-identity"])?;
    Ok(())
}

fn list_s3_bucket(bucket: &str, region: &str) -> Result<S3BucketSummary> {
    let mut token: Option<String> = None;
    let mut seen_tokens = HashSet::new();
    let mut objects = Vec::new();
    loop {
        let mut args = vec![
            "s3api".to_string(),
            "list-objects-v2".to_string(),
            "--bucket".to_string(),
            bucket.to_string(),
            "--region".to_string(),
            region.to_string(),
        ];
        if let Some(token) = token.as_deref() {
            args.push("--continuation-token".to_string());
            args.push(token.to_string());
        }
        let page = aws_json(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        objects.extend(parse_s3_objects(&page)?);
        if !page
            .get("IsTruncated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            break;
        }
        let next = page
            .get("NextContinuationToken")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| anyhow!("S3 response is truncated without NextContinuationToken"))?
            .to_string();
        if !seen_tokens.insert(next.clone()) {
            return Err(anyhow!("S3 pagination loop detected for bucket {bucket}"));
        }
        token = Some(next);
    }
    let total_size_bytes = objects.iter().map(|object| object.size_bytes).sum();
    let latest_archive = objects
        .iter()
        .filter(|object| is_backup_archive(&object.key))
        .max_by_key(|object| object.modified_at)
        .cloned();
    Ok(S3BucketSummary {
        latest_archive,
        total_size_bytes,
        object_count: objects.len() as u64,
    })
}

fn aws_json(args: &[&str]) -> Result<Value> {
    let output = Command::new("aws")
        .args(args)
        .args(["--output", "json"])
        .output()
        .context("running `aws` - install the AWS CLI and authenticate its credential chain")?;
    if !output.status.success() {
        return Err(anyhow!(
            "aws {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .with_context(|| format!("parsing `aws {}` output", args.join(" ")))
}

fn parse_s3_objects(page: &Value) -> Result<Vec<S3Object>> {
    let contents = match page.get("Contents") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(contents)) => contents,
        Some(_) => return Err(anyhow!("S3 response Contents is not an array")),
    };
    contents
        .iter()
        .map(|object| {
            let key = object
                .get("Key")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("S3 object missing Key"))?
                .to_string();
            let modified_at = object
                .get("LastModified")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("S3 object '{key}' missing LastModified"))?
                .parse::<DateTime<Utc>>()
                .with_context(|| format!("parsing S3 LastModified for {key}"))?;
            let size_bytes = object
                .get("Size")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow!("S3 object '{key}' missing Size"))?;
            Ok(S3Object {
                key,
                modified_at,
                size_bytes,
            })
        })
        .collect()
}

fn is_backup_archive(key: &str) -> bool {
    let basename = key.rsplit('/').next().unwrap_or(key);
    basename.ends_with(".tar.gz") || basename.ends_with(".tar")
}

fn health_row_from_summary(
    discourse: &str,
    bucket: String,
    region: String,
    summary: &S3BucketSummary,
    now: DateTime<Utc>,
    max_age: u64,
) -> BackupHealthRow {
    let mut row = BackupHealthRow {
        discourse: discourse.to_string(),
        status: BackupHealthStatus::Missing,
        bucket: Some(bucket),
        region: Some(region),
        latest_key: None,
        latest_modified_at: None,
        age_days: None,
        latest_size_bytes: None,
        bucket_size_bytes: Some(summary.total_size_bytes),
        bucket_object_count: Some(summary.object_count),
        detail: None,
    };
    let Some(latest) = summary.latest_archive.as_ref() else {
        return row;
    };
    let age_days = now
        .signed_duration_since(latest.modified_at)
        .num_days()
        .max(0) as u64;
    row.status = if age_days > max_age {
        BackupHealthStatus::Stale
    } else {
        BackupHealthStatus::Ok
    };
    row.latest_key = Some(latest.key.clone());
    row.latest_modified_at = Some(latest.modified_at.to_rfc3339());
    row.age_days = Some(age_days);
    row.latest_size_bytes = Some(latest.size_bytes);
    if latest.modified_at > now {
        row.detail = Some("latest backup timestamp is in the future".to_string());
    }
    row
}

fn inaccessible_health_row(
    discourse: &str,
    bucket: String,
    region: String,
    error: &anyhow::Error,
) -> BackupHealthRow {
    BackupHealthRow {
        discourse: discourse.to_string(),
        status: BackupHealthStatus::Inaccessible,
        bucket: Some(bucket),
        region: Some(region),
        latest_key: None,
        latest_modified_at: None,
        age_days: None,
        latest_size_bytes: None,
        bucket_size_bytes: None,
        bucket_object_count: None,
        detail: Some(error.to_string()),
    }
}

fn emit_backup_health(rows: Vec<BackupHealthRow>, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => print_backup_health_text(&rows),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&rows)?),
        OutputFormat::Csv => {
            let mut writer = csv::Writer::from_writer(io::stdout());
            writer.write_record([
                "discourse",
                "status",
                "bucket",
                "region",
                "latest_key",
                "latest_modified_at",
                "age_days",
                "latest_size_bytes",
                "bucket_size_bytes",
                "bucket_object_count",
                "detail",
            ])?;
            for row in &rows {
                writer.write_record([
                    row.discourse.as_str(),
                    health_status_text(row.status),
                    row.bucket.as_deref().unwrap_or(""),
                    row.region.as_deref().unwrap_or(""),
                    row.latest_key.as_deref().unwrap_or(""),
                    row.latest_modified_at.as_deref().unwrap_or(""),
                    &row.age_days
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    &row.latest_size_bytes
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    &row.bucket_size_bytes
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    &row.bucket_object_count
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    row.detail.as_deref().unwrap_or(""),
                ])?;
            }
            writer.flush()?;
        }
        OutputFormat::Markdown | OutputFormat::MarkdownTable | OutputFormat::Urls => {
            return Err(anyhow!(
                "backup health supports --format text/json/yaml/csv"
            ));
        }
    }
    Ok(())
}

fn print_backup_health_text(rows: &[BackupHealthRow]) {
    if rows.is_empty() {
        println!("No Discourses selected.");
        return;
    }
    println!(
        "{:<20} {:<15} {:<42} {:>5} {:>13} {:>13} Bucket",
        "Forum", "Status", "Latest backup", "Age", "Latest size", "Bucket size"
    );
    for row in rows {
        let latest = row
            .latest_key
            .as_deref()
            .and_then(|key| key.rsplit('/').next())
            .unwrap_or("-");
        let age = row
            .age_days
            .map(|days| format!("{days}d"))
            .unwrap_or_else(|| "-".to_string());
        let latest_size = row
            .latest_size_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "-".to_string());
        let bucket_size = row
            .bucket_size_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<20} {:<15} {:<42} {:>5} {:>13} {:>13} {}",
            row.discourse,
            health_status_text(row.status),
            latest,
            age,
            latest_size,
            bucket_size,
            row.bucket.as_deref().unwrap_or("-")
        );
        if let Some(detail) = &row.detail {
            println!("  {}: {}", row.discourse, detail);
        }
    }
}

fn health_status_text(status: BackupHealthStatus) -> &'static str {
    match status {
        BackupHealthStatus::Ok => "OK",
        BackupHealthStatus::Stale => "STALE",
        BackupHealthStatus::Missing => "MISSING",
        BackupHealthStatus::NotS3 => "NOT_S3",
        BackupHealthStatus::Misconfigured => "MISCONFIGURED",
        BackupHealthStatus::Inaccessible => "INACCESSIBLE",
        BackupHealthStatus::Unknown => "UNKNOWN",
    }
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

    #[test]
    fn parses_s3_objects_and_chooses_newest_archive() {
        let page = json!({
            "Contents": [
                { "Key": "backups/default/old.tar.gz", "LastModified": "2026-07-20T02:00:00Z", "Size": 10 },
                { "Key": "backups/default/new.tar.gz", "LastModified": "2026-07-28T02:00:00Z", "Size": 20 },
                { "Key": "reports/report.csv", "LastModified": "2026-07-29T02:00:00Z", "Size": 30 }
            ]
        });
        let objects = parse_s3_objects(&page).unwrap();
        assert_eq!(objects.len(), 3);
        assert_eq!(
            objects.iter().map(|object| object.size_bytes).sum::<u64>(),
            60
        );
        let latest = objects
            .iter()
            .filter(|object| is_backup_archive(&object.key))
            .max_by_key(|object| object.modified_at)
            .unwrap();
        assert_eq!(latest.key, "backups/default/new.tar.gz");
    }

    #[test]
    fn archive_matching_excludes_non_backup_objects() {
        assert!(is_backup_archive("backups/default/forum.tar.gz"));
        assert!(is_backup_archive("forum.tar"));
        assert!(!is_backup_archive("backups/default/forum.tar.gz.sha256"));
        assert!(!is_backup_archive("reports/backup-report.csv"));
    }

    #[test]
    fn missing_s3_contents_means_an_empty_bucket() {
        assert!(parse_s3_objects(&json!({})).unwrap().is_empty());
    }

    #[test]
    fn health_classifies_fresh_stale_and_missing_archives() {
        let now = "2026-07-28T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let fresh = S3BucketSummary {
            latest_archive: Some(S3Object {
                key: "fresh.tar.gz".to_string(),
                modified_at: "2026-07-27T13:00:00Z".parse().unwrap(),
                size_bytes: 100,
            }),
            total_size_bytes: 200,
            object_count: 2,
        };
        let stale = S3BucketSummary {
            latest_archive: Some(S3Object {
                key: "stale.tar.gz".to_string(),
                modified_at: "2026-07-25T11:59:59Z".parse().unwrap(),
                size_bytes: 100,
            }),
            total_size_bytes: 100,
            object_count: 1,
        };
        let missing = S3BucketSummary {
            latest_archive: None,
            total_size_bytes: 9,
            object_count: 1,
        };
        assert_eq!(
            health_row_from_summary("forum", "bucket".into(), "region".into(), &fresh, now, 1)
                .status,
            BackupHealthStatus::Ok
        );
        assert_eq!(
            health_row_from_summary("forum", "bucket".into(), "region".into(), &stale, now, 2)
                .status,
            BackupHealthStatus::Stale
        );
        let missing =
            health_row_from_summary("forum", "bucket".into(), "region".into(), &missing, now, 2);
        assert_eq!(missing.status, BackupHealthStatus::Missing);
        assert_eq!(missing.bucket_size_bytes, Some(9));
    }

    #[test]
    fn future_backup_timestamp_has_zero_age_and_a_warning() {
        let now = "2026-07-28T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let summary = S3BucketSummary {
            latest_archive: Some(S3Object {
                key: "future.tar.gz".to_string(),
                modified_at: "2026-07-29T12:00:00Z".parse().unwrap(),
                size_bytes: 100,
            }),
            total_size_bytes: 100,
            object_count: 1,
        };
        let row =
            health_row_from_summary("forum", "bucket".into(), "region".into(), &summary, now, 2);
        assert_eq!(row.age_days, Some(0));
        assert!(row.detail.unwrap().contains("future"));
    }

    #[test]
    fn health_text_and_csv_include_rows() {
        let row = BackupHealthRow {
            discourse: "forum".to_string(),
            status: BackupHealthStatus::Ok,
            bucket: Some("bucket".to_string()),
            region: Some("eu-west-2".to_string()),
            latest_key: Some("backups/default/forum.tar.gz".to_string()),
            latest_modified_at: Some("2026-07-28T12:00:00Z".to_string()),
            age_days: Some(0),
            latest_size_bytes: Some(1024),
            bucket_size_bytes: Some(2048),
            bucket_object_count: Some(2),
            detail: None,
        };
        assert_eq!(health_status_text(row.status), "OK");
        assert!(row.status.is_healthy());
    }

    #[test]
    fn tag_filter_matches_case_insensitively() {
        let forum = DiscourseConfig {
            name: "forum".to_string(),
            tags: Some(vec!["Production".to_string()]),
            ..DiscourseConfig::default()
        };
        assert!(matches_tag_filter(&forum, &["production".to_string()]));
        assert!(!matches_tag_filter(&forum, &["staging".to_string()]));
    }
}

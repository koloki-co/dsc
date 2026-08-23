// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::OutputFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse, selected_discourses};
use crate::config::{Config, DiscourseConfig};
use crate::utils::create_atomic_output;
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

pub fn backup_create(config: &Config, discourse_name: &str) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    client.create_backup()?;
    Ok(())
}

/// Fan out `backup create` to every configured forum, optionally filtered
/// by `--tags`. Continues past per-forum failures (missing credentials,
/// unreachable forum) so one bad entry doesn't block the rest of the fleet;
/// fails at the end if any forum could not be backed up.
pub fn backup_create_all(config: &Config, tags: Option<&str>) -> Result<()> {
    let discourses = selected_discourses(config, None, tags)?;
    if discourses.is_empty() {
        return Err(if tags.is_some() {
            anyhow!("no discourses configured matching the given tags")
        } else {
            anyhow!("no discourses configured")
        });
    }

    let mut failed = 0usize;
    for discourse in &discourses {
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
            discourses.len()
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
    Disabled,
    NotS3,
    Misconfigured,
    Inaccessible,
    Unknown,
}

impl BackupHealthStatus {
    fn is_healthy(self) -> bool {
        matches!(self, Self::Ok | Self::Disabled | Self::NotS3)
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
    endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup_frequency_days: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stale_after_days: Option<u64>,
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
    #[serde(skip)]
    access_key_id: Option<String>,
    #[serde(skip)]
    secret_access_key: Option<String>,
    #[serde(skip)]
    use_iam_profile: bool,
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
    max_age: Option<u64>,
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

    if !matches!(
        format,
        OutputFormat::Text | OutputFormat::Json | OutputFormat::Yaml | OutputFormat::Csv
    ) {
        return Err(anyhow!(
            "backup health supports --format text/json/yaml/csv"
        ));
    }

    let mut rows: Vec<BackupHealthRow> = discourses
        .iter()
        .map(|discourse| backup_health_configuration(discourse, max_age))
        .collect();
    let stream = matches!(format, OutputFormat::Text | OutputFormat::Csv);
    let forum_width = rows
        .iter()
        .map(|row| row.discourse.chars().count())
        .max()
        .unwrap_or(5)
        .max(5);
    let mut csv_writer =
        matches!(format, OutputFormat::Csv).then(|| csv::Writer::from_writer(io::stdout()));
    if matches!(format, OutputFormat::Text) {
        print_backup_health_text_header(forum_width);
    } else if let Some(writer) = csv_writer.as_mut() {
        write_backup_health_csv_header(writer)?;
        writer.flush()?;
    }

    let now = Utc::now();
    for row in &mut rows {
        if matches!(row.status, BackupHealthStatus::Unknown) && row.detail.is_none() {
            let bucket = row.bucket.as_deref().expect("validated bucket");
            let region = row.region.as_deref().expect("validated region");
            match list_s3_bucket(
                bucket,
                region,
                row.endpoint.as_deref(),
                row.access_key_id.as_deref(),
                row.secret_access_key.as_deref(),
            ) {
                Ok(summary) => apply_health_summary(row, &summary, now),
                Err(error) => apply_inaccessible_health(row, &error),
            }
        }
        if stream {
            match format {
                OutputFormat::Text => {
                    print_backup_health_text_row(row, forum_width);
                    io::stdout().flush()?;
                }
                OutputFormat::Csv => {
                    write_backup_health_csv_row(csv_writer.as_mut().expect("CSV writer"), row)?;
                    csv_writer.as_mut().expect("CSV writer").flush()?;
                }
                _ => unreachable!(),
            }
        }
    }

    let unhealthy = rows.iter().any(|row| !row.status.is_healthy());
    if !stream {
        rows.sort_by(|left, right| left.discourse.cmp(&right.discourse));
        emit_backup_health(rows, format)?;
    }
    if unhealthy {
        return Err(anyhow!("one or more backup health checks failed"));
    }
    Ok(())
}

fn backup_health_configuration(
    discourse: &DiscourseConfig,
    max_age: Option<u64>,
) -> BackupHealthRow {
    let mut row = BackupHealthRow {
        discourse: discourse.name.clone(),
        status: BackupHealthStatus::Unknown,
        bucket: None,
        region: None,
        endpoint: None,
        backup_frequency_days: None,
        stale_after_days: None,
        latest_key: None,
        latest_modified_at: None,
        age_days: None,
        latest_size_bytes: None,
        bucket_size_bytes: None,
        bucket_object_count: None,
        detail: None,
        access_key_id: None,
        secret_access_key: None,
        use_iam_profile: false,
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
    row.endpoint = settings.endpoint;
    row.backup_frequency_days = settings.backup_frequency_days;
    row.stale_after_days = settings
        .backup_frequency_days
        .and_then(|frequency| effective_stale_after_days(frequency, max_age));
    row.access_key_id = settings.access_key_id;
    row.secret_access_key = settings.secret_access_key;
    row.use_iam_profile = settings.use_iam_profile;
    if row.bucket.is_none() || row.region.is_none() {
        row.status = BackupHealthStatus::Misconfigured;
        row.detail =
            Some("backup_location=s3 but s3_backup_bucket or s3_region is empty".to_string());
    } else if row.backup_frequency_days.is_none() {
        row.status = BackupHealthStatus::Misconfigured;
        row.detail =
            Some("backup_frequency is missing or is not a whole number of days".to_string());
    } else if !row.use_iam_profile
        && (row.access_key_id.is_none() || row.secret_access_key.is_none())
    {
        row.status = BackupHealthStatus::Misconfigured;
        row.detail = Some(
            "S3 static credentials are incomplete and s3_use_iam_profile is false".to_string(),
        );
    }
    row
}

struct BackupS3Settings {
    location: String,
    bucket: Option<String>,
    region: Option<String>,
    endpoint: Option<String>,
    backup_frequency_days: Option<u64>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    use_iam_profile: bool,
}

fn backup_s3_settings(client: &DiscourseClient) -> Result<BackupS3Settings> {
    let settings = client.list_site_settings()?;
    parse_backup_s3_settings(&settings)
}

fn parse_backup_s3_settings(settings: &Value) -> Result<BackupS3Settings> {
    let entries = settings
        .get("site_settings")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("site settings response missing 'site_settings' array"))?;
    let mut values = HashMap::new();
    for entry in entries {
        if let Some(name) = entry.get("setting").and_then(Value::as_str)
            && matches!(
                name,
                "backup_location"
                    | "backup_frequency"
                    | "s3_backup_bucket"
                    | "s3_region"
                    | "s3_endpoint"
                    | "s3_access_key_id"
                    | "s3_secret_access_key"
                    | "s3_use_iam_profile"
            )
        {
            let value = setting_value_text(entry.get("value"));
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
        endpoint: values
            .remove("s3_endpoint")
            .filter(|value| !value.is_empty())
            .map(|value| validate_s3_endpoint(&value))
            .transpose()?,
        backup_frequency_days: values
            .remove("backup_frequency")
            .and_then(|value| value.parse().ok()),
        access_key_id: values
            .remove("s3_access_key_id")
            .filter(|value| !value.is_empty()),
        secret_access_key: values
            .remove("s3_secret_access_key")
            .filter(|value| !value.is_empty()),
        use_iam_profile: values
            .remove("s3_use_iam_profile")
            .is_some_and(|value| value.eq_ignore_ascii_case("true")),
    })
}

fn setting_value_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.trim().to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn validate_s3_endpoint(endpoint: &str) -> Result<String> {
    let parsed = reqwest::Url::parse(endpoint)
        .with_context(|| format!("invalid s3_endpoint URL: {endpoint:?}"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(anyhow!(
            "s3_endpoint must be an absolute HTTP(S) URL without credentials, query, or fragment"
        ));
    }
    Ok(endpoint.trim_end_matches('/').to_string())
}

fn effective_stale_after_days(frequency: u64, max_age: Option<u64>) -> Option<u64> {
    (frequency > 0).then(|| max_age.unwrap_or(frequency).max(frequency))
}

fn list_s3_bucket(
    bucket: &str,
    region: &str,
    endpoint: Option<&str>,
    access_key_id: Option<&str>,
    secret_access_key: Option<&str>,
) -> Result<S3BucketSummary> {
    let mut token: Option<String> = None;
    let mut seen_tokens = HashSet::new();
    let mut objects = Vec::new();
    loop {
        let args = list_s3_args(bucket, region, endpoint, token.as_deref());
        let page = aws_json(
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            access_key_id,
            secret_access_key,
        )?;
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

fn list_s3_args(
    bucket: &str,
    region: &str,
    endpoint: Option<&str>,
    token: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "s3api".to_string(),
        "list-objects-v2".to_string(),
        "--bucket".to_string(),
        bucket.to_string(),
        "--region".to_string(),
        region.to_string(),
    ];
    if let Some(endpoint) = endpoint {
        args.push("--endpoint-url".to_string());
        args.push(endpoint.to_string());
    }
    if let Some(token) = token {
        args.push("--continuation-token".to_string());
        args.push(token.to_string());
    }
    args
}

fn aws_json(
    args: &[&str],
    access_key_id: Option<&str>,
    secret_access_key: Option<&str>,
) -> Result<Value> {
    let mut command = Command::new("aws");
    command.args(args).args(["--output", "json"]);
    if let (Some(access_key_id), Some(secret_access_key)) = (access_key_id, secret_access_key) {
        command
            .env("AWS_ACCESS_KEY_ID", access_key_id)
            .env("AWS_SECRET_ACCESS_KEY", secret_access_key)
            .env_remove("AWS_SESSION_TOKEN");
    }
    let output = command
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

fn apply_health_summary(row: &mut BackupHealthRow, summary: &S3BucketSummary, now: DateTime<Utc>) {
    row.status = if row.backup_frequency_days == Some(0) {
        BackupHealthStatus::Disabled
    } else {
        BackupHealthStatus::Missing
    };
    row.bucket_size_bytes = Some(summary.total_size_bytes);
    row.bucket_object_count = Some(summary.object_count);
    let Some(latest) = summary.latest_archive.as_ref() else {
        return;
    };
    let age_days = now
        .signed_duration_since(latest.modified_at)
        .num_days()
        .max(0) as u64;
    row.status = match row.stale_after_days {
        None => BackupHealthStatus::Disabled,
        Some(stale_after_days) if age_days > stale_after_days => BackupHealthStatus::Stale,
        Some(_) => BackupHealthStatus::Ok,
    };
    row.latest_key = Some(latest.key.clone());
    row.latest_modified_at = Some(latest.modified_at.to_rfc3339());
    row.age_days = Some(age_days);
    row.latest_size_bytes = Some(latest.size_bytes);
    if latest.modified_at > now {
        row.detail = Some("latest backup timestamp is in the future".to_string());
    }
}

fn apply_inaccessible_health(row: &mut BackupHealthRow, error: &anyhow::Error) {
    row.status = BackupHealthStatus::Inaccessible;
    row.detail = Some(error.to_string());
}

fn emit_backup_health(rows: Vec<BackupHealthRow>, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => print_backup_health_text(&rows),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&rows)?),
        OutputFormat::Csv => {
            let mut writer = csv::Writer::from_writer(io::stdout());
            write_backup_health_csv_header(&mut writer)?;
            for row in &rows {
                write_backup_health_csv_row(&mut writer, row)?;
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
    let forum_width = rows
        .iter()
        .map(|row| row.discourse.chars().count())
        .max()
        .unwrap_or(5)
        .max(5);
    print_backup_health_text_header(forum_width);
    for row in rows {
        print_backup_health_text_row(row, forum_width);
    }
}

const LATEST_BACKUP_WIDTH: usize = 48;

fn print_backup_health_text_header(forum_width: usize) {
    println!(
        "{:<forum_width$} {:<13} {:<LATEST_BACKUP_WIDTH$} {:>5} {:>7} {:>13} {:>13} Bucket",
        "Forum", "Status", "Latest backup", "Age", "Every", "Latest size", "Bucket size"
    );
}

fn print_backup_health_text_row(row: &BackupHealthRow, forum_width: usize) {
    let latest = row
        .latest_key
        .as_deref()
        .and_then(|key| key.rsplit('/').next())
        .map(|value| truncate_column(value, LATEST_BACKUP_WIDTH))
        .unwrap_or_else(|| "-".to_string());
    let age = row
        .age_days
        .map(|days| format!("{days}d"))
        .unwrap_or_else(|| "-".to_string());
    let frequency = row
        .backup_frequency_days
        .map(|days| {
            if days == 0 {
                "off".to_string()
            } else {
                format!("{days}d")
            }
        })
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
        "{:<forum_width$} {:<13} {:<LATEST_BACKUP_WIDTH$} {:>5} {:>7} {:>13} {:>13} {}",
        row.discourse,
        health_status_text(row.status),
        latest,
        age,
        frequency,
        latest_size,
        bucket_size,
        row.bucket.as_deref().unwrap_or("-")
    );
    if let Some(detail) = &row.detail {
        println!("  {}: {}", row.discourse, detail);
    }
}

fn truncate_column(value: &str, width: usize) -> String {
    let count = value.chars().count();
    if count <= width {
        return value.to_string();
    }
    let left = (width - 3) / 2;
    let right = width - 3 - left;
    let start: String = value.chars().take(left).collect();
    let end: String = value.chars().skip(count - right).collect();
    format!("{start}...{end}")
}

fn write_backup_health_csv_header<W: io::Write>(writer: &mut csv::Writer<W>) -> Result<()> {
    writer.write_record([
        "discourse",
        "status",
        "bucket",
        "region",
        "endpoint",
        "backup_frequency_days",
        "stale_after_days",
        "latest_key",
        "latest_modified_at",
        "age_days",
        "latest_size_bytes",
        "bucket_size_bytes",
        "bucket_object_count",
        "detail",
    ])?;
    Ok(())
}

fn write_backup_health_csv_row<W: io::Write>(
    writer: &mut csv::Writer<W>,
    row: &BackupHealthRow,
) -> Result<()> {
    let backup_frequency_days = optional_number(row.backup_frequency_days);
    let stale_after_days = optional_number(row.stale_after_days);
    let age_days = optional_number(row.age_days);
    let latest_size_bytes = optional_number(row.latest_size_bytes);
    let bucket_size_bytes = optional_number(row.bucket_size_bytes);
    let bucket_object_count = optional_number(row.bucket_object_count);
    writer.write_record([
        row.discourse.as_str(),
        health_status_text(row.status),
        row.bucket.as_deref().unwrap_or(""),
        row.region.as_deref().unwrap_or(""),
        row.endpoint.as_deref().unwrap_or(""),
        &backup_frequency_days,
        &stale_after_days,
        row.latest_key.as_deref().unwrap_or(""),
        row.latest_modified_at.as_deref().unwrap_or(""),
        &age_days,
        &latest_size_bytes,
        &bucket_size_bytes,
        &bucket_object_count,
        row.detail.as_deref().unwrap_or(""),
    ])?;
    Ok(())
}

fn optional_number(value: Option<u64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn health_status_text(status: BackupHealthStatus) -> &'static str {
    match status {
        BackupHealthStatus::Ok => "OK",
        BackupHealthStatus::Stale => "STALE",
        BackupHealthStatus::Missing => "MISSING",
        BackupHealthStatus::Disabled => "DISABLED",
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
    fn parses_s3_compatible_settings_without_exposing_credentials() {
        let settings = json!({
            "site_settings": [
                {"setting": "backup_location", "value": "s3"},
                {"setting": "backup_frequency", "value": 1},
                {"setting": "s3_backup_bucket", "value": "forum-backups"},
                {"setting": "s3_region", "value": "us-east-1"},
                {"setting": "s3_endpoint", "value": "https://sfo3.digitaloceanspaces.com/"},
                {"setting": "s3_access_key_id", "value": "access-key"},
                {"setting": "s3_secret_access_key", "value": "secret-key"},
                {"setting": "s3_use_iam_profile", "value": false}
            ]
        });
        let parsed = parse_backup_s3_settings(&settings).unwrap();
        assert_eq!(parsed.backup_frequency_days, Some(1));
        assert_eq!(
            parsed.endpoint.as_deref(),
            Some("https://sfo3.digitaloceanspaces.com")
        );
        assert_eq!(parsed.access_key_id.as_deref(), Some("access-key"));
        assert_eq!(parsed.secret_access_key.as_deref(), Some("secret-key"));
        assert!(!parsed.use_iam_profile);
    }

    #[test]
    fn s3_compatible_listing_includes_the_endpoint_and_pagination_token() {
        let args = list_s3_args(
            "forum-backups",
            "us-east-1",
            Some("https://sfo3.digitaloceanspaces.com"),
            Some("next"),
        );
        assert!(
            args.windows(2)
                .any(|args| { args == ["--endpoint-url", "https://sfo3.digitaloceanspaces.com",] })
        );
        assert!(
            args.windows(2)
                .any(|args| args == ["--continuation-token", "next"])
        );
    }

    #[test]
    fn backup_frequency_is_the_minimum_stale_threshold() {
        assert_eq!(effective_stale_after_days(7, None), Some(7));
        assert_eq!(effective_stale_after_days(7, Some(2)), Some(7));
        assert_eq!(effective_stale_after_days(7, Some(14)), Some(14));
        assert_eq!(effective_stale_after_days(0, Some(14)), None);
    }

    #[test]
    fn rejects_s3_endpoints_with_embedded_credentials() {
        let error = validate_s3_endpoint("https://user:secret@example.com").unwrap_err();
        assert!(error.to_string().contains("without credentials"));
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
        let mut fresh_row = health_test_row(1);
        apply_health_summary(&mut fresh_row, &fresh, now);
        assert_eq!(fresh_row.status, BackupHealthStatus::Ok);
        let mut stale_row = health_test_row(2);
        apply_health_summary(&mut stale_row, &stale, now);
        assert_eq!(stale_row.status, BackupHealthStatus::Stale);
        let mut missing_row = health_test_row(2);
        apply_health_summary(&mut missing_row, &missing, now);
        assert_eq!(missing_row.status, BackupHealthStatus::Missing);
        assert_eq!(missing_row.bucket_size_bytes, Some(9));
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
        let mut row = health_test_row(2);
        apply_health_summary(&mut row, &summary, now);
        assert_eq!(row.age_days, Some(0));
        assert!(row.detail.unwrap().contains("future"));
    }

    #[test]
    fn health_text_and_csv_include_rows() {
        let mut row = health_test_row(1);
        row.status = BackupHealthStatus::Ok;
        row.latest_key = Some("backups/default/forum.tar.gz".to_string());
        row.latest_modified_at = Some("2026-07-28T12:00:00Z".to_string());
        row.age_days = Some(0);
        row.latest_size_bytes = Some(1024);
        row.bucket_size_bytes = Some(2048);
        row.bucket_object_count = Some(2);
        assert_eq!(health_status_text(row.status), "OK");
        assert!(row.status.is_healthy());
    }

    #[test]
    fn structured_health_output_never_serializes_s3_credentials() {
        let mut row = health_test_row(1);
        row.access_key_id = Some("access-key".to_string());
        row.secret_access_key = Some("secret-key".to_string());
        let value = serde_json::to_value(row).unwrap();
        assert!(value.get("access_key_id").is_none());
        assert!(value.get("secret_access_key").is_none());
        assert!(!value.to_string().contains("access-key"));
        assert!(!value.to_string().contains("secret-key"));
    }

    fn health_test_row(stale_after_days: u64) -> BackupHealthRow {
        BackupHealthRow {
            discourse: "forum".to_string(),
            status: BackupHealthStatus::Unknown,
            bucket: Some("bucket".to_string()),
            region: Some("eu-west-2".to_string()),
            endpoint: None,
            backup_frequency_days: Some(stale_after_days),
            stale_after_days: Some(stale_after_days),
            latest_key: None,
            latest_modified_at: None,
            age_days: None,
            latest_size_bytes: None,
            bucket_size_bytes: None,
            bucket_object_count: None,
            detail: None,
            access_key_id: None,
            secret_access_key: None,
            use_iam_profile: true,
        }
    }
}

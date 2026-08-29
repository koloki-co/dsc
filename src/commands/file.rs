// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::cli::{FileCommand, ListFormat};
use crate::commands::common::{
    fleet_worker_count, run_fleet, select_discourse, selected_discourses, shell_quote,
};
use crate::commands::ssh::{
    ReplaceOptions, build_replace_script, effective_host_key_checking, run_ssh_capture,
    run_ssh_pipe,
};
use crate::config::{Config, DiscourseConfig};
use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

pub fn run(config: &Config, command: &FileCommand, dry_run: bool) -> Result<()> {
    match command {
        FileCommand::Audit {
            discourse,
            local_path,
            remote_path,
            tags,
            parallel,
            max,
            format,
        } => file_audit(
            config,
            discourse,
            local_path,
            remote_path,
            tags.as_deref(),
            *parallel,
            *max,
            *format,
        ),
        FileCommand::Push {
            discourse,
            local_path,
            remote_path,
            tags,
            parallel,
            max,
            owner,
            group,
            mode,
            no_backup,
            sudo,
            yes,
            format,
        } => file_push(
            config,
            discourse,
            local_path,
            remote_path,
            tags.as_deref(),
            *parallel,
            *max,
            owner.as_deref(),
            group.as_deref(),
            mode.as_deref(),
            !(*no_backup),
            *sudo,
            *yes,
            dry_run,
            *format,
        ),
    }
}

#[derive(Serialize)]
struct AuditResult {
    forum: String,
    local_path: String,
    remote_path: String,
    local_checksum: String,
    local_size: u64,
    remote_checksum: Option<String>,
    remote_size: Option<u64>,
    status: String,
}

/// Resolve the target set for a file command: one configured forum by name,
/// or a fleet through the shared R48 selector. Returns the selected forums.
fn resolve_targets<'a>(
    config: &'a Config,
    discourse: &str,
    tags: Option<&str>,
) -> Result<Vec<&'a DiscourseConfig>> {
    if discourse == "all" {
        selected_discourses(config, None, tags)
    } else {
        if tags.is_some() {
            return Err(anyhow!(
                "--tags is only valid when the discourse argument is 'all'"
            ));
        }
        select_discourse(config, Some(discourse)).map(|d| vec![d])
    }
}

fn ssh_target(discourse: &DiscourseConfig) -> Result<&str> {
    discourse
        .ssh_host
        .as_deref()
        .filter(|h| !h.trim().is_empty())
        .ok_or_else(|| anyhow!("missing ssh_host for discourse {}", discourse.name))
}

fn audit_text(
    forum: &str,
    remote_path: &str,
    status: &str,
    local_checksum: &str,
    local_size: u64,
    remote_checksum: Option<&str>,
) -> String {
    format!(
        "{forum}: {remote_path} {status} (local {local_checksum} {local_size}, remote {})",
        remote_checksum.unwrap_or("(missing)")
    )
}

/// The per-forum audit outcome used by both the single and fleet paths.
struct AuditOutcome {
    forum: String,
    local_path: String,
    remote_path: String,
    checksum: Option<String>,
    size: Option<u64>,
    status: String,
    failed: bool,
}

fn audit_one_forum(
    local_path: &Path,
    remote_path: &str,
    discourse: &DiscourseConfig,
) -> AuditOutcome {
    let outcome = match ssh_target(discourse) {
        Err(e) => Err(e.to_string()),
        Ok(target) => remote_file_checksum(target, remote_path).map_err(|e| format!("{e:#}")),
    };
    match outcome {
        Ok((checksum, size, status)) => AuditOutcome {
            forum: discourse.name.clone(),
            local_path: local_path.display().to_string(),
            remote_path: remote_path.to_string(),
            checksum,
            size,
            status,
            failed: false,
        },
        Err(message) => AuditOutcome {
            forum: discourse.name.clone(),
            local_path: local_path.display().to_string(),
            remote_path: remote_path.to_string(),
            checksum: None,
            size: None,
            status: format!("failed: {message}"),
            failed: true,
        },
    }
}

fn render_audit_row(row: &AuditOutcome, local_checksum: &str, local_size: u64) -> AuditResult {
    AuditResult {
        forum: row.forum.clone(),
        local_path: row.local_path.clone(),
        remote_path: row.remote_path.clone(),
        local_checksum: local_checksum.to_string(),
        local_size,
        remote_checksum: row.checksum.clone(),
        remote_size: row.size,
        status: row.status.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn file_audit(
    config: &Config,
    discourse: &str,
    local_path: &Path,
    remote_path: &str,
    tags: Option<&str>,
    parallel: bool,
    max: Option<usize>,
    format: ListFormat,
) -> Result<()> {
    let discourses = resolve_targets(config, discourse, tags)?;
    if discourses.is_empty() {
        return Err(if tags.is_some() {
            anyhow!("no configured forums match the given tags")
        } else {
            anyhow!("no configured forums")
        });
    }

    let local_bytes = fs::read(local_path)
        .with_context(|| format!("reading local file {}", local_path.display()))?;
    let local_checksum = hex_sha256(&local_bytes);
    let local_size = local_bytes.len() as u64;

    let workers = if discourses.len() > 1 && parallel {
        fleet_worker_count(max, discourses.len(), 8, false)
    } else {
        1
    };

    let outcomes: Vec<AuditOutcome> = run_fleet(
        &discourses,
        workers,
        |d| audit_one_forum(local_path, remote_path, d),
        |outcome| {
            if outcome.failed {
                eprintln!(
                    "{}: audit failed - {}",
                    outcome.forum,
                    outcome.status.trim_start_matches("failed: ")
                );
            }
        },
    );

    let failed = outcomes.iter().filter(|o| o.failed).count();

    match format {
        ListFormat::Json => {
            let rows: Vec<AuditResult> = outcomes
                .iter()
                .map(|o| render_audit_row(o, &local_checksum, local_size))
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        ListFormat::Yaml => {
            let rows: Vec<AuditResult> = outcomes
                .iter()
                .map(|o| render_audit_row(o, &local_checksum, local_size))
                .collect();
            println!("{}", serde_yaml::to_string(&rows)?);
        }
        ListFormat::Text => {
            for o in &outcomes {
                let status_display = if o.failed {
                    o.status.clone()
                } else {
                    audit_text(
                        &o.forum,
                        &o.remote_path,
                        &o.status,
                        &local_checksum,
                        local_size,
                        o.checksum.as_deref(),
                    )
                };
                println!("{status_display}");
            }
        }
    }

    if failed > 0 {
        return Err(anyhow!("{failed} forum(s) failed audit"));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn file_push(
    config: &Config,
    discourse: &str,
    local_path: &Path,
    remote_path: &str,
    tags: Option<&str>,
    parallel: bool,
    max: Option<usize>,
    owner: Option<&str>,
    group: Option<&str>,
    mode: Option<&str>,
    backup: bool,
    sudo: bool,
    yes: bool,
    dry_run: bool,
    _format: ListFormat,
) -> Result<()> {
    if !remote_path.starts_with('/') {
        return Err(anyhow!("remote path must be absolute: {remote_path}"));
    }
    let discourses = resolve_targets(config, discourse, tags)?;
    if discourses.is_empty() {
        return Err(if tags.is_some() {
            anyhow!("no configured forums match the given tags")
        } else {
            anyhow!("no configured forums")
        });
    }
    let fleet = discourses.len() > 1;

    let local_bytes: std::sync::Arc<Vec<u8>> = std::sync::Arc::new(
        fs::read(local_path)
            .with_context(|| format!("reading local file {}", local_path.display()))?,
    );
    let local_checksum = hex_sha256(&local_bytes);
    let local_size = local_bytes.len() as u64;

    // Enumerate every target and print the complete plan first, including a
    // best-effort per-forum state read. Dry-run never uploads.
    if dry_run {
        println!(
            "[dry-run] file push plan: {} -> {} on {} forum(s)\n  host-key checking: {}",
            local_path.display(),
            remote_path,
            discourses.len(),
            effective_host_key_checking(),
        );
        println!(
            "  owner: {}  group: {}  mode: {}  backup: {}  sudo: {}",
            owner.unwrap_or("(preserve)"),
            group.unwrap_or("(preserve)"),
            mode.unwrap_or("(preserve)"),
            backup_display(backup, discourses.len()),
            if sudo { "yes" } else { "no" },
        );
        for d in &discourses {
            let status = match ssh_target(d) {
                Ok(target) => match remote_file_checksum(target, remote_path) {
                    Ok((checksum, _, status)) => match (&checksum, status.as_str()) {
                        (Some(c), _) if c == &local_checksum => "same".to_string(),
                        _ => status,
                    },
                    Err(_) => "unknown (inspection failed)".to_string(),
                },
                Err(e) => format!("unreachable ({e})"),
            };
            println!("  {}: {status}", d.name);
        }
        println!("Nothing was changed (--dry-run).");
        return Ok(());
    }

    // A fleet push is a mutation across many hosts: require --yes, and keep
    // the operator in control of parallelism (serial by default).
    if fleet && !yes {
        return Err(anyhow!(
            "refusing to push to {} forum(s) without --yes; review with --dry-run first",
            discourses.len()
        ));
    }

    let workers = if fleet && parallel {
        fleet_worker_count(max, discourses.len(), 3, false)
    } else {
        1
    };

    let params = PushParams {
        remote_path: remote_path.to_string(),
        owner,
        group,
        mode,
        backup,
        sudo,
        local_checksum: local_checksum.clone(),
        local_size,
    };
    let local_for_workers = std::sync::Arc::clone(&local_bytes);

    let results: Vec<(String, Result<String>)> = run_fleet(
        &discourses,
        workers,
        move |d| {
            let name = d.name.clone();
            (name, push_one_forum(d, &params, &local_for_workers))
        },
        |(name, res)| match res {
            Ok(msg) => println!("{msg}"),
            Err(e) => eprintln!("{name}: push failed - {e:#}"),
        },
    );

    let failed = results.iter().filter(|(_, r)| r.is_err()).count();
    if failed > 0 {
        return Err(anyhow!("{failed} forum(s) failed push; see messages above"));
    }
    Ok(())
}

/// Immutable per-run parameters shared by every fleet worker.
struct PushParams<'a> {
    remote_path: String,
    owner: Option<&'a str>,
    group: Option<&'a str>,
    mode: Option<&'a str>,
    backup: bool,
    sudo: bool,
    local_checksum: String,
    local_size: u64,
}

/// Push the local bytes to one forum. Idempotent: a destination whose
/// checksum already matches is reported and skipped without an upload.
fn push_one_forum(
    discourse: &DiscourseConfig,
    params: &PushParams,
    local_bytes: &[u8],
) -> Result<String> {
    let target = ssh_target(discourse).map_err(|e| anyhow!("{:#}", e))?;

    let (remote_checksum, _size, status) =
        remote_file_checksum(target, &params.remote_path).map_err(|e| anyhow!("{e:#}"))?;
    if status != "missing" && remote_checksum.as_deref() == Some(params.local_checksum.as_str()) {
        return Ok(format!(
            "{}: {} is already up to date ({})",
            discourse.name, params.remote_path, params.local_checksum
        ));
    }
    let existing = matches!(status.as_str(), "different" | "same");

    let opts = ReplaceOptions {
        owner: params.owner,
        group: params.group,
        mode: params.mode,
        backup: params.backup && existing,
        sudo: params.sudo,
        expected_checksum: Some(&params.local_checksum),
    };
    let full_command = build_replace_script(&params.remote_path, &opts);
    let (stdout, _stderr) = run_ssh_pipe(target, &full_command, local_bytes, 1024)
        .map_err(|e| anyhow!("pushing to {remote}: {e:#}", remote = params.remote_path))?;

    let staged_output = String::from_utf8_lossy(&stdout);
    let staged_checksum = staged_output.split_whitespace().next().unwrap_or("");
    if staged_checksum != params.local_checksum {
        anyhow::bail!(
            "staged checksum mismatch: expected {}, got {}",
            params.local_checksum,
            staged_checksum
        );
    }

    Ok(format!(
        "{}: pushed to {} ({} bytes, {}){}",
        discourse.name,
        params.remote_path,
        params.local_size,
        params.local_checksum,
        if params.backup && existing {
            " (previous file backed up)"
        } else {
            ""
        }
    ))
}

fn backup_display(backup: bool, _forum_count: usize) -> &'static str {
    if backup {
        "timestamped backup of any existing destination"
    } else {
        "no backup (--no-backup)"
    }
}

fn remote_file_checksum(
    target: &str,
    remote_path: &str,
) -> Result<(Option<String>, Option<u64>, String)> {
    let command = format!(
        "if test -L {p}; then echo SYMLINK; elif test -f {p}; then sha256sum {p} | cut -d' ' -f1; stat -c ' %s' {p}; else echo MISSING; fi",
        p = shell_quote(remote_path)
    );
    let (stdout, _stderr) = run_ssh_capture(target, &command, 1024)
        .context(format!("auditing {remote_path} on {target}"))?;
    let output = String::from_utf8_lossy(&stdout);

    if output.trim() == "SYMLINK" {
        return Ok((None, None, "symlink".to_string()));
    }
    if output.trim() == "MISSING" {
        return Ok((None, None, "missing".to_string()));
    }

    let mut lines = output.lines();
    let checksum = lines.next().map(|s| s.trim().to_string());
    let size = lines.next().and_then(|s| s.trim().parse::<u64>().ok());

    let status = match (&checksum, &size) {
        (Some(_), Some(_)) => "present".to_string(),
        _ => "failed".to_string(),
    };

    Ok((checksum, size, status))
}

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

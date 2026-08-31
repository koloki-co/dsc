// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::cli::{FileCommand, ListFormat};
use crate::commands::common::{
    fleet_worker_count, run_fleet, select_discourse, selected_discourses, shell_quote,
};
use crate::commands::ssh::{
    ReplaceOptions, build_fetch_script, build_replace_script, effective_host_key_checking,
    run_ssh_capture, run_ssh_pipe,
};
use crate::config::{Config, DiscourseConfig};
use crate::utils::{atomic_write, ensure_output_available};
use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Caps the bytes read from a remote file during `dsc file pull`, so a
/// hostile or misconfigured remote cannot exhaust memory through an
/// unbounded response. Matches the ceiling `dsc` already applies to API
/// response bodies (`api::client::MAX_BODY_BYTES`); `dsc file` is a narrow
/// operational primitive for scripts and configuration, not bulk transfer.
const MAX_PULL_BYTES: usize = 64 * 1024 * 1024;

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
        FileCommand::Pull {
            discourse,
            remote_path,
            local_path,
            tags,
            parallel,
            max,
            overwrite,
            format,
        } => file_pull(
            config,
            discourse,
            remote_path,
            local_path,
            tags.as_deref(),
            *parallel,
            *max,
            *overwrite,
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

#[derive(Serialize)]
struct PullResult {
    forum: String,
    remote_path: String,
    local_path: String,
    checksum: Option<String>,
    size: Option<u64>,
    status: String,
}

/// The per-forum pull outcome used by both the single and fleet paths.
struct PullOutcome {
    forum: String,
    remote_path: String,
    local_path: String,
    checksum: Option<String>,
    size: Option<u64>,
    status: String,
    failed: bool,
}

fn render_pull_row(row: &PullOutcome) -> PullResult {
    PullResult {
        forum: row.forum.clone(),
        remote_path: row.remote_path.clone(),
        local_path: row.local_path.clone(),
        checksum: row.checksum.clone(),
        size: row.size,
        status: row.status.clone(),
    }
}

/// Derive `<discourse>--<remote-basename>` from an absolute remote path.
/// Rejects an empty, `.`, or `..` basename so a remote path ending in a
/// separator (or a traversal segment) cannot produce a local filename that
/// escapes the destination directory.
fn remote_basename(remote_path: &str) -> Result<&str> {
    let basename = remote_path.rsplit('/').next().unwrap_or(remote_path);
    if basename.is_empty() || basename == "." || basename == ".." {
        return Err(anyhow!(
            "cannot derive a local filename from remote path: {remote_path}"
        ));
    }
    Ok(basename)
}

#[allow(clippy::too_many_arguments)]
fn file_pull(
    config: &Config,
    discourse: &str,
    remote_path: &str,
    local_path: &Path,
    tags: Option<&str>,
    parallel: bool,
    max: Option<usize>,
    overwrite: bool,
    format: ListFormat,
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

    let basename = if fleet {
        if !local_path.is_dir() {
            return Err(anyhow!(
                "{} must be an existing directory when pulling from more than one forum",
                local_path.display()
            ));
        }
        Some(remote_basename(remote_path)?.to_string())
    } else {
        None
    };

    let destination_for = |name: &str| -> PathBuf {
        match &basename {
            Some(base) => local_path.join(format!("{name}--{base}")),
            None => local_path.to_path_buf(),
        }
    };

    // Validate every destination before any network fetch, so a collision
    // on one forum aborts the whole pull before the others are touched.
    for d in &discourses {
        let dest = destination_for(&d.name);
        ensure_output_available(&dest, overwrite)
            .map_err(rewrite_overwrite_hint)
            .with_context(|| format!("{}: {}", d.name, dest.display()))?;
    }

    let workers = if fleet && parallel {
        fleet_worker_count(max, discourses.len(), 8, false)
    } else {
        1
    };

    let outcomes: Vec<PullOutcome> = run_fleet(
        &discourses,
        workers,
        |d| {
            let dest = destination_for(&d.name);
            pull_one_forum(d, remote_path, &dest, overwrite)
        },
        |outcome| {
            if outcome.failed {
                eprintln!(
                    "{}: pull failed - {}",
                    outcome.forum,
                    outcome.status.trim_start_matches("failed: ")
                );
            } else if matches!(format, ListFormat::Text) {
                println!("{}", outcome.local_path);
            }
        },
    );

    let failed = outcomes.iter().filter(|o| o.failed).count();

    match format {
        ListFormat::Json => {
            let rows: Vec<PullResult> = outcomes.iter().map(render_pull_row).collect();
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        ListFormat::Yaml => {
            let rows: Vec<PullResult> = outcomes.iter().map(render_pull_row).collect();
            println!("{}", serde_yaml::to_string(&rows)?);
        }
        ListFormat::Text => {}
    }

    if failed > 0 {
        return Err(anyhow!("{failed} forum(s) failed pull; see messages above"));
    }
    Ok(())
}

/// Fetch one forum's remote file, verify its checksum, and write it to
/// `dest` atomically. Never follows a remote symlink or writes through a
/// local one.
fn pull_one_forum(
    discourse: &DiscourseConfig,
    remote_path: &str,
    dest: &Path,
    overwrite: bool,
) -> PullOutcome {
    match pull_one_forum_inner(discourse, remote_path, dest, overwrite) {
        Ok((checksum, size)) => PullOutcome {
            forum: discourse.name.clone(),
            remote_path: remote_path.to_string(),
            local_path: dest.display().to_string(),
            checksum: Some(checksum),
            size: Some(size),
            status: "pulled".to_string(),
            failed: false,
        },
        Err(e) => PullOutcome {
            forum: discourse.name.clone(),
            remote_path: remote_path.to_string(),
            local_path: dest.display().to_string(),
            checksum: None,
            size: None,
            status: format!("failed: {e:#}"),
            failed: true,
        },
    }
}

fn pull_one_forum_inner(
    discourse: &DiscourseConfig,
    remote_path: &str,
    dest: &Path,
    overwrite: bool,
) -> Result<(String, u64)> {
    let target = ssh_target(discourse).map_err(|e| anyhow!("{:#}", e))?;

    let script = build_fetch_script(remote_path);
    let (content, stderr) = run_ssh_capture(target, &script, MAX_PULL_BYTES)
        .map_err(|e| anyhow!("pulling {remote_path} from {target}: {e:#}"))?;

    let reported_checksum = stderr.trim();
    let actual_checksum = hex_sha256(&content);
    if reported_checksum != actual_checksum {
        anyhow::bail!(
            "checksum mismatch after transfer: remote reported {}, computed {actual_checksum}",
            if reported_checksum.is_empty() {
                "(none)"
            } else {
                reported_checksum
            }
        );
    }

    let size = content.len() as u64;
    atomic_write(dest, &content, overwrite)
        .map_err(rewrite_overwrite_hint)
        .with_context(|| format!("writing {}", dest.display()))?;
    Ok((actual_checksum, size))
}

/// `ensure_output_available`/`atomic_write`'s shared collision message
/// names `--force`, the flag every other pull command in `dsc` uses;
/// `dsc file pull` names its equivalent flag `--overwrite` (see
/// `spec/commands/file-transfer.md`'s proposed CLI surface), so the hint is
/// rewritten to name the flag that actually exists on this command.
fn rewrite_overwrite_hint(error: anyhow::Error) -> anyhow::Error {
    let message = error.to_string();
    if message.contains("pass --force to replace it") {
        anyhow!(message.replace(
            "pass --force to replace it",
            "pass --overwrite to replace it"
        ))
    } else {
        error
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

/// `dsc file pull` behaviour that isn't already covered by the fetch
/// script's own unit/fixture tests in `commands::ssh` (symlink and
/// non-regular-file refusal on the remote side): collision, `--overwrite`,
/// local symlink refusal, fleet filename convention, and partial failure.
/// Runs the real `tests/fixtures/fake-ssh` stand-in via the same
/// `FakeSshPath` helper `commands::ssh`'s own fixture tests use, so `ssh`
/// really resolves to a local shell rather than a canned mock.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::commands::ssh::fixture_tests::FakeSshPath;
    use crate::config::Config;

    fn mock_discourse(name: &str) -> DiscourseConfig {
        DiscourseConfig {
            name: name.to_string(),
            ssh_host: Some("remote.invalid".to_string()),
            ..DiscourseConfig::default()
        }
    }

    #[test]
    fn remote_basename_rejects_traversal_and_empty_segments() {
        assert_eq!(
            remote_basename("/var/discourse/scripts/update.sh").unwrap(),
            "update.sh"
        );
        assert!(remote_basename("/var/discourse/scripts/").is_err());
        assert!(remote_basename("/var/discourse/scripts/..").is_err());
        assert!(remote_basename("/").is_err());
    }

    #[test]
    fn pulls_a_regular_remote_file_and_verifies_its_checksum() {
        let _fake_ssh = FakeSshPath::install();
        let remote_dir = tempfile::tempdir().unwrap();
        let remote = remote_dir.path().join("update.sh");
        let content = b"#!/bin/sh\necho hi\n";
        std::fs::write(&remote, content).unwrap();

        let local_dir = tempfile::tempdir().unwrap();
        let dest = local_dir.path().join("update.sh");

        let (checksum, size) = pull_one_forum_inner(
            &mock_discourse("mock"),
            remote.to_str().unwrap(),
            &dest,
            false,
        )
        .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), content);
        assert_eq!(size, content.len() as u64);
        assert_eq!(checksum, hex_sha256(content));
    }

    #[test]
    fn refuses_an_existing_local_destination_without_overwrite() {
        let _fake_ssh = FakeSshPath::install();
        let remote_dir = tempfile::tempdir().unwrap();
        let remote = remote_dir.path().join("update.sh");
        std::fs::write(&remote, b"new content").unwrap();

        let local_dir = tempfile::tempdir().unwrap();
        let dest = local_dir.path().join("update.sh");
        std::fs::write(&dest, b"old content").unwrap();

        let error = pull_one_forum_inner(
            &mock_discourse("mock"),
            remote.to_str().unwrap(),
            &dest,
            false,
        )
        .unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("refusing to overwrite"));
        assert!(
            message.contains("--overwrite") && !message.contains("--force"),
            "the hint must name this command's actual flag, not the generic --force: {message}"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"old content");
    }

    #[test]
    fn overwrite_replaces_an_existing_local_destination() {
        let _fake_ssh = FakeSshPath::install();
        let remote_dir = tempfile::tempdir().unwrap();
        let remote = remote_dir.path().join("update.sh");
        std::fs::write(&remote, b"new content").unwrap();

        let local_dir = tempfile::tempdir().unwrap();
        let dest = local_dir.path().join("update.sh");
        std::fs::write(&dest, b"old content").unwrap();

        pull_one_forum_inner(
            &mock_discourse("mock"),
            remote.to_str().unwrap(),
            &dest,
            true,
        )
        .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"new content");
    }

    #[test]
    fn refuses_to_write_through_a_local_symlink_destination_even_with_overwrite() {
        let _fake_ssh = FakeSshPath::install();
        let remote_dir = tempfile::tempdir().unwrap();
        let remote = remote_dir.path().join("update.sh");
        std::fs::write(&remote, b"attempted overwrite").unwrap();

        let local_dir = tempfile::tempdir().unwrap();
        let target = local_dir.path().join("real-target");
        std::fs::write(&target, b"do not overwrite me").unwrap();
        let dest = local_dir.path().join("update.sh");
        std::os::unix::fs::symlink(&target, &dest).unwrap();

        let error = pull_one_forum_inner(
            &mock_discourse("mock"),
            remote.to_str().unwrap(),
            &dest,
            true,
        )
        .unwrap_err();

        assert!(
            format!("{error:#}")
                .to_lowercase()
                .contains("symbolic link")
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"do not overwrite me");
        assert!(
            std::fs::symlink_metadata(&dest)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the symlink itself must be left in place"
        );
    }

    #[test]
    fn fleet_pull_uses_flat_discourse_prefixed_filenames_and_reports_partial_failure() {
        let _fake_ssh = FakeSshPath::install();
        let remote_dir = tempfile::tempdir().unwrap();
        let remote = remote_dir.path().join("update.sh");
        let content = b"fleet content";
        std::fs::write(&remote, content).unwrap();

        // "alpha" has a working ssh_host and reaches the file above;
        // "beta" is missing ssh_host entirely, so its pull fails
        // client-side before any transfer - a simple, deterministic way to
        // force one fleet member to fail without the fake-ssh stand-in
        // needing to distinguish between "remote" hosts.
        let config = Config {
            discourse: vec![
                mock_discourse("alpha"),
                DiscourseConfig {
                    name: "beta".to_string(),
                    ssh_host: None,
                    ..DiscourseConfig::default()
                },
            ],
            ..Config::default()
        };

        let local_dir = tempfile::tempdir().unwrap();
        let error = file_pull(
            &config,
            "all",
            remote.to_str().unwrap(),
            local_dir.path(),
            None,
            false,
            None,
            false,
            ListFormat::Text,
        )
        .unwrap_err();

        assert!(error.to_string().contains("1 forum(s) failed pull"));
        assert_eq!(
            std::fs::read(local_dir.path().join("alpha--update.sh")).unwrap(),
            content
        );
        assert!(!local_dir.path().join("beta--update.sh").exists());
    }

    #[test]
    fn a_fleet_pull_requires_an_existing_local_destination_directory() {
        let config = Config {
            discourse: vec![mock_discourse("alpha"), mock_discourse("beta")],
            ..Config::default()
        };
        let missing_dir = tempfile::tempdir().unwrap().path().join("does-not-exist");

        let error = file_pull(
            &config,
            "all",
            "/var/discourse/scripts/update.sh",
            &missing_dir,
            None,
            false,
            None,
            false,
            ListFormat::Text,
        )
        .unwrap_err();

        assert!(error.to_string().contains("must be an existing directory"));
    }

    #[test]
    fn file_pull_refuses_an_existing_destination_before_any_fetch_and_hints_overwrite() {
        // No FakeSshPath here: an existing destination must be refused by
        // the pre-flight check before any SSH call is attempted, so this
        // must fail even without a working `ssh` on PATH.
        let config = Config {
            discourse: vec![mock_discourse("mock")],
            ..Config::default()
        };
        let local_dir = tempfile::tempdir().unwrap();
        let dest = local_dir.path().join("update.sh");
        std::fs::write(&dest, b"already here").unwrap();

        let error = file_pull(
            &config,
            "mock",
            "/var/discourse/scripts/update.sh",
            &dest,
            None,
            false,
            None,
            false,
            ListFormat::Text,
        )
        .unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("refusing to overwrite"));
        assert!(message.contains("--overwrite") && !message.contains("--force"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"already here");
    }
}

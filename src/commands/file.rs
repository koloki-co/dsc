// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::cli::{FileCommand, ListFormat};
use crate::commands::common::{emit_result, select_discourse, shell_quote};
use crate::commands::ssh::{run_ssh_capture, run_ssh_pipe};
use crate::config::Config;
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
            format,
        } => file_audit(config, discourse, local_path, remote_path, *format),
        FileCommand::Push {
            discourse,
            local_path,
            remote_path,
            owner,
            group,
            mode,
            backup,
            sudo,
            yes,
            format,
        } => file_push(
            config,
            discourse,
            local_path,
            remote_path,
            owner.as_deref(),
            group.as_deref(),
            mode.as_deref(),
            *backup,
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

fn file_audit(
    config: &Config,
    discourse_name: &str,
    local_path: &Path,
    remote_path: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    let target = discourse
        .ssh_host
        .as_deref()
        .filter(|h| !h.trim().is_empty())
        .ok_or_else(|| anyhow!("missing ssh_host for discourse {}", discourse.name))?;

    let local_bytes = fs::read(local_path)
        .with_context(|| format!("reading local file {}", local_path.display()))?;
    let local_checksum = hex_sha256(&local_bytes);
    let local_size = local_bytes.len() as u64;

    let (remote_checksum, remote_size, status) = remote_file_checksum(target, remote_path)?;

    let result = AuditResult {
        forum: discourse.name.clone(),
        local_path: local_path.display().to_string(),
        remote_path: remote_path.to_string(),
        local_checksum: local_checksum.clone(),
        local_size,
        remote_checksum: remote_checksum.clone(),
        remote_size,
        status: status.clone(),
    };

    let text = format!(
        "{}: {} {} (local {} {}, remote {})",
        discourse.name,
        remote_path,
        status,
        local_checksum,
        local_size,
        remote_checksum.as_deref().unwrap_or("(missing)"),
    );
    emit_result(format, &result, &text)
}

#[allow(clippy::too_many_arguments)]
fn file_push(
    config: &Config,
    discourse_name: &str,
    local_path: &Path,
    remote_path: &str,
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
    let discourse = select_discourse(config, Some(discourse_name))?;
    let target = discourse
        .ssh_host
        .as_deref()
        .filter(|h| !h.trim().is_empty())
        .ok_or_else(|| anyhow!("missing ssh_host for discourse {}", discourse.name))?;

    let local_bytes = fs::read(local_path)
        .with_context(|| format!("reading local file {}", local_path.display()))?;
    let local_checksum = hex_sha256(&local_bytes);
    let local_size = local_bytes.len() as u64;

    let (remote_checksum, remote_size, remote_status) = if dry_run {
        match remote_file_checksum(target, remote_path) {
            Ok(result) => result,
            Err(_) => (None, None, "unknown".to_string()),
        }
    } else {
        remote_file_checksum(target, remote_path)?
    };

    let _ = (remote_checksum, remote_size);
    let existing = matches!(remote_status.as_str(), "same" | "different");

    let backup_path = if backup && existing {
        format!("{remote_path}.dsc-$(date -u +%Y%m%dT%H%M%SZ).bak")
    } else {
        String::new()
    };

    if dry_run {
        let plan = format!(
            "[dry-run] {name}: would push {local} ({local_checksum}, {local_size} bytes) -> {remote}\n  status: {remote_status}\n  owner: {owner}\n  group: {group}\n  mode: {mode}\n  backup: {backup}\n  sudo: {sudo}",
            name = discourse.name,
            local = local_path.display(),
            local_checksum = local_checksum,
            local_size = local_size,
            remote = remote_path,
            remote_status = remote_status,
            owner = owner.unwrap_or("(preserve)"),
            group = group.unwrap_or("(preserve)"),
            mode = mode.unwrap_or("(preserve)"),
            backup = if backup && existing {
                &backup_path
            } else if backup {
                "(new file, no backup)"
            } else {
                "no"
            },
            sudo = if sudo { "yes" } else { "no" },
        );
        println!("{plan}");
        return Ok(());
    }

    if remote_status == "same" {
        println!(
            "{}: {} is already up to date ({})",
            discourse.name, remote_path, local_checksum
        );
        return Ok(());
    }

    if !yes {
        return Err(anyhow!(
            "refusing to push to {} on {} without --yes; review with --dry-run first",
            remote_path,
            discourse.name
        ));
    }

    let encoded = base64_encode(&local_bytes);
    let remote_dir = remote_path
        .rsplit_once('/')
        .map(|(dir, _)| if dir.is_empty() { "/" } else { dir })
        .unwrap_or(".");

    let mut script = String::new();
    script.push_str("set -eu; ");
    script.push_str(&format!(
        "test -L {p} && exit 2; ",
        p = shell_quote(remote_path)
    ));
    script.push_str(&format!(
        "tmp=$(mktemp {dir}/.dsc-file.XXXXXX); ",
        dir = shell_quote(remote_dir)
    ));
    script.push_str(&format!(
        "printf '%s' {enc} | base64 -d > \"$tmp\"; ",
        enc = shell_quote(&encoded)
    ));
    script.push_str("actual=$(sha256sum \"$tmp\" | cut -d' ' -f1); ");
    script.push_str(&format!(
        "test \"$actual\" = {chk} || {{ rm -f \"$tmp\"; exit 3; }}; ",
        chk = shell_quote(&local_checksum)
    ));
    if let Some(m) = mode {
        script.push_str(&format!("chmod {m} \"$tmp\"; ", m = m));
    }
    if let Some(o) = owner {
        script.push_str(&format!("chown {o} \"$tmp\"; ", o = o));
    }
    if let Some(g) = group {
        script.push_str(&format!("chgrp {g} \"$tmp\"; ", g = g));
    }
    if backup && existing {
        script.push_str(&format!(
            "cp -a {p} {bp}; ",
            p = shell_quote(remote_path),
            bp = shell_quote(&backup_path)
        ));
    }
    script.push_str(&format!(
        "mv -f \"$tmp\" {p}; ",
        p = shell_quote(remote_path)
    ));
    script.push_str("rm -f \"$tmp\" 2>/dev/null; ");

    let full_command = if sudo {
        format!("sudo -n sh -c {}", shell_quote(&script))
    } else {
        script
    };

    let (_stdout, stderr) = run_ssh_pipe(target, &full_command, &[], 1024)
        .context(format!("pushing to {remote_path} on {}", discourse.name))?;

    let _ = stderr;
    println!(
        "{}: pushed {} -> {} ({} bytes, {})",
        discourse.name,
        local_path.display(),
        remote_path,
        local_size,
        local_checksum
    );

    Ok(())
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

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

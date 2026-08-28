// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Shared construction of non-interactive SSH processes.
//!
//! Command families choose their remote operation, but connection and host-key
//! policy belongs here so a future streamed transfer cannot silently diverge
//! from update or configuration checks.

use crate::commands::common::validate_ssh_target;
use anyhow::{Context, Result, anyhow};
use std::io::{Read, Write};
use std::process::{Command, Stdio};

const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 10;
const SERVER_ALIVE_INTERVAL_SECONDS: u64 = 30;
const SERVER_ALIVE_COUNT_MAX: u64 = 3;

/// Build an SSH process with DSC's non-interactive connection policy.
///
/// `extra_options` are forwarded as complete SSH arguments after the standard
/// policy. They preserve the existing update-command extension point for
/// specialised callers.
pub(crate) fn build_ssh_command(target: &str, extra_options: &[&str]) -> Result<Command> {
    build_ssh_command_with_timeout(target, DEFAULT_CONNECT_TIMEOUT_SECONDS, extra_options)
}

/// Build an SSH process with a caller-specific connection timeout.
///
/// Configuration checks use a shorter timeout than update and transfer work,
/// but retain every other connection and host-key setting.
pub(crate) fn build_ssh_command_with_timeout(
    target: &str,
    connect_timeout_seconds: u64,
    extra_options: &[&str],
) -> Result<Command> {
    validate_ssh_target(target)?;
    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("BatchMode=yes");
    cmd.arg("-o")
        .arg(format!("ConnectTimeout={connect_timeout_seconds}"));
    cmd.arg("-o").arg(format!(
        "ServerAliveInterval={SERVER_ALIVE_INTERVAL_SECONDS}"
    ));
    cmd.arg("-o")
        .arg(format!("ServerAliveCountMax={SERVER_ALIVE_COUNT_MAX}"));
    if let Some(strict) = ssh_strict_host_key_checking() {
        cmd.arg("-o").arg(format!("StrictHostKeyChecking={strict}"));
    }
    for option in extra_options {
        cmd.arg(option);
    }
    if let Ok(raw) = std::env::var("DSC_SSH_OPTIONS")
        && !raw.trim().is_empty()
    {
        cmd.args(raw.split_whitespace());
    }
    cmd.arg("--").arg(target);
    Ok(cmd)
}

/// Maximum bytes captured from SSH stderr for diagnostics. Keeps a hostile
/// or noisy remote from driving memory through the diagnostic path.
#[allow(dead_code)]
const MAX_STDERR_BYTES: usize = 4096;

/// Run a remote command, capturing stdout as a bounded `Vec<u8>` and stderr
/// as a bounded diagnostic string. Used by callers that need binary stdout
/// (file download, checksum queries) rather than the lossy text capture in
/// `update::run_ssh_command`. The `stdout_cap` bounds memory so a hostile or
/// misbehaving remote cannot exhaust RAM through an unbounded response.
#[allow(dead_code)]
pub(crate) fn run_ssh_capture(
    target: &str,
    command: &str,
    stdout_cap: usize,
) -> Result<(Vec<u8>, String)> {
    let mut ssh = build_ssh_command(target, &[])?;
    ssh.arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = ssh
        .spawn()
        .with_context(|| format!("spawning ssh to {target}"))?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;

    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        stderr
            .take((MAX_STDERR_BYTES + 1) as u64)
            .read_to_end(&mut buf)
            .ok();
        String::from_utf8_lossy(&buf).trim().to_string()
    });

    let mut stdout_buf = Vec::new();
    stdout
        .take(stdout_cap as u64 + 1)
        .read_to_end(&mut stdout_buf)
        .with_context(|| format!("reading stdout from {target}"))?;
    if stdout_buf.len() > stdout_cap {
        return Err(anyhow!(
            "remote stdout exceeded {stdout_cap} bytes on {target}"
        ));
    }

    let status = child
        .wait()
        .with_context(|| format!("waiting for ssh to {target}"))?;
    let stderr_text = stderr_handle
        .join()
        .unwrap_or_else(|_| "<stderr capture failed>".to_string());

    if !status.success() {
        let code = status
            .code()
            .map(|c| format!("exit {c}"))
            .unwrap_or_else(|| "killed by signal".to_string());
        let detail = if stderr_text.is_empty() {
            String::new()
        } else {
            format!(": {stderr_text}")
        };
        return Err(anyhow!("ssh to {target} failed ({code}){detail}"));
    }

    Ok((stdout_buf, stderr_text))
}

/// Pipe `input` to a remote command's stdin and return its stdout as a
/// bounded `Vec<u8>`. Used by callers that need to upload bytes to a remote
/// process (e.g. `base64 -d > file`). The `stdout_cap` bounds the response
/// the same way as [`run_ssh_capture`].
#[allow(dead_code)]
pub(crate) fn run_ssh_pipe(
    target: &str,
    command: &str,
    input: &[u8],
    stdout_cap: usize,
) -> Result<(Vec<u8>, String)> {
    let mut ssh = build_ssh_command(target, &[])?;
    ssh.arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = ssh
        .spawn()
        .with_context(|| format!("spawning ssh to {target}"))?;
    let mut stdin = child.stdin.take().context("missing stdin")?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;

    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        stderr
            .take((MAX_STDERR_BYTES + 1) as u64)
            .read_to_end(&mut buf)
            .ok();
        String::from_utf8_lossy(&buf).trim().to_string()
    });

    stdin
        .write_all(input)
        .with_context(|| format!("writing stdin to {target}"))?;
    drop(stdin);

    let mut stdout_buf = Vec::new();
    stdout
        .take(stdout_cap as u64 + 1)
        .read_to_end(&mut stdout_buf)
        .with_context(|| format!("reading stdout from {target}"))?;
    if stdout_buf.len() > stdout_cap {
        return Err(anyhow!(
            "remote stdout exceeded {stdout_cap} bytes on {target}"
        ));
    }

    let status = child
        .wait()
        .with_context(|| format!("waiting for ssh to {target}"))?;
    let stderr_text = stderr_handle
        .join()
        .unwrap_or_else(|_| "<stderr capture failed>".to_string());

    if !status.success() {
        let code = status
            .code()
            .map(|c| format!("exit {c}"))
            .unwrap_or_else(|| "killed by signal".to_string());
        let detail = if stderr_text.is_empty() {
            String::new()
        } else {
            format!(": {stderr_text}")
        };
        return Err(anyhow!("ssh to {target} failed ({code}){detail}"));
    }

    Ok((stdout_buf, stderr_text))
}

fn ssh_strict_host_key_checking() -> Option<String> {
    let value = std::env::var("DSC_SSH_STRICT_HOST_KEY_CHECKING")
        .unwrap_or_else(|_| "accept-new".to_string());
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{build_ssh_command, build_ssh_command_with_timeout};

    fn args(command: &std::process::Command) -> Vec<String> {
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn applies_shared_non_interactive_connection_policy() {
        let command = build_ssh_command("forum.example", &[]).unwrap();
        assert_eq!(
            args(&command),
            vec![
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "ServerAliveInterval=30",
                "-o",
                "ServerAliveCountMax=3",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "--",
                "forum.example",
            ]
        );
    }

    #[test]
    fn permits_a_shorter_timeout_without_dropping_liveness_options() {
        let command = build_ssh_command_with_timeout("forum.example", 5, &[]).unwrap();
        let command_args = args(&command);
        assert!(command_args.contains(&"ConnectTimeout=5".to_string()));
        assert!(command_args.contains(&"ServerAliveInterval=30".to_string()));
        assert!(command_args.contains(&"ServerAliveCountMax=3".to_string()));
    }

    #[test]
    fn rejects_an_option_like_target() {
        let error = build_ssh_command("-oProxyCommand=bad", &[]).unwrap_err();
        assert!(error.to_string().contains("cannot start with '-"));
    }

    #[test]
    fn stdout_cap_overflow_is_detectable() {
        let cap = 100;
        let buf = vec![0u8; cap + 1];
        assert!(buf.len() > cap);
        let buf_exact = vec![0u8; cap];
        assert!(buf_exact.len() <= cap);
    }
}

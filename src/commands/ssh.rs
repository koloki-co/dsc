// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Shared construction of non-interactive SSH processes.
//!
//! Command families choose their remote operation, but connection and host-key
//! policy belongs here so a future streamed transfer cannot silently diverge
//! from update or configuration checks.

use crate::commands::common::validate_ssh_target;
use anyhow::Result;
use std::process::Command;

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
}

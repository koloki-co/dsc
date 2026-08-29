// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Shared construction of non-interactive SSH processes.
//!
//! Command families choose their remote operation, but connection and host-key
//! policy belongs here so a future streamed transfer cannot silently diverge
//! from update or configuration checks.

use crate::commands::common::{shell_quote, validate_ssh_target};
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

/// Run a remote command and return its stdout as a `String`. This is the
/// text counterpart of [`run_ssh_capture`] for callers that need the full
/// stdout but don't need binary-safe handling. Caps stdout at 1 MiB to
/// prevent a hostile remote from exhausting memory through a text response.
pub(crate) fn run_ssh_text(target: &str, command: &str) -> Result<String> {
    const TEXT_CAP: usize = 1024 * 1024;
    let (bytes, _stderr) = run_ssh_capture(target, command, TEXT_CAP)?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
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

/// Ownership, mode, backup, and verification requests for
/// [`build_replace_script`].
///
/// `owner`/`group`/`mode` are applied to the staged file only, never to the
/// destination directly or via `--reference`, so a symlink target already at
/// the destination cannot influence the replacement.
///
/// `expected_checksum`, when set, is the SHA-256 hex digest the uploaded
/// bytes must produce. The script verifies the staged file against it
/// *before* any metadata, backup, or rename step runs, so a corrupted
/// transfer aborts with the destination untouched.
#[allow(dead_code)]
pub(crate) struct ReplaceOptions<'a> {
    pub(crate) owner: Option<&'a str>,
    pub(crate) group: Option<&'a str>,
    pub(crate) mode: Option<&'a str>,
    pub(crate) backup: bool,
    pub(crate) sudo: bool,
    pub(crate) expected_checksum: Option<&'a str>,
}

/// Build the remote stage-and-replace script for `dsc file push`'s no-follow
/// atomic replacement protocol (see the "Remote no-follow replacement
/// protocol" section of `spec/commands/file-transfer.md`).
///
/// The returned string is a single shell invocation, intended to be run via
/// [`run_ssh_pipe`] with the uploaded file's bytes as stdin. In one shell
/// process it: refuses an existing symlink destination, stages the uploaded
/// bytes into a same-directory temporary file, reports the staged file's
/// checksum (for the caller to compare against the local one), optionally
/// applies ownership/mode and a timestamped backup of any existing
/// destination, then atomically renames the staged file over the
/// destination. Refusing the symlink and performing the rename in the same
/// shell process closes the TOCTOU window between the check and the
/// replacement.
#[allow(dead_code)]
pub(crate) fn build_replace_script(remote_path: &str, opts: &ReplaceOptions) -> String {
    let script = build_replace_script_body(remote_path, opts);
    if opts.sudo {
        format!("sudo -n sh -c {}", shell_quote(&script))
    } else {
        format!("sh -c {}", shell_quote(&script))
    }
}

/// The stage-and-replace shell logic itself, before it is wrapped as the
/// single `sh -c`/`sudo -n sh -c` argument [`build_replace_script`] sends
/// over SSH. Split out so tests can assert on the unescaped script content
/// rather than re-deriving the outer quoting.
fn build_replace_script_body(remote_path: &str, opts: &ReplaceOptions) -> String {
    let dir = remote_path.rsplit_once('/').map_or(".", |(dir, _)| dir);
    let quoted_path = shell_quote(remote_path);
    let mut script = format!(
        "set -eu; test -L {quoted_path} && exit 2; tmp=$(mktemp {}/.dsc-file.XXXXXX); trap 'rm -f \"$tmp\"' EXIT; cat > \"$tmp\"; sha256sum \"$tmp\"",
        shell_quote(dir),
    );
    if let Some(expected) = opts.expected_checksum {
        script.push_str(&format!(
            "; actual=$(sha256sum \"$tmp\" | cut -d' ' -f1); test \"$actual\" = {} || exit 3",
            shell_quote(expected),
        ));
    }
    if let Some(owner) = opts.owner {
        script.push_str(&format!("; chown {} \"$tmp\"", shell_quote(owner)));
    }
    if let Some(group) = opts.group {
        script.push_str(&format!("; chgrp {} \"$tmp\"", shell_quote(group)));
    }
    if let Some(mode) = opts.mode {
        script.push_str(&format!("; chmod {} \"$tmp\"", shell_quote(mode)));
    }
    if opts.backup {
        script.push_str(&format!(
            "; if [ -e {quoted_path} ]; then cp -a {quoted_path} {quoted_path}.dsc-$(date -u +%Y%m%dT%H%M%SZ).bak; fi",
        ));
    }
    script.push_str(&format!("; mv -f \"$tmp\" {quoted_path}"));
    script
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
    use super::{
        ReplaceOptions, build_replace_script, build_replace_script_body, build_ssh_command,
        build_ssh_command_with_timeout,
    };

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

    fn no_op_options() -> ReplaceOptions<'static> {
        ReplaceOptions {
            owner: None,
            group: None,
            mode: None,
            backup: false,
            sudo: false,
            expected_checksum: None,
        }
    }

    #[test]
    fn replace_script_checks_symlink_before_staging() {
        let script =
            build_replace_script_body("/var/discourse/scripts/update.sh", &no_op_options());
        let check_pos = script.find("test -L").unwrap();
        let mktemp_pos = script.find("mktemp").unwrap();
        assert!(
            check_pos < mktemp_pos,
            "the symlink refusal must run before any staged file is created: {script}"
        );
    }

    #[test]
    fn replace_script_stages_and_replaces_atomically() {
        let script =
            build_replace_script_body("/var/discourse/scripts/update.sh", &no_op_options());
        assert!(script.starts_with("set -eu"));
        assert!(script.contains("cat > \"$tmp\""));
        assert!(script.contains("sha256sum \"$tmp\""));
        assert!(script.contains("mv -f \"$tmp\" '/var/discourse/scripts/update.sh'"));
        assert!(!script.contains("chown"));
        assert!(!script.contains("chgrp"));
        assert!(!script.contains("chmod"));
        assert!(!script.contains(".bak"));

        // The wrapped form sent over SSH is a single `sh -c` argument.
        let wrapped = build_replace_script("/var/discourse/scripts/update.sh", &no_op_options());
        assert!(wrapped.starts_with("sh -c "));
    }

    #[test]
    fn replace_script_applies_ownership_and_mode_to_the_staged_file_only() {
        let opts = ReplaceOptions {
            owner: Some("root"),
            group: Some("www-data"),
            mode: Some("0755"),
            backup: false,
            sudo: false,
            expected_checksum: None,
        };
        let script = build_replace_script_body("/etc/example.conf", &opts);
        assert!(script.contains("chown 'root' \"$tmp\""));
        assert!(script.contains("chgrp 'www-data' \"$tmp\""));
        assert!(script.contains("chmod '0755' \"$tmp\""));
        assert!(!script.contains("--reference"));
    }

    #[test]
    fn replace_script_backs_up_an_existing_destination_before_replacing() {
        let opts = ReplaceOptions {
            backup: true,
            ..no_op_options()
        };
        let script = build_replace_script_body("/etc/example.conf", &opts);
        assert!(script.contains("if [ -e '/etc/example.conf' ]"));
        assert!(script.contains("cp -a '/etc/example.conf' '/etc/example.conf'.dsc-"));
        let backup_pos = script.find(".bak").unwrap();
        let replace_pos = script.find("mv -f").unwrap();
        assert!(
            backup_pos < replace_pos,
            "the backup must be taken before the destination is replaced: {script}"
        );
    }

    #[test]
    fn replace_script_wraps_in_non_interactive_sudo_only_when_requested() {
        let opts = ReplaceOptions {
            sudo: true,
            ..no_op_options()
        };
        let sudo_script = build_replace_script("/etc/example.conf", &opts);
        assert!(sudo_script.starts_with("sudo -n sh -c "));

        let plain_script = build_replace_script("/etc/example.conf", &no_op_options());
        assert!(!plain_script.contains("sudo"));
    }

    #[test]
    fn replace_script_quotes_a_path_containing_a_single_quote() {
        let script = build_replace_script_body("/var/discourse/it's.txt", &no_op_options());
        assert!(script.contains(r"'/var/discourse/it'\''s.txt'"));
    }

    #[test]
    fn replace_script_verifies_the_expected_checksum_before_replacing() {
        let opts = ReplaceOptions {
            expected_checksum: Some("abc123"),
            ..no_op_options()
        };
        let script = build_replace_script_body("/etc/example.conf", &opts);
        let verify_pos = script.find("test \"$actual\" = 'abc123'").unwrap();
        let chown_pos = script.find("chown").unwrap_or(script.len());
        let backup_pos = script.find("cp -a").unwrap_or(script.len());
        let mv_pos = script.find("mv -f").unwrap();
        assert!(
            verify_pos < mv_pos && verify_pos < backup_pos && verify_pos < chown_pos,
            "checksum verification must precede every mutation and the rename: {script}"
        );
        assert!(script.contains("|| exit 3"));
    }
}

/// Integration tests that run [`build_replace_script`] (and the plain
/// [`run_ssh_capture`]/[`run_ssh_pipe`] transport) against a real shell via
/// the `tests/fixtures/fake-ssh` fixture, with a scratch directory standing
/// in for the remote filesystem. See that fixture's header comment and the
/// "Isolated SSH/process fixtures" section of `spec/commands/file-transfer.md`
/// for why this is a real local execution rather than a canned mock: it
/// exercises dsc's actual argument construction and the protocol's real
/// shell logic, not just parsed intent.
#[cfg(all(test, unix))]
mod fixture_tests {
    use super::{ReplaceOptions, build_replace_script, run_ssh_capture, run_ssh_pipe};
    use std::os::unix::fs::{MetadataExt, symlink};
    use std::sync::Mutex;

    // `PATH` is process-wide, and `cargo test` runs a binary's tests on
    // multiple threads by default - so every test that installs the fake
    // `ssh` must hold this lock for the duration of its SSH calls, or two
    // tests could race and see each other's PATH.
    static PATH_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct FakeSshPath {
        original: Option<String>,
        // Held only so the symlink directory outlives the PATH override;
        // never read directly.
        _bin_dir: tempfile::TempDir,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl FakeSshPath {
        fn install() -> Self {
            let guard = PATH_ENV_LOCK
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-ssh");
            // `Command::new("ssh")` resolves the literal name `ssh` on PATH,
            // so the fixture (named `fake-ssh` to avoid a file literally
            // called `ssh` in the repo) is symlinked under that name into a
            // scratch directory that is prepended to PATH just for this
            // guard's lifetime.
            let bin_dir = tempfile::tempdir().expect("create fake-ssh PATH dir");
            symlink(fixture, bin_dir.path().join("ssh")).expect("symlink fake-ssh as ssh");

            let original = std::env::var("PATH").ok();
            let new_path = match &original {
                Some(existing) => format!("{}:{existing}", bin_dir.path().display()),
                None => bin_dir.path().display().to_string(),
            };
            // SAFETY: serialised by `PATH_ENV_LOCK`, held for this guard's
            // whole lifetime, so no other thread observes a torn PATH.
            unsafe { std::env::set_var("PATH", new_path) };
            Self {
                original,
                _bin_dir: bin_dir,
                _guard: guard,
            }
        }
    }

    impl Drop for FakeSshPath {
        fn drop(&mut self) {
            // SAFETY: see `install` - still under `PATH_ENV_LOCK`.
            unsafe {
                match &self.original {
                    Some(value) => std::env::set_var("PATH", value),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    fn push_bytes(
        remote_path: &str,
        opts: &ReplaceOptions,
        bytes: &[u8],
    ) -> anyhow::Result<Vec<u8>> {
        let _fake_ssh = FakeSshPath::install();
        let script = build_replace_script(remote_path, opts);
        run_ssh_pipe("remote.invalid", &script, bytes, 4096).map(|(stdout, _stderr)| stdout)
    }

    #[test]
    fn replaces_a_missing_destination_with_uploaded_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("update.sh");
        let content = b"#!/bin/sh\necho updated\n";

        push_bytes(
            dest.to_str().unwrap(),
            &ReplaceOptions {
                owner: None,
                group: None,
                mode: None,
                backup: false,
                sudo: false,
                expected_checksum: None,
            },
            content,
        )
        .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), content);
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".dsc-file.")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "no staged temporary file should remain after a successful replace"
        );
    }

    #[test]
    fn replaces_an_existing_destination_and_reports_the_staged_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("update.sh");
        std::fs::write(&dest, b"old content").unwrap();
        let content = b"new content";

        let stdout = push_bytes(
            dest.to_str().unwrap(),
            &ReplaceOptions {
                owner: None,
                group: None,
                mode: None,
                backup: false,
                sudo: false,
                expected_checksum: None,
            },
            content,
        )
        .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), content);
        let mut hasher_output = String::from_utf8_lossy(&stdout).to_string();
        hasher_output.truncate(64);
        assert_eq!(hasher_output.len(), 64, "expected a sha256 hex digest");
    }

    #[test]
    fn refuses_a_symlink_destination_without_touching_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real-target");
        std::fs::write(&target, b"do not overwrite me").unwrap();
        let dest = dir.path().join("update.sh");
        symlink(&target, &dest).unwrap();

        let error = push_bytes(
            dest.to_str().unwrap(),
            &ReplaceOptions {
                owner: None,
                group: None,
                mode: None,
                backup: false,
                sudo: false,
                expected_checksum: None,
            },
            b"attempted overwrite",
        )
        .unwrap_err();

        assert!(error.to_string().contains("exit 2"));
        assert_eq!(std::fs::read(&target).unwrap(), b"do not overwrite me");
        assert!(
            std::fs::symlink_metadata(&dest)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the symlink itself must be left in place"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".dsc-file.")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "the symlink check must run before any staged file is created"
        );
    }

    #[test]
    fn backs_up_the_previous_destination_before_replacing_it() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("update.sh");
        std::fs::write(&dest, b"old content").unwrap();

        push_bytes(
            dest.to_str().unwrap(),
            &ReplaceOptions {
                owner: None,
                group: None,
                mode: None,
                backup: true,
                sudo: false,
                expected_checksum: None,
            },
            b"new content",
        )
        .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"new content");
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".dsc-"))
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".bak"))
            .collect();
        assert_eq!(backups.len(), 1, "expected exactly one backup file");
        assert_eq!(
            std::fs::read(backups[0].path()).unwrap(),
            b"old content",
            "the backup must preserve the pre-replacement content"
        );
    }

    #[test]
    fn applies_the_requested_mode_to_the_replaced_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("update.sh");

        push_bytes(
            dest.to_str().unwrap(),
            &ReplaceOptions {
                owner: None,
                group: None,
                mode: Some("0640"),
                backup: false,
                sudo: false,
                expected_checksum: None,
            },
            b"content",
        )
        .unwrap();

        let mode = std::fs::metadata(&dest).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    #[test]
    fn a_checksum_mismatch_aborts_before_the_destination_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("update.sh");
        std::fs::write(&dest, b"original content").unwrap();

        let error = push_bytes(
            dest.to_str().unwrap(),
            &ReplaceOptions {
                owner: None,
                group: None,
                mode: None,
                backup: false,
                sudo: false,
                expected_checksum: Some("0".repeat(64).leak() as &str),
            },
            b"corrupted in transit",
        )
        .unwrap_err();

        assert!(error.to_string().contains("exit 3"));
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"original content",
            "a checksum mismatch must leave the destination untouched"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".dsc-file.")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "the trap must clean up the staged file after a checksum failure"
        );
    }

    #[test]
    fn strips_sudo_before_local_execution_so_the_fixture_needs_no_privileges() {
        let _fake_ssh = FakeSshPath::install();
        let (stdout, _stderr) =
            run_ssh_capture("remote.invalid", "sudo -n echo unprivileged", 64).unwrap();
        assert_eq!(String::from_utf8_lossy(&stdout).trim(), "unprivileged");
    }

    #[test]
    fn stdout_cap_is_enforced_against_a_real_oversized_remote_response() {
        let _fake_ssh = FakeSshPath::install();
        let error = run_ssh_capture("remote.invalid", "head -c 200000 /dev/zero", 100).unwrap_err();
        assert!(error.to_string().contains("exceeded 100 bytes"));
    }

    #[test]
    fn a_remote_command_that_never_reads_stdin_surfaces_a_pipe_write_error() {
        let _fake_ssh = FakeSshPath::install();
        let large_input = vec![b'x'; 4 * 1024 * 1024];
        let result = run_ssh_pipe("remote.invalid", "exit 0", &large_input, 64);
        assert!(
            result.is_err(),
            "writing 4 MiB into a command that exits immediately without reading stdin should fail"
        );
    }

    #[test]
    fn remote_stderr_and_exit_status_are_surfaced_on_failure() {
        let _fake_ssh = FakeSshPath::install();
        let error =
            run_ssh_capture("remote.invalid", "echo diagnostic >&2; exit 7", 64).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("exit 7"));
        assert!(message.contains("diagnostic"));
    }
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{DiscourseClient, VersionInfo};
use crate::commands::common::{
    ensure_api_credentials, missing_config, shell_quote, validate_ssh_target,
};
use crate::commands::update_log::{self, LogKind};
use crate::config::{Config, DiscourseConfig, find_discourse};
use crate::utils::color_discourse_label;
use anyhow::{Context, Result, anyhow};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::blocking::Client;
use std::collections::{HashSet, VecDeque};
use std::io::{self, Write};
use std::io::{BufRead, BufReader, IsTerminal};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const DEFAULT_PARALLEL_UPDATE_WORKERS: usize = 3;
const KIBIBYTE: u64 = 1024;
const GIBIBYTE: u64 = 1024 * 1024 * 1024;
const MAX_REMOTE_DIAGNOSTIC_LINES: usize = 20;
const MAX_REMOTE_DIAGNOSTIC_CHARS: usize = 4096;
/// Window for "was this forum updated recently?" when `--skip-recent` is given
/// without a value, or for the interactive re-update prompt.
const DEFAULT_RECENT_WINDOW: Duration = Duration::from_secs(24 * 3600);

pub fn update_one(
    config: &Config,
    name: &str,
    post_changelog: bool,
    yes: bool,
    force: bool,
    skip_recent: Option<Duration>,
) -> Result<()> {
    let discourse =
        find_discourse(config, name).ok_or_else(|| anyhow!("discourse not found: {}", name))?;

    if !force && skip_recent_single(&discourse.name, skip_recent) {
        update_log::append(&discourse.name, LogKind::SkippedRecent, "-", "-", "-");
        println!(
            "{}: fully updated recently - skipping (use --force to update anyway)",
            discourse.name
        );
        return Ok(());
    }

    update_and_log(discourse, post_changelog, yes, force)
}

pub fn update_all(
    config: &Config,
    parallel: Option<usize>,
    post_changelog: bool,
    yes: bool,
    force: bool,
    skip_recent: Option<Duration>,
) -> Result<()> {
    let updatable: Vec<DiscourseConfig> = config
        .discourse
        .iter()
        .filter(|d| d.ssh_host.is_some())
        .cloned()
        .collect();

    // Decide (prompting if needed) which recently-updated forums to skip - up
    // front, before any slow work, so the rest of the run is unattended.
    let skip_set = recent_skip_set(&updatable, skip_recent, force);
    for d in &updatable {
        if skip_set.contains(&d.name) {
            update_log::append(&d.name, LogKind::SkippedRecent, "-", "-", "-");
            println!(
                "{}: updated recently - skipping (--force to override)",
                d.name
            );
        }
    }
    let to_update: Vec<DiscourseConfig> = updatable
        .into_iter()
        .filter(|d| !skip_set.contains(&d.name))
        .collect();

    let Some(width) = parallel else {
        for discourse in &to_update {
            update_and_log(discourse, post_changelog, yes, force)?;
        }
        return Ok(());
    };

    let max_threads = parallel_worker_count(Some(width), to_update.len());
    let mut handles: Vec<thread::JoinHandle<Result<()>>> = Vec::new();
    for discourse in to_update {
        if handles.len() >= max_threads
            && let Some(handle) = handles.pop()
        {
            handle.join().expect("thread panicked")?;
        }
        let do_post = post_changelog;
        let auto_yes = yes;
        handles.push(thread::spawn(move || {
            update_and_log(&discourse, do_post, auto_yes, force)
        }));
    }

    for handle in handles {
        handle.join().expect("thread panicked")?;
    }

    Ok(())
}

/// Run one forum's update and record the outcome (updated / current /
/// skipped-rebuild / failed) in the update log.
fn update_and_log(
    discourse: &DiscourseConfig,
    post_changelog: bool,
    yes: bool,
    force: bool,
) -> Result<()> {
    match run_update(discourse, force) {
        Ok(UpdateOutcome::Updated(metadata)) => {
            let kind = if metadata.discourse_rebuilt {
                LogKind::Updated
            } else {
                LogKind::Current
            };
            update_log::append(
                &discourse.name,
                kind,
                metadata.before_version.as_deref().unwrap_or("-"),
                metadata.after_version.as_deref().unwrap_or("-"),
                "-",
            );
            let payload = print_update_summary(discourse, &metadata);
            if post_changelog {
                handle_changelog_post(discourse, &payload, yes)?;
            }
            Ok(())
        }
        Ok(UpdateOutcome::SkippedRebuildInProgress) => {
            update_log::append(&discourse.name, LogKind::SkippedRebuild, "-", "-", "-");
            Ok(())
        }
        Err(e) => {
            update_log::append(
                &discourse.name,
                LogKind::Failed,
                "-",
                "-",
                &concise_failure_detail(&e),
            );
            Err(e)
        }
    }
}

/// Single forum: skip it as recently updated? Explicit `--skip-recent` window
/// skips silently; otherwise (interactive TTY) prompt.
fn skip_recent_single(name: &str, skip_recent: Option<Duration>) -> bool {
    match skip_recent {
        Some(window) => update_log::updated_within(name, window),
        None => {
            io::stdin().is_terminal()
                && update_log::updated_within(name, DEFAULT_RECENT_WINDOW)
                && !prompt_yes_no(
                    &format!("{name} was fully updated within the last 24h - update it again?"),
                    false,
                )
        }
    }
}

/// `update all`: the set of forum names to skip as recently updated. Any prompt
/// happens here, once, before the run starts.
fn recent_skip_set(
    updatable: &[DiscourseConfig],
    skip_recent: Option<Duration>,
    force: bool,
) -> HashSet<String> {
    if force {
        return HashSet::new();
    }
    let window = skip_recent.unwrap_or(DEFAULT_RECENT_WINDOW);
    let recent: Vec<&str> = updatable
        .iter()
        .map(|d| d.name.as_str())
        .filter(|n| update_log::updated_within(n, window))
        .collect();
    if recent.is_empty() {
        return HashSet::new();
    }
    // Explicit --skip-recent: skip silently. No flag + interactive: prompt once.
    // No flag + non-interactive: skip nothing (unchanged behaviour).
    let skip_them = if skip_recent.is_some() {
        true
    } else if io::stdin().is_terminal() {
        println!(
            "These {} were fully updated within the last 24h:",
            recent.len()
        );
        for n in &recent {
            println!("  - {n}");
        }
        !prompt_yes_no("Update them again anyway?", false)
    } else {
        false
    };
    if skip_them {
        recent.iter().map(|s| s.to_string()).collect()
    } else {
        HashSet::new()
    }
}

fn prompt_yes_no(question: &str, default_yes: bool) -> bool {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("{question} {hint} ");
    let _ = io::stdout().flush();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return default_yes;
    }
    match input.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default_yes,
    }
}

fn parallel_worker_count(max: Option<usize>, discourse_count: usize) -> usize {
    let requested = max.unwrap_or(DEFAULT_PARALLEL_UPDATE_WORKERS).max(1);
    requested.min(discourse_count.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct FakeDiskOps {
        measurements: VecDeque<u64>,
        cleanup_calls: usize,
        older_base_image_ids: Vec<String>,
        image_list_fails: bool,
    }

    impl FakeDiskOps {
        fn new(measurements: &[u64]) -> Self {
            Self {
                measurements: measurements.iter().copied().collect(),
                cleanup_calls: 0,
                older_base_image_ids: Vec::new(),
                image_list_fails: false,
            }
        }
    }

    impl DiskRecoveryOps for FakeDiskOps {
        fn available_bytes(&mut self) -> Result<u64> {
            self.measurements
                .pop_front()
                .ok_or_else(|| anyhow!("missing fake disk measurement"))
        }

        fn cleanup(&mut self) -> Result<()> {
            self.cleanup_calls += 1;
            Ok(())
        }

        fn older_base_image_ids(&mut self) -> Result<Vec<String>> {
            if self.image_list_fails {
                Err(anyhow!("image listing failed"))
            } else {
                Ok(self.older_base_image_ids.clone())
            }
        }
    }

    #[test]
    fn default_parallel_workers_is_three() {
        assert_eq!(parallel_worker_count(None, 10), 3);
    }

    #[test]
    fn max_workers_is_capped_by_discourse_count() {
        assert_eq!(parallel_worker_count(Some(8), 2), 2);
    }

    #[test]
    fn rebuild_check_avoids_self_match() {
        // The bracketed pattern must match a real `./launcher rebuild` but NOT
        // contain the literal "launcher rebuild" - otherwise pgrep matches its
        // own shell and every host looks busy.
        assert!(super::REBUILD_CHECK_CMD.contains("[l]auncher rebuild"));
        assert!(!super::REBUILD_CHECK_CMD.contains("launcher rebuild"));
    }

    #[test]
    fn successful_ssh_status_ignores_git_progress_on_stderr() {
        let stderr = "From https://github.com/discourse/discourse_docker\n   e7f1201..7d4fa59 main -> origin/main\n * [new branch] build-cache -> origin/build-cache\n";
        assert!(
            ensure_ssh_success("Discourse rebuild", "forum", true, Some(0), "", stderr).is_ok()
        );
    }

    #[test]
    fn failed_ssh_status_reports_exit_code_even_without_stderr() {
        let error = ensure_ssh_success("Discourse rebuild", "forum", false, Some(23), "", "")
            .unwrap_err()
            .to_string();
        assert_eq!(error, "Discourse rebuild failed on forum (ssh exit 23)");
    }

    #[test]
    fn failed_ssh_status_keeps_stdout_and_stderr_as_context() {
        let error = ensure_ssh_success(
            "Discourse rebuild",
            "forum",
            false,
            Some(1),
            "substantive launcher failure",
            "From https://github.com/discourse/discourse_docker",
        )
        .unwrap_err()
        .to_string();
        assert!(error.starts_with("Discourse rebuild failed on forum (ssh exit 1)"));
        assert!(error.contains("stdout (tail):\nsubstantive launcher failure"));
        assert!(error.contains("stderr (tail):\nFrom https://github.com"));
        assert_eq!(
            concise_failure_detail(&anyhow!(error)),
            "Discourse rebuild failed on forum (ssh exit 1)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn streamed_command_uses_exit_status_and_preserves_both_streams() {
        let mut success = Command::new("sh");
        success.args([
            "-c",
            "printf 'launcher complete\\n'; printf 'From git.example/repo\\n' >&2; exit 0",
        ]);
        let stdout =
            run_command_with_tail(success, "forum", "Discourse rebuild", "testing", 0).unwrap();
        assert_eq!(stdout, "launcher complete\n");

        let mut failure = Command::new("sh");
        failure.args([
            "-c",
            "printf 'launcher failed\\n'; printf 'From git.example/repo\\n' >&2; exit 17",
        ]);
        let error = run_command_with_tail(failure, "forum", "Discourse rebuild", "testing", 0)
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("Discourse rebuild failed on forum (ssh exit 17)"));
        assert!(error.contains("stdout (tail):\nlauncher failed"));
        assert!(error.contains("stderr (tail):\nFrom git.example/repo"));
    }

    #[test]
    fn sufficient_disk_skips_recovery() {
        let mut ops = FakeDiskOps::new(&[6 * GIBIBYTE]);
        let outcome = recover_disk_space(&mut ops, 5 * GIBIBYTE).unwrap();
        assert_eq!(outcome, DiskSpaceOutcome::Sufficient);
        assert_eq!(ops.cleanup_calls, 0);
    }

    #[test]
    fn disk_threshold_is_exact() {
        let minimum = 5 * GIBIBYTE;
        let mut exact = FakeDiskOps::new(&[minimum]);
        assert_eq!(
            recover_disk_space(&mut exact, minimum).unwrap(),
            DiskSpaceOutcome::Sufficient
        );

        let mut below = FakeDiskOps::new(&[minimum - 1, minimum - 1]);
        assert!(matches!(
            recover_disk_space(&mut below, minimum).unwrap(),
            DiskSpaceOutcome::Insufficient(_)
        ));
    }

    #[test]
    fn cleanup_that_recovers_enough_space_proceeds() {
        let mut ops = FakeDiskOps::new(&[4 * GIBIBYTE, 6 * GIBIBYTE]);
        let outcome = recover_disk_space(&mut ops, 5 * GIBIBYTE).unwrap();
        let DiskSpaceOutcome::Recovered(report) = outcome else {
            panic!("expected recovered disk outcome");
        };
        assert_eq!(ops.cleanup_calls, 1);
        assert_eq!(report.initial_available_bytes, 4 * GIBIBYTE);
        assert_eq!(report.final_available_bytes, 6 * GIBIBYTE);
    }

    #[test]
    fn low_disk_reports_older_base_images_without_removing_them() {
        let image_id = format!("sha256:{}", "a".repeat(64));
        let mut ops = FakeDiskOps::new(&[4 * GIBIBYTE, 4 * GIBIBYTE]);
        ops.older_base_image_ids.push(image_id.clone());
        let outcome = recover_disk_space(&mut ops, 5 * GIBIBYTE).unwrap();
        let DiskSpaceOutcome::Insufficient(report) = outcome else {
            panic!("expected insufficient disk outcome");
        };
        assert_eq!(report.older_base_image_ids, vec![image_id]);
    }

    #[test]
    fn insufficient_disk_error_includes_safe_manual_commands() {
        let image_id = format!("sha256:{}", "c".repeat(64));
        let report = DiskRecoveryReport {
            initial_available_bytes: 4 * GIBIBYTE,
            final_available_bytes: 4 * GIBIBYTE,
            cleanup_error: None,
            image_list_error: None,
            older_base_image_ids: vec![image_id.clone()],
        };
        let error = insufficient_disk_error("forum", false, 5 * GIBIBYTE, &report).to_string();
        assert!(error.contains("4.00 GiB -> 4.00 GiB available"));
        assert!(error.contains("sudo docker system df"));
        assert!(error.contains("sudo docker image ls --no-trunc discourse/base"));
        assert!(error.contains(&format!("sudo docker rmi {image_id}")));
        assert!(error.contains("intentionally omits `--force`"));
        assert!(error.contains("Do not use `docker system prune -a`"));
    }

    #[test]
    fn insufficient_disk_error_shell_quotes_the_ssh_target() {
        let report = DiskRecoveryReport {
            initial_available_bytes: 4 * GIBIBYTE,
            final_available_bytes: 4 * GIBIBYTE,
            cleanup_error: None,
            image_list_error: None,
            older_base_image_ids: Vec::new(),
        };
        let error =
            insufficient_disk_error("forum; touch /tmp/not-safe", true, 5 * GIBIBYTE, &report)
                .to_string();
        assert!(error.contains("ssh 'forum; touch /tmp/not-safe'"));
    }

    #[test]
    fn insufficient_disk_error_never_rounds_available_space_up_to_the_minimum() {
        let minimum = 5 * GIBIBYTE;
        let report = DiskRecoveryReport {
            initial_available_bytes: minimum - KIBIBYTE,
            final_available_bytes: minimum - KIBIBYTE,
            cleanup_error: None,
            image_list_error: None,
            older_base_image_ids: Vec::new(),
        };
        let error = insufficient_disk_error("forum", true, minimum, &report).to_string();
        assert!(error.contains("4.99 GiB -> 4.99 GiB available (minimum 5.00 GiB"));
    }

    #[test]
    fn image_id_parser_deduplicates_and_rejects_shell_input() {
        let digest = "D".repeat(64);
        let parsed =
            parse_discourse_base_image_ids(&format!("sha256:{digest}\nsha256:{digest}\n")).unwrap();
        assert_eq!(
            parsed,
            vec![format!("sha256:{}", digest.to_ascii_lowercase())]
        );
        assert!(parse_discourse_base_image_ids("sha256:abc; reboot").is_err());
    }

    #[test]
    fn older_image_selection_preserves_the_newest_image() {
        let newest = "a".repeat(64);
        let older = "b".repeat(64);
        let older_images =
            older_discourse_base_image_ids(&format!("sha256:{newest}\nsha256:{older}\n")).unwrap();
        assert_eq!(older_images, vec![format!("sha256:{older}")]);
    }

    #[test]
    fn root_disk_parser_uses_exact_available_kibibytes() {
        let output = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/vda1 10485760 6291456 4194304 60% /\n";
        assert_eq!(
            parse_root_disk_available_bytes(output).unwrap(),
            4 * GIBIBYTE
        );
        assert!(parse_root_disk_available_bytes("").is_err());
        assert!(parse_root_disk_available_bytes("unexpected").is_err());
    }

    #[test]
    fn docker_commands_preserve_rootless_mode() {
        assert_eq!(
            docker_command(false, "rmi sha256:abc"),
            "sudo -n docker rmi sha256:abc"
        );
        assert_eq!(
            docker_command(true, "rmi sha256:abc"),
            "docker rmi sha256:abc"
        );
        assert_eq!(
            preflight_cleanup_command(false),
            "sudo -n docker image prune -f"
        );
        assert_eq!(preflight_cleanup_command(true), "docker image prune -f");
        assert!(!default_cleanup_command(false).contains("prune -a"));
        assert!(!default_cleanup_command(true).contains("prune -a"));
    }
}

struct UpdateMetadata {
    before_version: Option<String>,
    before_commit: Option<String>,
    after_version: Option<String>,
    after_commit: Option<String>,
    reclaimed_space: Option<String>,
    before_os_version: Option<String>,
    after_version_error: Option<String>,
    root_disk_usage: Option<String>,
    preflight_disk_recovery: Option<Box<DiskRecoveryReport>>,
    os_updated: bool,
    server_rebooted: bool,
    /// Whether the Discourse rebuild actually ran (vs skipped as already current).
    discourse_rebuilt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiskRecoveryReport {
    initial_available_bytes: u64,
    final_available_bytes: u64,
    cleanup_error: Option<String>,
    image_list_error: Option<String>,
    older_base_image_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DiskSpaceOutcome {
    Sufficient,
    Recovered(DiskRecoveryReport),
    Insufficient(DiskRecoveryReport),
}

trait DiskRecoveryOps {
    fn available_bytes(&mut self) -> Result<u64>;
    fn cleanup(&mut self) -> Result<()>;
    fn older_base_image_ids(&mut self) -> Result<Vec<String>>;
}

struct SshDiskRecovery<'a> {
    target: &'a str,
    rootless: bool,
}

impl DiskRecoveryOps for SshDiskRecovery<'_> {
    fn available_bytes(&mut self) -> Result<u64> {
        get_root_disk_available_bytes(self.target)
    }

    fn cleanup(&mut self) -> Result<()> {
        let command = preflight_cleanup_command(self.rootless);
        run_ssh_command_combined_named(self.target, &command, "Preflight Docker cleanup")
            .map(|_| ())
    }

    fn older_base_image_ids(&mut self) -> Result<Vec<String>> {
        let command = docker_command(self.rootless, "image ls --no-trunc --quiet discourse/base");
        let output = run_ssh_command_named(self.target, &command, "Listing discourse/base images")?;
        older_discourse_base_image_ids(&output)
    }
}

fn recover_disk_space<O: DiskRecoveryOps>(
    ops: &mut O,
    minimum_bytes: u64,
) -> Result<DiskSpaceOutcome> {
    let initial_available_bytes = ops.available_bytes()?;
    if initial_available_bytes >= minimum_bytes {
        return Ok(DiskSpaceOutcome::Sufficient);
    }

    let cleanup_error = ops
        .cleanup()
        .err()
        .map(|error| concise_failure_detail(&error));
    let after_cleanup_bytes = ops.available_bytes()?;
    let mut report = DiskRecoveryReport {
        initial_available_bytes,
        final_available_bytes: after_cleanup_bytes,
        cleanup_error,
        image_list_error: None,
        older_base_image_ids: Vec::new(),
    };
    if after_cleanup_bytes >= minimum_bytes {
        return Ok(DiskSpaceOutcome::Recovered(report));
    }

    report.older_base_image_ids = match ops.older_base_image_ids() {
        Ok(image_ids) => image_ids,
        Err(error) => {
            report.image_list_error = Some(concise_failure_detail(&error));
            Vec::new()
        }
    };
    Ok(DiskSpaceOutcome::Insufficient(report))
}

fn preflight_cleanup_command(rootless: bool) -> String {
    // Only dangling images are removed automatically. Do not reuse the
    // configurable post-update hook here: it may be non-idempotent or broader
    // than is appropriate before an update has started.
    docker_command(rootless, "image prune -f")
}

fn default_cleanup_command(rootless: bool) -> String {
    // Both Docker prune commands prompt for confirmation by default. `-f`
    // makes the existing cleanup deterministic under non-interactive SSH.
    if rootless {
        "docker container prune -f && docker image prune -f".to_string()
    } else {
        "sudo -n docker container prune -f && sudo -n docker image prune -f".to_string()
    }
}

fn docker_command(rootless: bool, arguments: &str) -> String {
    if rootless {
        format!("docker {arguments}")
    } else {
        format!("sudo -n docker {arguments}")
    }
}

fn manual_docker_command(rootless: bool, arguments: &str) -> String {
    if rootless {
        format!("docker {arguments}")
    } else {
        format!("sudo docker {arguments}")
    }
}

fn parse_discourse_base_image_ids(output: &str) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut image_ids = Vec::new();
    for raw in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let digest = raw.strip_prefix("sha256:").unwrap_or(raw);
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(anyhow!(
                "Docker returned an invalid image ID; refusing automatic removal"
            ));
        }
        let image_id = format!("sha256:{}", digest.to_ascii_lowercase());
        if seen.insert(image_id.clone()) {
            image_ids.push(image_id);
        }
    }
    Ok(image_ids)
}

fn older_discourse_base_image_ids(output: &str) -> Result<Vec<String>> {
    let image_ids = parse_discourse_base_image_ids(output)?;
    // Docker lists images newest-first. Preserve the newest base image and only
    // show older IDs as no-force manual `docker rmi` candidates. Creation order
    // alone does not prove an image is unused, so dsc never removes these itself.
    Ok(image_ids.into_iter().skip(1).collect())
}

fn format_gib(bytes: u64) -> String {
    let whole = bytes / GIBIBYTE;
    let hundredths = ((bytes % GIBIBYTE) as u128 * 100 / GIBIBYTE as u128) as u64;
    format!("{whole}.{hundredths:02} GiB")
}

fn insufficient_disk_error(
    target: &str,
    rootless: bool,
    minimum_bytes: u64,
    report: &DiskRecoveryReport,
) -> anyhow::Error {
    let reclaimed = report
        .final_available_bytes
        .saturating_sub(report.initial_available_bytes);
    let mut message = format!(
        "insufficient disk space on {target} after automatic recovery: {} -> {} available (minimum {}; reclaimed {})",
        format_gib(report.initial_available_bytes),
        format_gib(report.final_available_bytes),
        format_gib(minimum_bytes),
        format_gib(reclaimed)
    );
    if let Some(error) = &report.cleanup_error {
        message.push_str(&format!("\nAutomatic Docker cleanup failed: {error}"));
    }
    if let Some(error) = &report.image_list_error {
        message.push_str(&format!("\nStale base-image inspection failed: {error}"));
    }

    message.push_str(&format!(
        "\n\nSafe manual checks and cleanup:\n  ssh {}\n  {}\n  {}",
        shell_quote(target),
        manual_docker_command(rootless, "system df"),
        manual_docker_command(rootless, "image ls --no-trunc discourse/base")
    ));
    if report.older_base_image_ids.is_empty() {
        message.push_str(&format!(
            "\n  {}  # replace IMAGE_ID with a confirmed-unused older ID from the list above",
            manual_docker_command(rootless, "rmi IMAGE_ID")
        ));
    } else {
        for image_id in &report.older_base_image_ids {
            message.push_str(&format!(
                "\n  {}",
                manual_docker_command(rootless, &format!("rmi {image_id}"))
            ));
        }
    }
    message.push_str(
        "\n  df -h /\n\nConfirm an older image is unused before running its `docker rmi` suggestion. The command intentionally omits `--force`, so Docker can refuse images still in use. Do not use `docker system prune -a`, which can remove the current base image. If journal retention policy permits, inspect it with `sudo journalctl --disk-usage` before considering a bounded vacuum.",
    );
    anyhow!(message)
}

fn concise_failure_detail(error: &anyhow::Error) -> String {
    error
        .to_string()
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or("update failed")
        .to_string()
}

/// The result of one forum's update pass.
enum UpdateOutcome {
    Updated(Box<UpdateMetadata>),
    /// A `./launcher rebuild` was already running on the host, so the whole
    /// forum was left untouched (no OS update, no reboot, no rebuild).
    SkippedRebuildInProgress,
}

/// Detect an in-progress `./launcher rebuild` on the host. The `[l]auncher`
/// bracket keeps pgrep from matching its own shell (its cmdline holds
/// `[l]auncher`, not `launcher`), and the `&& … || …` makes the check always
/// exit 0 so a "not running" result isn't surfaced as an ssh error.
const REBUILD_CHECK_CMD: &str =
    "pgrep -f '[l]auncher rebuild' >/dev/null 2>&1 && echo REBUILDING || echo IDLE";

fn rebuild_in_progress(target: &str) -> Result<bool> {
    Ok(run_ssh_command(target, REBUILD_CHECK_CMD)?.contains("REBUILDING"))
}

fn run_update(discourse: &DiscourseConfig, force: bool) -> Result<UpdateOutcome> {
    let client = DiscourseClient::new(discourse)?;
    let target = discourse
        .ssh_host
        .clone()
        .unwrap_or_else(|| discourse.name.clone());
    let discourse_label = colored_discourse_display(discourse);
    println!("\n==> Updating {} ({})", discourse_label, target);

    // Never step on a rebuild that's already running - dsc's first destructive
    // act would be a reboot, which would kill an in-flight `./launcher rebuild`.
    if !force {
        stage(&discourse_label, "Checking for an in-progress rebuild");
        if rebuild_in_progress(&target)? {
            stage(
                &discourse_label,
                "A ./launcher rebuild is already running - skipping this forum (use --force to override)",
            );
            return Ok(UpdateOutcome::SkippedRebuildInProgress);
        }
    }

    stage(
        &discourse_label,
        "Fetching Discourse version (before update)",
    );
    let before_info = match client.fetch_version_info() {
        Ok(info) => {
            let label = info.version.as_deref().unwrap_or("unknown");
            stage(
                &discourse_label,
                &format!("Initial Discourse Version (before update): {}", label),
            );
            info
        }
        Err(err) => {
            stage(
                &discourse_label,
                &format!(
                    "Initial Discourse Version (before update): unknown (fetch failed: {})",
                    err
                ),
            );
            VersionInfo {
                version: None,
                commit: None,
            }
        }
    };
    stage(&discourse_label, "Fetching OS details");
    let before_os_version = match get_os_version(&target) {
        Ok(version) => {
            let label = version.as_deref().unwrap_or("unknown");
            stage(&discourse_label, &format!("OS: {}", label));
            version
        }
        Err(err) => {
            stage(
                &discourse_label,
                &format!(
                    "Initial OS Version (before update): unknown (fetch failed: {})",
                    err
                ),
            );
            None
        }
    };

    let rootless = discourse.docker_rootless.unwrap_or(false);

    stage(&discourse_label, "Checking root disk free space");
    let min_free_gb = std::env::var("DSC_DISCOURSE_MIN_FREE_GB")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|gb| *gb > 0)
        .unwrap_or(5);
    let min_free_bytes = min_free_gb.saturating_mul(GIBIBYTE);
    let mut disk_ops = SshDiskRecovery {
        target: &target,
        rootless,
    };
    let preflight_disk_recovery = match recover_disk_space(&mut disk_ops, min_free_bytes)? {
        DiskSpaceOutcome::Sufficient => None,
        DiskSpaceOutcome::Recovered(report) => {
            stage(
                &discourse_label,
                &format!(
                    "Preflight disk recovery: {} -> {} available",
                    format_gib(report.initial_available_bytes),
                    format_gib(report.final_available_bytes)
                ),
            );
            Some(Box::new(report))
        }
        DiskSpaceOutcome::Insufficient(report) => {
            return Err(insufficient_disk_error(
                &target,
                rootless,
                min_free_bytes,
                &report,
            ));
        }
    };

    let os_update_cmd = std::env::var("DSC_SSH_OS_UPDATE_CMD").unwrap_or_else(|_| {
        "sudo -n DEBIAN_FRONTEND=noninteractive apt update && sudo -n DEBIAN_FRONTEND=noninteractive apt upgrade -y"
            .to_string()
    });
    let reboot_cmd =
        std::env::var("DSC_SSH_REBOOT_CMD").unwrap_or_else(|_| "sudo -n reboot".to_string());
    let discourse_update_cmd = std::env::var("DSC_SSH_UPDATE_CMD").unwrap_or_else(|_| {
        if rootless {
            "cd /var/discourse && ./launcher rebuild app".to_string()
        } else {
            "cd /var/discourse && sudo -n ./launcher rebuild app".to_string()
        }
    });
    let cleanup_cmd =
        std::env::var("DSC_SSH_CLEANUP_CMD").unwrap_or_else(|_| default_cleanup_command(rootless));
    let mut server_rebooted = false;

    stage(&discourse_label, "Running OS update");
    if let Err(err) = run_ssh_command_with_tail(
        &target,
        &os_update_cmd,
        "OS update",
        "OS update in progress",
        3,
    ) {
        if let Some(rollback_cmd) = os_update_rollback_cmd() {
            stage(&discourse_label, "Running OS update rollback");
            if let Err(rollback_err) = run_ssh_command(&target, &rollback_cmd) {
                eprintln!(
                    "Warning: OS update rollback failed for {}: {}",
                    target, rollback_err
                );
            }
        }
        return Err(err);
    }
    let os_updated = true;
    stage(&discourse_label, "Rebooting server");
    if run_ssh_command(&target, &reboot_cmd).is_ok() {
        server_rebooted = true;
        if std::env::var("DSC_SSH_OS_UPDATE_CMD").unwrap_or_default() != "echo OS packages updated"
        {
            stage(&discourse_label, "Waiting for server to come back online");
            std::thread::sleep(std::time::Duration::from_secs(30));
            let mut attempts = 0;
            let max_attempts = 12;
            while attempts < max_attempts {
                match ssh_probe(&target) {
                    Ok(true) => break,
                    Ok(false) | Err(_) => {
                        attempts += 1;
                        if attempts < max_attempts {
                            println!(
                                "[{}] Still waiting for SSH (attempt {}/{})",
                                discourse_label,
                                attempts + 1,
                                max_attempts
                            );
                            std::thread::sleep(std::time::Duration::from_secs(30));
                        }
                    }
                }
            }
            if attempts >= max_attempts {
                return Err(anyhow!("Server did not come back online after reboot"));
            }
        }
    }

    stage(&discourse_label, "Checking if Discourse update is needed");
    let discourse_up_to_date = is_discourse_up_to_date(before_info.commit.as_deref());
    let discourse_rebuilt = !discourse_up_to_date;
    if discourse_up_to_date {
        stage(
            &discourse_label,
            "Discourse is already at the latest stable commit — skipping rebuild",
        );
    } else {
        stage(&discourse_label, "Running Discourse update");
        run_ssh_command_with_tail(
            &target,
            &discourse_update_cmd,
            "Discourse rebuild",
            "Discourse update in progress",
            3,
        )?;
    }
    stage(&discourse_label, "Waiting for Discourse to serve pages");
    let wait_secs = std::env::var("DSC_DISCOURSE_BOOT_WAIT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(15);
    std::thread::sleep(std::time::Duration::from_secs(wait_secs));
    stage(
        &discourse_label,
        "Fetching Discourse version (after update)",
    );
    let mut after_version_error = None;
    let after_info = match fetch_version_info_with_retry(&client, 6) {
        Ok(info) => {
            let label = info.version.as_deref().unwrap_or("unknown");
            stage(
                &discourse_label,
                &format!("Final Discourse Version (after update): {}", label),
            );
            info
        }
        Err(err) => {
            let message = format!("{}", err);
            after_version_error = Some(message.clone());
            stage(
                &discourse_label,
                &format!(
                    "Final Discourse Version (after update): unknown (fetch failed: {})",
                    message
                ),
            );
            VersionInfo {
                version: None,
                commit: None,
            }
        }
    };
    stage(&discourse_label, "Running cleanup");
    let cleanup = run_ssh_command_combined_named(&target, &cleanup_cmd, "Docker cleanup")?;
    let reclaimed_space = parse_reclaimed_space(&cleanup);
    // No OS version check after update; routine updates don't upgrade OS versions.
    stage(&discourse_label, "Fetching root disk usage");
    let root_disk_usage = match get_root_disk_usage(&target) {
        Ok(output) => Some(output),
        Err(err) => {
            stage(
                &discourse_label,
                &format!("Root disk usage: unknown (fetch failed: {})", err),
            );
            None
        }
    };

    Ok(UpdateOutcome::Updated(Box::new(UpdateMetadata {
        before_version: before_info.version,
        before_commit: before_info.commit,
        after_version: after_info.version,
        after_commit: after_info.commit,
        reclaimed_space,
        before_os_version,
        after_version_error,
        root_disk_usage,
        preflight_disk_recovery,
        os_updated,
        server_rebooted,
        discourse_rebuilt,
    })))
}

pub(crate) fn run_ssh_command(target: &str, command: &str) -> Result<String> {
    run_ssh_command_named(target, command, "SSH command")
}

fn run_ssh_command_named(target: &str, command: &str, step: &str) -> Result<String> {
    let mut cmd = build_ssh_command(target, &[])?;
    let output = cmd
        .arg(command)
        .output()
        .with_context(|| format!("running ssh to {}: {}", target, command))?;
    ensure_ssh_success(
        step,
        target,
        output.status.success(),
        output.status.code(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    )?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_ssh_command_combined_named(target: &str, command: &str, step: &str) -> Result<String> {
    let mut cmd = build_ssh_command(target, &[])?;
    let output = cmd
        .arg(command)
        .output()
        .with_context(|| format!("running ssh to {}: {}", target, command))?;
    ensure_ssh_success(
        step,
        target,
        output.status.success(),
        output.status.code(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    )?;
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(combined)
}

struct LineEvent {
    is_stderr: bool,
    line: String,
}

fn run_ssh_command_with_tail(
    target: &str,
    command: &str,
    step: &str,
    message: &str,
    tail_lines: usize,
) -> Result<String> {
    let mut command_process = build_ssh_command(target, &[])?;
    command_process.arg(command);
    run_command_with_tail(command_process, target, step, message, tail_lines)
}

fn run_command_with_tail(
    mut command: Command,
    target: &str,
    step: &str,
    message: &str,
    tail_lines: usize,
) -> Result<String> {
    let use_progress = io::stderr().is_terminal();
    let pb = if use_progress {
        ProgressBar::new_spinner()
    } else {
        ProgressBar::hidden()
    };
    if use_progress {
        let style = ProgressStyle::with_template("{spinner} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner());
        pb.set_style(style);
        pb.enable_steady_tick(Duration::from_millis(120));
    }

    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {step} command for {target}"))?;

    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;

    let (tx, rx) = mpsc::channel::<LineEvent>();
    let tx_out = tx.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    let _ = tx_out.send(LineEvent {
                        is_stderr: false,
                        line,
                    });
                }
                Err(_) => break,
            }
        }
    });

    let tx_err = tx.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    let _ = tx_err.send(LineEvent {
                        is_stderr: true,
                        line,
                    });
                }
                Err(_) => break,
            }
        }
    });

    drop(tx);

    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();
    let mut tail: VecDeque<String> = VecDeque::new();
    let base = format!("[{}] {}", target, message);
    pb.set_message(base.clone());

    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(event) => {
                if event.is_stderr {
                    stderr_buf.push_str(&event.line);
                    stderr_buf.push('\n');
                } else {
                    stdout_buf.push_str(&event.line);
                    stdout_buf.push('\n');
                }

                if tail_lines > 0 {
                    if tail.len() == tail_lines {
                        tail.pop_front();
                    }
                    tail.push_back(event.line);

                    let mut msg = base.clone();
                    for line in &tail {
                        msg.push('\n');
                        msg.push_str("  ");
                        msg.push_str(line);
                    }
                    pb.set_message(msg);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let status = child.wait().context("waiting for ssh command")?;
    pb.finish_and_clear();

    ensure_ssh_success(
        step,
        target,
        status.success(),
        status.code(),
        &stdout_buf,
        &stderr_buf,
    )?;

    Ok(stdout_buf)
}

fn ensure_ssh_success(
    step: &str,
    target: &str,
    success: bool,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Result<()> {
    if success {
        return Ok(());
    }

    let status = exit_code
        .map(|code| format!("ssh exit {code}"))
        .unwrap_or_else(|| "ssh terminated by signal".to_string());
    let mut message = format!("{step} failed on {target} ({status})");
    append_remote_diagnostic(&mut message, "stdout", stdout);
    append_remote_diagnostic(&mut message, "stderr", stderr);
    Err(anyhow!(message))
}

fn append_remote_diagnostic(message: &mut String, stream: &str, output: &str) {
    let output = output.trim();
    if output.is_empty() {
        return;
    }
    message.push_str(&format!("\n{stream} (tail):\n{}", diagnostic_tail(output)));
}

fn diagnostic_tail(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let start = lines.len().saturating_sub(MAX_REMOTE_DIAGNOSTIC_LINES);
    let tail = lines[start..].join("\n");
    if tail.chars().count() <= MAX_REMOTE_DIAGNOSTIC_CHARS {
        tail
    } else {
        let truncated: String = tail
            .chars()
            .rev()
            .take(MAX_REMOTE_DIAGNOSTIC_CHARS)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("[truncated]\n{truncated}")
    }
}

fn build_ssh_command(target: &str, extra_options: &[&str]) -> Result<std::process::Command> {
    validate_ssh_target(target)?;
    let mut cmd = std::process::Command::new("ssh");
    cmd.arg("-o").arg("BatchMode=yes");
    if let Some(strict) = ssh_strict_host_key_checking() {
        cmd.arg("-o")
            .arg(format!("StrictHostKeyChecking={}", strict));
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

fn ssh_probe(target: &str) -> Result<bool> {
    let mut cmd = build_ssh_command(target, &["-o", "ConnectTimeout=10"])?;
    let output = cmd
        .arg("echo 'server is up'")
        .output()
        .with_context(|| format!("running ssh probe to {}", target))?;
    Ok(output.status.success())
}

fn stage(target: &str, message: &str) {
    println!("[{}] {}", target, message);
}

fn print_update_summary(discourse: &DiscourseConfig, metadata: &UpdateMetadata) -> String {
    let payload = build_changelog_payload(metadata);
    let discourse_label = colored_discourse_display(discourse);
    println!("\nUpdate summary for {}:", discourse_label);
    for line in payload.lines() {
        println!("{}", line);
    }
    println!();
    payload
}

fn discourse_display_name(discourse: &DiscourseConfig) -> String {
    if let Some(fullname) = discourse.fullname.as_deref() {
        let trimmed = fullname.trim();
        if !trimmed.is_empty() {
            return format!("{} [{}]", trimmed, discourse.name);
        }
    }
    discourse.name.clone()
}

fn colored_discourse_display(discourse: &DiscourseConfig) -> String {
    let label = discourse_display_name(discourse);
    color_discourse_label(&label, &discourse.name)
}

fn get_os_version(target: &str) -> Result<Option<String>> {
    let version_cmd = std::env::var("DSC_SSH_OS_VERSION_CMD")
        .unwrap_or_else(|_| "lsb_release -d | cut -f2".to_string());
    match run_ssh_command(target, &version_cmd) {
        Ok(output) => Ok(Some(output.trim().to_string())),
        Err(_) => {
            let fallback_cmd = "grep PRETTY_NAME /etc/os-release | cut -d'=' -f2 | tr -d '\"'";
            match run_ssh_command(target, fallback_cmd) {
                Ok(output) => Ok(Some(output.trim().to_string())),
                Err(_) => Ok(None),
            }
        }
    }
}

fn parse_reclaimed_space(output: &str) -> Option<String> {
    let cleaned = strip_ansi_codes(output);
    // Use the last match: container prune runs first (typically 0B) and image prune runs
    // second (the meaningful amount), so the last "Total reclaimed space:" is what matters.
    cleaned
        .lines()
        .filter_map(|line| {
            let lower = line.to_ascii_lowercase();
            let idx = lower.find("total reclaimed space:")?;
            let (_, rest) = line.split_at(idx);
            rest.split_once(':')
                .map(|x| x.1)
                .map(|value| value.trim().to_string())
        })
        .next_back()
}

fn get_root_disk_usage(target: &str) -> Result<String> {
    let cmd = "df -h / | awk 'NR==2 {print $2 \" total, \" $3 \" used, \" $4 \" available, \" $5 \" used\"}'";
    let output = run_ssh_command(target, cmd)?;
    Ok(output.trim().to_string())
}

fn get_root_disk_available_bytes(target: &str) -> Result<u64> {
    let output = run_ssh_command_named(target, "df -Pk /", "Root disk measurement")?;
    parse_root_disk_available_bytes(&output)
}

fn parse_root_disk_available_bytes(output: &str) -> Result<u64> {
    let row = output
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .ok_or_else(|| anyhow!("root disk measurement returned no rows"))?;
    let available_kib = row
        .split_whitespace()
        .nth(3)
        .ok_or_else(|| anyhow!("root disk measurement returned an unexpected row"))?
        .parse::<u64>()
        .context("parsing root disk available blocks")?;
    available_kib
        .checked_mul(KIBIBYTE)
        .ok_or_else(|| anyhow!("root disk available-byte count overflowed"))
}

fn os_update_rollback_cmd() -> Option<String> {
    let raw = std::env::var("DSC_SSH_OS_UPDATE_ROLLBACK_CMD").unwrap_or_default();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn build_changelog_payload(metadata: &UpdateMetadata) -> String {
    let before_version = metadata.before_version.as_deref().unwrap_or("unknown");
    let after_version = metadata.after_version.as_deref().unwrap_or("unknown");
    let reclaimed = metadata.reclaimed_space.as_deref().unwrap_or("unknown");
    let os_version = metadata.before_os_version.as_deref().unwrap_or("unknown");
    let root_disk = metadata.root_disk_usage.as_deref().unwrap_or("unknown");
    let before_commit = format_commit_link(metadata.before_commit.as_deref());
    let after_commit = format_commit_link(metadata.after_commit.as_deref());

    let mut body = Vec::new();
    if metadata.os_updated {
        body.push(format!("- [x] OS updated {}", os_version));
    } else {
        body.push(format!("- [ ] OS updated {}", os_version));
    }

    if metadata.server_rebooted {
        body.push("- [x] Server rebooted".to_string());
    }
    if let Some(recovery) = &metadata.preflight_disk_recovery {
        body.push(format!(
            "- [x] Preflight disk recovery: {} -> {} available",
            format_gib(recovery.initial_available_bytes),
            format_gib(recovery.final_available_bytes)
        ));
    }

    body.push("- [x] Updated Discourse:".to_string());
    if let Some(commit) = before_commit.as_deref() {
        body.push(format!(
            "  - Initial version: {} {}",
            before_version, commit
        ));
    } else {
        body.push(format!("  - Initial version: {}", before_version));
    }
    let after_error = metadata
        .after_version_error
        .as_deref()
        .map(|err| format!(" (fetch failed: {})", err))
        .unwrap_or_default();
    if let Some(commit) = after_commit.as_deref() {
        body.push(format!(
            "  - Updated version: {}{} {}",
            after_version, after_error, commit
        ));
    } else {
        body.push(format!(
            "  - Updated version: {}{}",
            after_version, after_error
        ));
    }
    if reclaimed == "unknown" {
        body.push("- [x] Docker cleanup performed".to_string());
    } else {
        body.push(format!(
            "- [x] Docker cleanup total reclaimed space: {}",
            reclaimed
        ));
    }
    body.push(format!("- [x] Root disk usage (df -h /): {}", root_disk));
    let test_marker = std::env::var("DSC_TEST_MARKER").ok();
    if let Some(marker) = &test_marker {
        body.push(format!("- Run-ID: {}", marker));
    }
    body.join("\n")
}

fn fetch_version_info_with_retry(client: &DiscourseClient, attempts: usize) -> Result<VersionInfo> {
    let mut last_err = None;
    let total = attempts.max(1);
    for attempt in 0..total {
        match client.fetch_version_info() {
            Ok(info) => return Ok(info),
            Err(err) => {
                let message = err.to_string();
                last_err = Some(err);
                if attempt + 1 < total {
                    if message.contains("502") {
                        std::thread::sleep(std::time::Duration::from_secs(10));
                    } else {
                        std::thread::sleep(std::time::Duration::from_secs(
                            2 * (attempt + 1) as u64,
                        ));
                    }
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("fetch version failed")))
}

/// Fetch the latest commit SHA on the `stable` branch of discourse/discourse
/// from the GitHub API. Returns `None` on any failure (network, rate limit,
/// parse error) — callers treat that as "unknown, proceed with update".
fn fetch_latest_discourse_commit() -> Option<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let resp = client
        .get("https://api.github.com/repos/discourse/discourse/commits/stable")
        .header("Accept", "application/vnd.github.sha")
        .header("User-Agent", "dsc-cli")
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let sha = resp.text().ok()?.trim().to_string();
    if sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(sha)
    } else {
        None
    }
}

/// Returns `true` if the running Discourse commit matches the latest available
/// stable commit — meaning a rebuild would be a no-op.
fn is_discourse_up_to_date(running_commit: Option<&str>) -> bool {
    let Some(running) = running_commit else {
        return false;
    };
    let running = running.trim();
    if running.is_empty() {
        return false;
    }
    let Some(latest) = fetch_latest_discourse_commit() else {
        return false;
    };
    // Compare by the shorter of the two — the running commit from the
    // HTML meta tag is often truncated to 10 characters.
    let cmp_len = running.len().min(latest.len());
    running[..cmp_len].eq_ignore_ascii_case(&latest[..cmp_len])
}

fn format_commit_link(commit: Option<&str>) -> Option<String> {
    let commit = commit?;
    let trimmed = commit.trim();
    if trimmed.is_empty() {
        return None;
    }
    let short = trimmed.chars().take(10).collect::<String>();
    Some(format!(
        "[{}](https://github.com/discourse/discourse/commits/{})",
        short, trimmed
    ))
}

fn strip_ansi_codes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn post_changelog_update(discourse: &DiscourseConfig, payload: &str) -> Result<u64> {
    let topic_id = discourse.changelog_topic_id.ok_or_else(|| {
        missing_config(
            "changelog_topic_id",
            &format!("discourse {}", discourse.name),
            "changelog_topic_id",
        )
    })?;
    let client = DiscourseClient::new(discourse)?;
    let post_id = client.create_post(topic_id, payload)?;
    if std::env::var("DSC_TEST_MARKER").is_ok() {
        println!("DSC_TEST_POST_ID={}", post_id);
    }
    Ok(post_id)
}

fn confirm_changelog_post(yes: bool) -> Result<bool> {
    if yes {
        println!("Post this to changelog? [y/N]: y (--yes)");
        return Ok(true);
    }
    if std::env::var("DSC_TEST_MARKER").is_ok() {
        println!("Post this to changelog? [y/N]: y (auto)");
        return Ok(true);
    }
    print!("Post this to changelog? [y/N]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim(), "y" | "Y" | "yes" | "YES"))
}

fn handle_changelog_post(discourse: &DiscourseConfig, payload: &str, yes: bool) -> Result<()> {
    let topic_id = discourse.changelog_topic_id;
    if topic_id.is_none() {
        println!(
            "Changelog post skipped: missing changelog_topic_id for {}",
            discourse.name
        );
        return Ok(());
    }

    if let Err(err) = ensure_api_credentials(discourse) {
        println!("Changelog post skipped: {}", err);
        return Ok(());
    }

    if !confirm_changelog_post(yes)? {
        println!("Changelog post skipped.");
        return Ok(());
    }

    match post_changelog_update(discourse, payload) {
        Ok(post_id) => {
            let base = discourse.baseurl.trim_end_matches('/');
            println!("Changelog post created: {}/p/{}", base, post_id);
            Ok(())
        }
        Err(err) => {
            println!("Changelog post failed: {}", err);
            Err(err)
        }
    }
}

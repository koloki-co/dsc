// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! `dsc install` — declarative provisioning on a `dsc harden`-prepared box:
//! clone `discourse_docker`, render `app.yml`, `launcher bootstrap && start`,
//! poll `/about.json`, and append a `[[discourse]]` entry to `dsc.toml`.
//!
//! **Phase 1 (this file):** the from-zero bootstrap itself, at the default
//! `containers/app.yml` path with the standard `standalone.yml` template.
//! Declarative diff/push of an app.yml against an *already-running* install
//! is a different job — see `dsc app` and `dsc update`. Not yet implemented:
//! `--image` (base image override) and `--bootstrap-admin` (see
//! `spec/commands/install.md`).

use crate::commands::common::{oneline_for_dry_run, shell_quote, validate_ssh_target};
use crate::commands::ssh::build_ssh_command;
use crate::config::{Config, DiscourseConfig, save_config};
use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, IsTerminal};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const REPO_URL: &str = "https://github.com/discourse/discourse_docker.git";
const CONTAINER_DIR: &str = "/var/discourse";
const APP_YML_PATH: &str = "/var/discourse/containers/app.yml";
const ABOUT_JSON_TIMEOUT: Duration = Duration::from_secs(300);
const ABOUT_JSON_POLL_INTERVAL: Duration = Duration::from_secs(5);
const MAX_REMOTE_DIAGNOSTIC_LINES: usize = 20;

/// Resolved options for one `dsc install` run. Built in `main.rs` from CLI
/// flags (including reading `--smtp-pass-stdin` before construction, since
/// that's an interactive/piped read, not a pure argument).
pub struct InstallOptions {
    pub name: String,
    pub host: String,
    pub ssh_user: String,
    pub ssh_port: u16,
    pub emails: Vec<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_user: Option<String>,
    pub smtp_pass: Option<String>,
    pub branch: Option<String>,
}

pub fn install(
    config: &mut Config,
    config_path: &Path,
    opts: &InstallOptions,
    dry_run: bool,
) -> Result<()> {
    if config.discourse.iter().any(|d| d.name == opts.name) {
        return Err(anyhow!(
            "a discourse named '{}' already exists in {}",
            opts.name,
            config_path.display()
        ));
    }
    if opts.emails.is_empty() {
        return Err(anyhow!(
            "at least one --email is required (used for DISCOURSE_DEVELOPER_EMAILS)"
        ));
    }
    validate_ssh_target(&opts.ssh_user).context("invalid --ssh-user")?;
    validate_ssh_target(&opts.host).context("invalid --host")?;

    let target = format!("{}@{}", opts.ssh_user, opts.host);
    let port_arg;
    let extra: &[&str] = if opts.ssh_port == 22 {
        &[]
    } else {
        port_arg = opts.ssh_port.to_string();
        &["-p", &port_arg]
    };

    announce(&format!(
        "Installing '{}' on {} ({})",
        opts.name, opts.host, target
    ));

    // --- Preflight ---
    let mem_kb = ssh_text(
        &target,
        extra,
        "awk '/^MemTotal:/ {print $2}' /proc/meminfo",
        dry_run,
    )?;
    assert_enough_memory(&mem_kb, dry_run)?;

    let disk_gb = ssh_text(
        &target,
        extra,
        "df -B1G --output=avail /var | tail -n 1 | tr -d ' '",
        dry_run,
    )?;
    assert_enough_disk(&disk_gb, dry_run)?;

    // --- Step 1: discourse_docker checkout ---
    ensure_discourse_docker(&target, extra, dry_run)?;

    // --- Step 2: render + upload app.yml ---
    let app_yml = render_app_yml(opts);
    upload_app_yml(&target, extra, &app_yml, dry_run)?;

    // --- Step 3: bootstrap + start ---
    run_launcher(&target, extra, dry_run)?;

    // --- Step 4: poll for a live Discourse ---
    poll_about_json(&opts.host, dry_run)?;

    // --- Step 5: record the new forum in dsc.toml ---
    let entry = DiscourseConfig {
        name: opts.name.clone(),
        baseurl: format!("https://{}", opts.host),
        ssh_host: Some(opts.host.clone()),
        ssh_user: Some(opts.ssh_user.clone()),
        ssh_port: (opts.ssh_port != 22).then_some(opts.ssh_port as u64),
        ..DiscourseConfig::default()
    };
    if dry_run {
        announce(&format!(
            "[dry-run] would append [[discourse]] entry '{}' to {}",
            opts.name,
            config_path.display()
        ));
    } else {
        config.discourse.push(entry);
        save_config(config_path, config)?;
        announce(&format!(
            "✓ added '{}' to {}. apikey/api_username are still empty — create an API key on the new forum and run `dsc setting set {} apikey/api_username`, or edit the file directly.",
            opts.name,
            config_path.display(),
            opts.name,
        ));
    }

    Ok(())
}

/// Clone `discourse_docker` to `/var/discourse` if it isn't there yet,
/// otherwise fast-forward it. Idempotent, like every other install step —
/// re-running `dsc install` after a partial failure shouldn't redo work
/// that already succeeded.
///
/// Ownership: `/var/discourse` ends up owned by the SSH user, not root.
/// Phase 1 targets rootless Docker (the documented `dsc harden` stage-3
/// default), where `launcher` runs as that unprivileged user with no
/// `sudo` at all — so the checkout it operates on must be writable by
/// that user too. Creating the top-level directory is the one step that
/// still needs `sudo`, since `/var` itself isn't user-writable.
fn ensure_discourse_docker(target: &str, extra: &[&str], dry_run: bool) -> Result<()> {
    announce("checking for an existing /var/discourse checkout");
    let exists = ssh_text(
        target,
        extra,
        "test -d /var/discourse/.git && echo yes || echo no",
        dry_run,
    )?;
    if dry_run || exists.trim() == "no" {
        announce(
            "cloning discourse_docker to /var/discourse (owned by the SSH user, for rootless Docker)",
        );
        let cmd = format!(
            r#"
set -e
sudo -n mkdir -p {dir}
sudo -n chown "$(id -un)":"$(id -gn)" {dir}
git clone {repo} {dir}
"#,
            dir = shell_quote(CONTAINER_DIR),
            repo = shell_quote(REPO_URL),
        );
        ssh_text(target, extra, cmd.trim(), dry_run)?;
    } else {
        announce("/var/discourse already present, pulling latest discourse_docker");
        ssh_text(
            target,
            extra,
            &format!("cd {} && git pull", shell_quote(CONTAINER_DIR)),
            dry_run,
        )?;
    }
    Ok(())
}

/// Write the rendered `app.yml` to the remote host. Transported as base64
/// (same trick as `dsc harden`'s sshd drop-in) so arbitrary bytes in an SMTP
/// password or hostname can never be misparsed as shell syntax — the shell
/// only ever sees the base64 alphabet, and the YAML-level quoting in
/// [`render_app_yml`] is what protects the *rendered file's* structure.
/// No `sudo`: see [`ensure_discourse_docker`] on why `/var/discourse` is
/// user-owned for the rootless-Docker case this phase targets.
fn upload_app_yml(target: &str, extra: &[&str], app_yml: &str, dry_run: bool) -> Result<()> {
    announce(&format!("writing {}", APP_YML_PATH));
    let b64 = base64::engine::general_purpose::STANDARD.encode(app_yml.as_bytes());
    let cmd = format!(
        r#"
set -e
mkdir -p {dir}
tmp=$(mktemp)
printf '%s' {b64} | base64 -d > "$tmp"
install -m 0644 "$tmp" {path}
rm -f "$tmp"
"#,
        dir = shell_quote(
            Path::new(APP_YML_PATH)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or(CONTAINER_DIR)
        ),
        b64 = shell_quote(&b64),
        path = shell_quote(APP_YML_PATH),
    );
    ssh_text(target, extra, cmd.trim(), dry_run)?;
    Ok(())
}

/// Run `./launcher bootstrap app && ./launcher start app`, streaming
/// progress live (this step routinely takes 5-20 minutes) with a bounded
/// tail retained for the error message if it fails.
///
/// No `sudo`, and `DOCKER_HOST` is exported explicitly rather than relied
/// on from the user's shell rc: this targets rootless Docker (per `dsc
/// harden`'s stage-3 default, `docker_rootless = true`), where the
/// unprivileged user talks to their own per-user daemon socket directly —
/// running `launcher` as root instead would talk to a root-owned rootful
/// daemon that doesn't exist in this setup. A non-interactive SSH command
/// doesn't source `~/.bashrc` (only interactive shells do), which is where
/// the rootless install script normally adds `DOCKER_HOST` — so it has to
/// be set inline here instead of assumed from the environment. Phase 1
/// doesn't support a rootful-Docker target; see `docs/install.md`.
fn run_launcher(target: &str, extra: &[&str], dry_run: bool) -> Result<()> {
    let cmd_str = format!(
        "export DOCKER_HOST=unix:///run/user/$(id -u)/docker.sock && cd {dir} && ./launcher bootstrap app && ./launcher start app",
        dir = shell_quote(CONTAINER_DIR),
    );
    if dry_run {
        announce(&format!(
            "[dry-run] would run on {}{}: {}",
            ssh_extra_display(extra),
            target,
            oneline_for_dry_run(&cmd_str)
        ));
        return Ok(());
    }
    let mut cmd = build_ssh_command(target, extra)?;
    cmd.arg(&cmd_str);
    run_streamed(cmd, "launcher bootstrap + start")
}

/// Poll `http://<host>/about.json` until it responds successfully. Plain
/// HTTP, not HTTPS: a freshly-bootstrapped `standalone.yml` has no SSL
/// template enabled (that's a separate, later step involving DNS + Let's
/// Encrypt), so port 80 is the only thing that can possibly answer yet.
fn poll_about_json(host: &str, dry_run: bool) -> Result<()> {
    let url = format!("http://{}/about.json", host);
    if dry_run {
        announce(&format!(
            "[dry-run] would poll {} until it responds successfully (up to {}s)",
            url,
            ABOUT_JSON_TIMEOUT.as_secs()
        ));
        return Ok(());
    }
    announce(&format!(
        "polling {} until Discourse answers (this can take a few minutes after `launcher start` returns)…",
        url
    ));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("building HTTP client")?;
    let deadline = Instant::now() + ABOUT_JSON_TIMEOUT;
    let mut last = String::from("no response yet");
    while Instant::now() < deadline {
        match client.get(&url).send() {
            Ok(resp) if resp.status().is_success() => {
                announce("✓ Discourse is responding");
                return Ok(());
            }
            Ok(resp) => last = format!("HTTP {}", resp.status()),
            Err(e) => last = e.to_string(),
        }
        thread::sleep(ABOUT_JSON_POLL_INTERVAL);
    }
    Err(anyhow!(
        "{} did not return a successful response within {}s (last: {}). \
         The container may still be starting — check `sudo docker logs app` on the host, \
         or retry `curl -sv {}` manually.",
        url,
        ABOUT_JSON_TIMEOUT.as_secs(),
        last,
        url,
    ))
}

// --- app.yml templating ---

/// Base template for `containers/app.yml`, modelled directly on
/// `discourse_docker`'s own `samples/standalone.yml`. Two markers are
/// substituted by [`render_app_yml`]; everything else is the standard
/// single-container stack (Postgres + Redis + rate-limited web, ports 80
/// and 443 exposed, `docker_manager` pre-installed so plugins can be
/// managed from the admin UI afterwards).
const APP_YML_TEMPLATE: &str = r#"## Generated by `dsc install`. Based on discourse_docker's own
## samples/standalone.yml — see that file for the full annotated
## reference. From here on, manage this file with `dsc app`/`dsc update`,
## or edit by hand and re-run `./launcher rebuild app`.
##
## YAML is sensitive to whitespace and indentation — be careful editing.

templates:
  - "templates/postgres.template.yml"
  - "templates/redis.template.yml"
  - "templates/web.template.yml"
  - "templates/web.ratelimited.template.yml"

expose:
  - "80:80"
  - "443:443"

params:
  db_default_text_search_config: "pg_catalog.english"__PARAMS_VERSION__

env:
  LC_ALL: en_US.UTF-8
  LANG: en_US.UTF-8
  LANGUAGE: en_US.UTF-8
__ENV_LINES__
volumes:
  - volume:
      host: /var/discourse/shared/standalone
      guest: /shared
  - volume:
      host: /var/discourse/shared/standalone/log/var-log
      guest: /var/log

hooks:
  after_code:
    - exec:
        cd: $home/plugins
        cmd:
          - git clone https://github.com/discourse/docker_manager.git

run:
  - exec: echo "Provisioned by dsc install"
"#;

fn render_app_yml(opts: &InstallOptions) -> String {
    let mut env_lines = String::new();
    env_lines.push_str(&format!(
        "  DISCOURSE_HOSTNAME: {}\n",
        yaml_single_quote(&opts.host)
    ));
    env_lines.push_str(&format!(
        "  DISCOURSE_DEVELOPER_EMAILS: {}\n",
        yaml_single_quote(&opts.emails.join(","))
    ));
    if let Some(smtp_host) = &opts.smtp_host {
        env_lines.push_str(&format!(
            "  DISCOURSE_SMTP_ADDRESS: {}\n",
            yaml_single_quote(smtp_host)
        ));
        if let Some(port) = opts.smtp_port {
            env_lines.push_str(&format!("  DISCOURSE_SMTP_PORT: {port}\n"));
        }
        if let Some(user) = &opts.smtp_user {
            env_lines.push_str(&format!(
                "  DISCOURSE_SMTP_USER_NAME: {}\n",
                yaml_single_quote(user)
            ));
        }
        if let Some(pass) = &opts.smtp_pass {
            env_lines.push_str(&format!(
                "  DISCOURSE_SMTP_PASSWORD: {}\n",
                yaml_single_quote(pass)
            ));
        }
    } else {
        env_lines.push_str(
            "  # No --smtp-host given: Discourse cannot send mail until SMTP is configured.\n",
        );
    }

    let params_version = opts
        .branch
        .as_deref()
        .map(|b| format!("\n  version: {}", yaml_single_quote(b)))
        .unwrap_or_default();

    APP_YML_TEMPLATE
        .replace("__PARAMS_VERSION__", &params_version)
        .replace("__ENV_LINES__", env_lines.trim_end_matches('\n'))
}

/// Quote a value as a YAML single-quoted scalar: wrap in `'...'` and double
/// any embedded single quote. Safe for arbitrary bytes (hostnames, emails,
/// SMTP passwords) without needing to know what characters YAML treats
/// specially outside quotes.
fn yaml_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

// --- preflight ---

fn assert_enough_memory(mem_kb_raw: &str, dry_run: bool) -> Result<()> {
    if dry_run && mem_kb_raw.is_empty() {
        return Ok(());
    }
    let kb: u64 = mem_kb_raw
        .trim()
        .parse()
        .with_context(|| format!("parsing MemTotal from {:?}", mem_kb_raw))?;
    let mb = kb / 1024;
    if mb < 1000 {
        return Err(anyhow!(
            "remote host has only {} MB RAM — Discourse's hard minimum is 1024 MB. Bail out.",
            mb
        ));
    }
    if mb < 2048 {
        eprintln!(
            "[install] warning: only {} MB RAM detected. `launcher bootstrap` compiles assets and can OOM below 2 GB without swap.",
            mb
        );
    } else {
        announce(&format!("memory OK ({} MB)", mb));
    }
    Ok(())
}

fn assert_enough_disk(gb_raw: &str, dry_run: bool) -> Result<()> {
    if dry_run && gb_raw.is_empty() {
        return Ok(());
    }
    let gb: u64 = gb_raw
        .trim()
        .parse()
        .with_context(|| format!("parsing free-GB from {:?}", gb_raw))?;
    if gb < 5 {
        return Err(anyhow!(
            "only {} GB free on /var — `launcher bootstrap` needs room to build and land a fresh image. Bail out and get a bigger disk.",
            gb
        ));
    }
    if gb < 30 {
        eprintln!(
            "[install] warning: only {} GB free on /var. Fine for the initial bootstrap, but rebuilds get tight below ~30 GB.",
            gb
        );
    } else {
        announce(&format!("disk OK ({} GB free on /var)", gb));
    }
    Ok(())
}

// --- SSH plumbing ---

fn announce(msg: &str) {
    eprintln!("[install] {}", msg);
}

/// Run a short remote command and capture its stdout as text. For quick
/// preflight/idempotency probes; long-running steps use [`run_streamed`]
/// instead so their output isn't silently swallowed until the end.
/// `-p 2227 ` (trailing space) when `extra` carries a non-default port,
/// empty otherwise — for splicing into dry-run messages that otherwise
/// read `ssh <target>` and would silently hide a non-default port.
fn ssh_extra_display(extra: &[&str]) -> String {
    if extra.is_empty() {
        String::new()
    } else {
        format!("{} ", extra.join(" "))
    }
}

fn ssh_text(target: &str, extra: &[&str], command: &str, dry_run: bool) -> Result<String> {
    if dry_run {
        eprintln!(
            "[dry-run] ssh {}{} -- {}",
            ssh_extra_display(extra),
            target,
            oneline_for_dry_run(command)
        );
        return Ok(String::new());
    }
    let mut cmd = build_ssh_command(target, extra)?;
    let output = cmd
        .arg(command)
        .output()
        .with_context(|| format!("running ssh to {}", target))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "ssh to {} failed ({}): {}",
            target,
            output.status,
            stderr.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

struct LineEvent {
    is_stderr: bool,
    line: String,
}

/// Run a (possibly long-lived) spawned command, showing a live spinner with
/// the latest output line while it runs, and surfacing a bounded tail of
/// stdout/stderr in the error message if it fails. Modelled on `dsc
/// update`'s rebuild runner — a `launcher bootstrap` can take 5-20 minutes
/// and produce thousands of lines, so neither silently waiting nor
/// buffering it all in memory is acceptable.
fn run_streamed(mut command: Command, step: &str) -> Result<()> {
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
        .with_context(|| format!("running {step}"))?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;

    let (tx, rx) = mpsc::sync_channel::<LineEvent>(64);
    let tx_out = tx.clone();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx_out
                .send(LineEvent {
                    is_stderr: false,
                    line,
                })
                .is_err()
            {
                break;
            }
        }
    });
    let tx_err = tx.clone();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tx_err
                .send(LineEvent {
                    is_stderr: true,
                    line,
                })
                .is_err()
            {
                break;
            }
        }
    });
    drop(tx);

    let mut stdout_ring: VecDeque<String> = VecDeque::with_capacity(MAX_REMOTE_DIAGNOSTIC_LINES);
    let mut stderr_ring: VecDeque<String> = VecDeque::with_capacity(MAX_REMOTE_DIAGNOSTIC_LINES);
    pb.set_message(step.to_string());

    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(event) => {
                let ring = if event.is_stderr {
                    &mut stderr_ring
                } else {
                    &mut stdout_ring
                };
                if ring.len() >= MAX_REMOTE_DIAGNOSTIC_LINES {
                    ring.pop_front();
                }
                if use_progress {
                    pb.set_message(format!("{step}\n  {}", event.line));
                }
                ring.push_back(event.line);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let status = child.wait().context("waiting for command")?;
    pb.finish_and_clear();

    if !status.success() {
        let mut message = format!("{step} failed ({status})");
        if !stdout_ring.is_empty() {
            message.push_str(&format!(
                "\n--- stdout (last {} lines) ---\n{}",
                stdout_ring.len(),
                stdout_ring.iter().cloned().collect::<Vec<_>>().join("\n")
            ));
        }
        if !stderr_ring.is_empty() {
            message.push_str(&format!(
                "\n--- stderr (last {} lines) ---\n{}",
                stderr_ring.len(),
                stderr_ring.iter().cloned().collect::<Vec<_>>().join("\n")
            ));
        }
        return Err(anyhow!(message));
    }
    Ok(())
}

/// Read an SMTP password from stdin (one line, trailing newline stripped).
/// Kept out of argv so it never appears in `ps` output or shell history.
pub fn read_smtp_pass_from_stdin() -> Result<String> {
    let mut buf = String::new();
    io::stdin()
        .read_line(&mut buf)
        .context("reading SMTP password from stdin")?;
    let pass = buf.trim_end_matches(['\n', '\r']).to_string();
    if pass.is_empty() {
        return Err(anyhow!("no SMTP password read from stdin"));
    }
    Ok(pass)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_opts() -> InstallOptions {
        InstallOptions {
            name: "numun".to_string(),
            host: "communities.numun.fund".to_string(),
            ssh_user: "discourse".to_string(),
            ssh_port: 2227,
            emails: vec!["marcus@koloki.co".to_string()],
            smtp_host: None,
            smtp_port: None,
            smtp_user: None,
            smtp_pass: None,
            branch: None,
        }
    }

    #[test]
    fn ssh_extra_display_is_empty_for_default_port() {
        assert_eq!(ssh_extra_display(&[]), "");
    }

    #[test]
    fn ssh_extra_display_shows_a_non_default_port() {
        assert_eq!(ssh_extra_display(&["-p", "2227"]), "-p 2227 ");
    }

    #[test]
    fn yaml_single_quote_escapes_embedded_quotes() {
        assert_eq!(yaml_single_quote("plain"), "'plain'");
        assert_eq!(yaml_single_quote("it's"), "'it''s'");
        assert_eq!(
            yaml_single_quote("pa$$w'ord`;rm -rf /"),
            "'pa$$w''ord`;rm -rf /'"
        );
    }

    #[test]
    fn rendered_app_yml_is_valid_yaml() {
        let opts = base_opts();
        let rendered = render_app_yml(&opts);
        let value: serde_yaml::Value =
            serde_yaml::from_str(&rendered).expect("rendered app.yml must parse as YAML");
        assert!(value.get("templates").is_some());
        assert!(value.get("env").is_some());
    }

    #[test]
    fn rendered_app_yml_contains_hostname_and_emails() {
        let opts = base_opts();
        let rendered = render_app_yml(&opts);
        assert!(rendered.contains("DISCOURSE_HOSTNAME: 'communities.numun.fund'"));
        assert!(rendered.contains("DISCOURSE_DEVELOPER_EMAILS: 'marcus@koloki.co'"));
    }

    #[test]
    fn rendered_app_yml_joins_multiple_emails_with_commas() {
        let mut opts = base_opts();
        opts.emails = vec!["a@example.com".to_string(), "b@example.com".to_string()];
        let rendered = render_app_yml(&opts);
        assert!(rendered.contains("DISCOURSE_DEVELOPER_EMAILS: 'a@example.com,b@example.com'"));
    }

    #[test]
    fn rendered_app_yml_omits_smtp_block_when_unset() {
        let opts = base_opts();
        let rendered = render_app_yml(&opts);
        assert!(!rendered.contains("DISCOURSE_SMTP_ADDRESS"));
        assert!(rendered.contains("No --smtp-host given"));
    }

    #[test]
    fn rendered_app_yml_includes_full_smtp_block_when_set() {
        let mut opts = base_opts();
        opts.smtp_host = Some("smtp.example.com".to_string());
        opts.smtp_port = Some(587);
        opts.smtp_user = Some("user@example.com".to_string());
        opts.smtp_pass = Some("pa$$word".to_string());
        let rendered = render_app_yml(&opts);
        assert!(rendered.contains("DISCOURSE_SMTP_ADDRESS: 'smtp.example.com'"));
        assert!(rendered.contains("DISCOURSE_SMTP_PORT: 587"));
        assert!(rendered.contains("DISCOURSE_SMTP_USER_NAME: 'user@example.com'"));
        assert!(rendered.contains("DISCOURSE_SMTP_PASSWORD: 'pa$$word'"));
        assert!(!rendered.contains("No --smtp-host given"));

        // Still valid YAML with the full block present, including a
        // dollar-sign-bearing password that could confuse a naive parser.
        let value: serde_yaml::Value = serde_yaml::from_str(&rendered).unwrap();
        assert_eq!(
            value["env"]["DISCOURSE_SMTP_PASSWORD"].as_str(),
            Some("pa$$word")
        );
    }

    #[test]
    fn rendered_app_yml_omits_version_param_when_branch_unset() {
        let opts = base_opts();
        let rendered = render_app_yml(&opts);
        assert!(!rendered.contains("version:"));
    }

    #[test]
    fn rendered_app_yml_sets_version_param_when_branch_given() {
        let mut opts = base_opts();
        opts.branch = Some("stable".to_string());
        let rendered = render_app_yml(&opts);
        assert!(rendered.contains("version: 'stable'"));
        let value: serde_yaml::Value = serde_yaml::from_str(&rendered).unwrap();
        assert_eq!(value["params"]["version"].as_str(), Some("stable"));
    }

    #[test]
    fn rendered_app_yml_survives_single_quote_injection_attempt() {
        let mut opts = base_opts();
        opts.host = "evil'; env: {}\nfoo".to_string();
        let rendered = render_app_yml(&opts);
        // The attempted structural break is neutralised by YAML single-quote
        // escaping, so the whole thing still parses as one scalar value.
        let value: serde_yaml::Value = serde_yaml::from_str(&rendered)
            .expect("a single-quote injection attempt must not break the YAML structure");
        assert!(
            value["env"]["DISCOURSE_HOSTNAME"]
                .as_str()
                .unwrap()
                .starts_with("evil'")
        );
    }

    #[test]
    fn memory_bail_below_1024() {
        assert!(assert_enough_memory("800000", false).is_err());
    }

    #[test]
    fn memory_ok_at_2048() {
        assert!(assert_enough_memory("2097152", false).is_ok());
    }

    #[test]
    fn disk_bail_below_5gb() {
        assert!(assert_enough_disk("3", false).is_err());
    }

    #[test]
    fn disk_happy_at_40gb() {
        assert!(assert_enough_disk("40", false).is_ok());
    }

    #[test]
    fn refuses_duplicate_discourse_name() {
        let mut config = Config {
            discourse: vec![DiscourseConfig {
                name: "numun".to_string(),
                baseurl: "https://communities.numun.fund".to_string(),
                ..DiscourseConfig::default()
            }],
            ..Config::default()
        };
        let opts = base_opts();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dsc.toml");
        let err = install(&mut config, &path, &opts, true).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn refuses_when_no_email_given() {
        let mut config = Config::default();
        let mut opts = base_opts();
        opts.emails.clear();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dsc.toml");
        let err = install(&mut config, &path, &opts, true).unwrap_err();
        assert!(err.to_string().contains("--email"));
    }

    #[test]
    fn dry_run_does_not_write_config_file() {
        let mut config = Config::default();
        let opts = base_opts();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dsc.toml");
        // Dry-run's SSH probes return empty strings (see `ssh_text`), which
        // the preflight asserts tolerate; the poll step is also a no-op
        // under dry-run, so the whole plan can run with no real network
        // access at all.
        install(&mut config, &path, &opts, true).unwrap();
        assert!(!path.exists(), "dry-run must not write dsc.toml");
        assert!(
            config.discourse.is_empty(),
            "dry-run must not mutate the in-memory config either"
        );
    }
}

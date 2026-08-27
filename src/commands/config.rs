// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::config::{Config, DiscourseConfig};
use anyhow::{Result, anyhow};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::process::Stdio;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

/// Default concurrent workers for `config check --parallel`. Higher than the
/// update default (3) because these probes are short and I/O-bound.
const DEFAULT_PARALLEL_CHECK_WORKERS: usize = 8;

#[derive(Serialize)]
struct CheckReport {
    name: String,
    baseurl: String,
    api: CheckStatus,
    ssh: Option<CheckStatus>,
}

#[derive(Serialize)]
struct CheckStatus {
    ok: bool,
    detail: String,
}

pub fn config_check(
    config: &Config,
    format: ListFormat,
    skip_ssh: bool,
    parallel: bool,
    max: Option<usize>,
) -> Result<()> {
    if config.discourse.is_empty() {
        return Err(anyhow!("no discourses configured"));
    }

    let text = matches!(format, ListFormat::Text);
    // Column width known up front (from the config) so streamed rows align.
    let name_width = config
        .discourse
        .iter()
        .map(|d| d.name.len())
        .max()
        .unwrap_or(0)
        .max(4);

    // Signpost on stderr: this contacts every forum over the network (and SSH),
    // so it can take a while. stderr keeps json/yaml stdout clean.
    let workers = if parallel {
        check_worker_count(max, config.discourse.len())
    } else {
        1
    };
    eprintln!(
        "Checking {} discourse(s) via API{}{}... contacts each over the network and can take a while.",
        config.discourse.len(),
        if skip_ssh { "" } else { " + SSH" },
        if workers > 1 {
            format!(" ({workers} parallel)")
        } else {
            String::new()
        }
    );

    // Print each report as it lands (text mode). Returns the collected set.
    let on_done = |report: &CheckReport| {
        if text {
            print_report_text(report, name_width);
            let _ = std::io::stdout().flush();
        }
    };
    let mut reports = if parallel && workers > 1 {
        check_parallel(&config.discourse, skip_ssh, workers, on_done)
    } else {
        let mut out = Vec::with_capacity(config.discourse.len());
        for discourse in &config.discourse {
            let report = check_one(discourse, skip_ssh);
            on_done(&report);
            out.push(report);
        }
        out
    };

    // Parallel results arrive fastest-first; restore config order for the
    // (buffered) json/yaml output so it's deterministic.
    if !text {
        let order: HashMap<&str, usize> = config
            .discourse
            .iter()
            .enumerate()
            .map(|(i, d)| (d.name.as_str(), i))
            .collect();
        reports.sort_by_key(|r| order.get(r.name.as_str()).copied().unwrap_or(usize::MAX));
    }

    let failed = reports
        .iter()
        .filter(|r| !(r.api.ok && r.ssh.as_ref().map(|s| s.ok).unwrap_or(true)))
        .count();

    match format {
        ListFormat::Text => eprintln!(
            "Done: {} ok, {} failed (of {}).",
            reports.len() - failed,
            failed,
            reports.len()
        ),
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&reports)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&reports)?),
    }

    if failed == 0 {
        Ok(())
    } else {
        Err(anyhow!("{failed} discourse(s) failed checks"))
    }
}

/// Run the API (+ optional SSH) probes for one forum.
fn check_one(discourse: &DiscourseConfig, skip_ssh: bool) -> CheckReport {
    let api = check_api(discourse);
    let ssh = if skip_ssh {
        None
    } else {
        discourse
            .ssh_host
            .as_deref()
            .filter(|h| !h.trim().is_empty())
            .map(check_ssh)
    };
    CheckReport {
        name: discourse.name.clone(),
        baseurl: discourse.baseurl.clone(),
        api,
        ssh,
    }
}

/// Probe forums across a fixed worker pool, invoking `on_done` (on this thread)
/// for each result as it completes - fastest-first - and returning the full
/// set. Workers pull from a shared queue, so a slow forum never blocks others.
fn check_parallel(
    discourses: &[DiscourseConfig],
    skip_ssh: bool,
    workers: usize,
    mut on_done: impl FnMut(&CheckReport),
) -> Vec<CheckReport> {
    let queue: Arc<Mutex<VecDeque<DiscourseConfig>>> =
        Arc::new(Mutex::new(discourses.iter().cloned().collect()));
    let (tx, rx) = mpsc::channel::<CheckReport>();
    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let queue = Arc::clone(&queue);
        let tx = tx.clone();
        handles.push(thread::spawn(move || {
            loop {
                let next = queue.lock().unwrap().pop_front();
                let Some(discourse) = next else { break };
                if tx.send(check_one(&discourse, skip_ssh)).is_err() {
                    break;
                }
            }
        }));
    }
    drop(tx); // rx closes once every worker's sender is dropped

    let mut reports = Vec::with_capacity(discourses.len());
    for report in rx {
        on_done(&report);
        reports.push(report);
    }
    for handle in handles {
        let _ = handle.join();
    }
    reports
}

/// Workers for `--parallel`: the requested max (default 8), never more than
/// the number of forums.
fn check_worker_count(max: Option<usize>, count: usize) -> usize {
    max.unwrap_or(DEFAULT_PARALLEL_CHECK_WORKERS)
        .max(1)
        .min(count.max(1))
}

/// True when the configured baseurl would put the API key on the wire in
/// cleartext. Loopback is exempt: a local dev forum on plain HTTP is normal
/// and warning about it would just train people to ignore the warning.
fn sends_key_in_cleartext(baseurl: &str) -> bool {
    // URL schemes are case-insensitive, so normalise before matching.
    let trimmed = baseurl.trim();
    let lowered = trimmed.to_ascii_lowercase();
    let Some(rest) = lowered.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split(['/', ':', '?', '#']).next().unwrap_or("");
    !matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

fn check_api(discourse: &DiscourseConfig) -> CheckStatus {
    if discourse.baseurl.trim().is_empty() {
        return CheckStatus {
            ok: false,
            detail: "missing baseurl".to_string(),
        };
    }
    let client = match DiscourseClient::new(discourse) {
        Ok(client) => client,
        Err(err) => {
            return CheckStatus {
                ok: false,
                detail: format!("client init failed: {err}"),
            };
        }
    };
    match client.get("/about.json") {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                CheckStatus {
                    ok: true,
                    detail: if sends_key_in_cleartext(&discourse.baseurl) {
                        format!(
                            "{} OK - warning: baseurl is http://, so the API key is sent in cleartext",
                            status.as_u16()
                        )
                    } else {
                        format!("{} OK", status.as_u16())
                    },
                }
            } else if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                CheckStatus {
                    ok: false,
                    detail: format!("{} — check apikey/api_username", status.as_u16()),
                }
            } else {
                CheckStatus {
                    ok: false,
                    detail: format!("HTTP {}", status.as_u16()),
                }
            }
        }
        Err(err) => CheckStatus {
            ok: false,
            detail: format!("request failed: {err}"),
        },
    }
}

fn check_ssh(host: &str) -> CheckStatus {
    let mut command = match crate::commands::ssh::build_ssh_command_with_timeout(host, 5, &[]) {
        Ok(command) => command,
        Err(err) => {
            return CheckStatus {
                ok: false,
                detail: err.to_string(),
            };
        }
    };
    let output = command
        .arg("true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(out) if out.status.success() => CheckStatus {
            ok: true,
            detail: format!("ssh {} OK", host),
        },
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let first = stderr.lines().next().unwrap_or("").trim();
            CheckStatus {
                ok: false,
                detail: if first.is_empty() {
                    format!("ssh failed (exit {})", out.status)
                } else {
                    format!("ssh: {}", first)
                },
            }
        }
        Err(err) => CheckStatus {
            ok: false,
            detail: format!("ssh spawn failed: {err}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{check_api, check_worker_count, sends_key_in_cleartext};
    use crate::config::DiscourseConfig;

    #[test]
    fn flags_plaintext_http_baseurls() {
        assert!(sends_key_in_cleartext("http://forum.example.com"));
        assert!(sends_key_in_cleartext("HTTP://Forum.Example.Com/"));
        assert!(sends_key_in_cleartext("http://forum.example.com:8080/x"));
    }

    #[test]
    fn accepts_https_and_loopback_without_warning() {
        assert!(!sends_key_in_cleartext("https://forum.example.com"));
        assert!(!sends_key_in_cleartext("http://localhost:3000"));
        assert!(!sends_key_in_cleartext("http://127.0.0.1:4200/"));
    }

    #[test]
    fn default_check_workers_is_eight() {
        assert_eq!(check_worker_count(None, 20), 8);
    }

    #[test]
    fn check_workers_capped_by_forum_count() {
        assert_eq!(check_worker_count(Some(8), 3), 3);
        assert_eq!(check_worker_count(None, 2), 2);
    }

    #[test]
    fn check_workers_floor_of_one() {
        assert_eq!(check_worker_count(Some(0), 5), 1);
    }

    #[test]
    fn check_api_flags_empty_baseurl() {
        let discourse = DiscourseConfig {
            name: "empty".to_string(),
            baseurl: String::new(),
            ..DiscourseConfig::default()
        };
        let status = check_api(&discourse);
        assert!(!status.ok);
        assert!(
            status.detail.contains("missing baseurl"),
            "expected missing-baseurl detail, got {:?}",
            status.detail
        );
    }

    #[test]
    fn check_api_flags_whitespace_baseurl() {
        let discourse = DiscourseConfig {
            name: "ws".to_string(),
            baseurl: "   ".to_string(),
            ..DiscourseConfig::default()
        };
        let status = check_api(&discourse);
        assert!(!status.ok);
        assert!(status.detail.contains("missing baseurl"));
    }
}

fn print_report_text(r: &CheckReport, name_width: usize) {
    let api_mark = if r.api.ok { "ok " } else { "FAIL" };
    println!(
        "{:<width$}  api  {:<4}  {}",
        r.name,
        api_mark,
        r.api.detail,
        width = name_width
    );
    if let Some(ssh) = &r.ssh {
        let mark = if ssh.ok { "ok " } else { "FAIL" };
        println!(
            "{:<width$}  ssh  {:<4}  {}",
            r.name,
            mark,
            ssh.detail,
            width = name_width
        );
    }
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::config::{Config, DiscourseConfig, find_discourse};
use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::fmt::Display;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Absolute ceiling on fleet parallelism. Above this, an explicit
/// `--max` override is required. Each worker may hold an SSH process
/// and reader threads, so unbounded widths can exhaust local FDs/CPU.
const MAX_FLEET_WORKERS: usize = 32;

/// Emit a single command result honouring `--format`. Text mode prints the
/// human-readable `text`; json/yaml serialise `value`. Lets the otherwise
/// single-value commands (`setting get`, `theme duplicate`, `topic reply`/
/// `new`, …) participate in scripting pipelines without bespoke per-command
/// formatting code.
pub fn emit_result<T: Serialize>(format: ListFormat, value: &T, text: &str) -> Result<()> {
    match format {
        ListFormat::Text => println!("{}", text),
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(value)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(value)?),
    }
    Ok(())
}

pub fn select_discourse<'a>(
    config: &'a Config,
    discourse_name: Option<&str>,
) -> Result<&'a DiscourseConfig> {
    if let Some(name) = discourse_name {
        return find_discourse(config, name).ok_or_else(|| not_found("discourse", name));
    }
    Err(anyhow!("missing discourse for command"))
}

/// Shared fleet selector for `--all`/`--tags`-style fan-out commands
/// (`backup create --all`, `backup setup-s3 --tags`, `backup health`,
/// `search all`, `user find`). A single `discourse_name` selects exactly
/// that forum; otherwise `tags` (comma/semicolon separated, match-any) is
/// applied against every configured forum, or every forum is returned when
/// `tags` is `None`. Rejects `--tags` values that parse to no tags at all
/// (e.g. `--tags ""` or `--tags ",;"`), since silently falling back to "no
/// filter" there is a footgun that would fan a mutating command out to the
/// entire fleet when the caller meant to scope it down.
pub fn selected_discourses<'a>(
    config: &'a Config,
    discourse_name: Option<&str>,
    tags: Option<&str>,
) -> Result<Vec<&'a DiscourseConfig>> {
    if let Some(name) = discourse_name {
        return find_discourse(config, name)
            .map(|discourse| vec![discourse])
            .ok_or_else(|| not_found("discourse", name));
    }
    let filter = match tags {
        Some(raw) => {
            let parsed = parse_tags(raw);
            if parsed.is_empty() {
                bail!("--tags must include at least one non-empty tag");
            }
            parsed
        }
        None => Vec::new(),
    };
    Ok(config
        .discourse
        .iter()
        .filter(|discourse| matches_tag_filter(discourse, &filter))
        .collect())
}

fn matches_tag_filter(discourse: &DiscourseConfig, filter: &[String]) -> bool {
    if filter.is_empty() {
        return true;
    }
    let tags: HashSet<String> = discourse
        .tags
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|tag| tag.to_ascii_lowercase())
        .collect();
    filter
        .iter()
        .any(|tag| tags.contains(&tag.to_ascii_lowercase()))
}

/// Compute the worker count for a fleet operation: the requested value
/// (or `default`), floored at 1, capped at `count`, and never exceeding
/// [`MAX_FLEET_WORKERS`] unless `override_ceiling` is true.
pub fn fleet_worker_count(
    max: Option<usize>,
    count: usize,
    default: usize,
    override_ceiling: bool,
) -> usize {
    let requested = max.unwrap_or(default).max(1);
    let capped = requested.min(count.max(1));
    if override_ceiling {
        capped
    } else {
        capped.min(MAX_FLEET_WORKERS)
    }
}

/// Run a read-only operation across a fleet of Discourses with a bounded
/// worker pool, invoking `on_done` on this thread for each result as it
/// completes (fastest-first). Workers pull from a shared queue, so a slow
/// forum never blocks others.
///
/// `work` runs on a worker thread and must be `Send + Sync`; `on_done`
/// runs on the calling thread and can safely print. The returned `Vec`
/// is in completion order; callers that need config-file order should
/// sort by name afterwards.
pub fn run_fleet<T, F, G>(
    discourses: &[&DiscourseConfig],
    workers: usize,
    work: F,
    mut on_done: G,
) -> Vec<T>
where
    T: Send,
    F: Fn(&DiscourseConfig) -> T + Send + Sync,
    G: FnMut(&T),
{
    if workers <= 1 || discourses.len() <= 1 {
        return discourses
            .iter()
            .map(|d| {
                let result = work(d);
                on_done(&result);
                result
            })
            .collect();
    }

    let queue: Arc<Mutex<VecDeque<usize>>> = Arc::new(Mutex::new((0..discourses.len()).collect()));
    let work = Arc::new(work);
    let (tx, rx) = std::sync::mpsc::channel::<(usize, T)>();

    std::thread::scope(|s| {
        for _ in 0..workers {
            let queue = Arc::clone(&queue);
            let work = Arc::clone(&work);
            let tx = tx.clone();
            s.spawn(move || {
                loop {
                    let next = queue.lock().unwrap().pop_front();
                    let Some(idx) = next else { break };
                    let result = work(discourses[idx]);
                    if tx.send((idx, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        let mut results: Vec<(usize, T)> = Vec::with_capacity(discourses.len());
        for (idx, result) in rx {
            on_done(&result);
            results.push((idx, result));
        }
        // sort is safe: handles are joined by the scope when it returns
        results.sort_by_key(|(idx, _)| *idx);
        results.into_iter().map(|(_, r)| r).collect()
    })
}

pub fn ensure_api_credentials(discourse: &DiscourseConfig) -> Result<()> {
    let apikey = discourse.apikey.as_deref().unwrap_or("").trim();
    let api_username = discourse.api_username.as_deref().unwrap_or("").trim();
    if apikey.is_empty() || api_username.is_empty() {
        return Err(missing_config(
            "apikey/api_username",
            &format!("discourse {}", discourse.name),
            "apikey and api_username",
        ));
    }
    Ok(())
}

pub fn not_found(resource: &str, identifier: impl Display) -> anyhow::Error {
    anyhow!("{} not found: {}", resource, identifier)
}

pub fn missing_config(field: &str, resource: &str, hint: &str) -> anyhow::Error {
    anyhow!(
        "missing {} for {}; please set {} or check your config",
        field,
        resource,
        hint
    )
}

/// Quote one value for safe interpolation into a POSIX shell command.
pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Replace command-template placeholders with shell-quoted values.
///
/// Substitution is single-pass: text emitted for one placeholder is never
/// rescanned for another. Successive `str::replace` passes would rewrite the
/// text a previous pass had just inserted, splitting its quoting open - and
/// callers here deliberately pass the same value under two keys.
pub(crate) fn render_shell_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    'scan: while !rest.is_empty() {
        let Some(open) = rest.find('{') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..open]);
        let at_brace = &rest[open..];
        for (key, value) in replacements {
            let token = format!("{{{}}}", key);
            if let Some(tail) = at_brace.strip_prefix(token.as_str()) {
                out.push_str(&shell_quote(value));
                rest = tail;
                continue 'scan;
            }
        }
        // Not a recognised placeholder: emit the brace literally.
        out.push('{');
        rest = &at_brace[1..];
    }
    out
}

/// Reject targets that SSH could interpret as options or multiple arguments.
pub(crate) fn validate_ssh_target(target: &str) -> Result<()> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("ssh target is empty"));
    }
    if trimmed.starts_with('-') {
        return Err(anyhow!("ssh target cannot start with '-': {}", target));
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Err(anyhow!("ssh target cannot contain whitespace: {}", target));
    }
    Ok(())
}

pub fn parse_tags(raw: &str) -> Vec<String> {
    raw.split([';', ','])
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect()
}

pub fn fetch_fullname_from_url(baseurl: &str) -> Option<String> {
    let temp = DiscourseConfig {
        name: "temp".to_string(),
        baseurl: baseurl.to_string(),
        ..DiscourseConfig::default()
    };
    let client = match DiscourseClient::new(&temp) {
        Ok(client) => client,
        Err(err) => {
            eprintln!("Failed to query site title for {}: {}", baseurl, err);
            return None;
        }
    };
    match client.fetch_site_title() {
        Ok(title) => {
            let title = title.trim().to_string();
            if title.is_empty() { None } else { Some(title) }
        }
        Err(err) => {
            eprintln!("Failed to query site title for {}: {}", baseurl, err);
            None
        }
    }
}

pub fn open_url(url: &str) -> Result<()> {
    if url.trim().is_empty() {
        return Err(anyhow!("cannot open empty base URL"));
    }

    let mut cmd = if let Ok(opener) = std::env::var("DSC_BROWSER_OPENER") {
        let mut cmd = Command::new(opener);
        cmd.arg(url);
        cmd
    } else if cfg!(target_os = "macos") {
        let mut cmd = Command::new("open");
        cmd.arg(url);
        cmd
    } else if cfg!(target_os = "windows") {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", url]);
        cmd
    } else {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        cmd
    };

    let status = cmd.status().context("failed to launch browser opener")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("browser opener exited with status {}", status))
    }
}

/// Parse one-email-per-line input. Ignores blank lines, `#` comments
/// (full-line and inline), and leading/trailing whitespace. De-duplicates
/// while preserving the first-seen order, lowercasing as it goes.
pub fn parse_emails(input: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in input.lines() {
        let stripped = match line.find('#') {
            Some(idx) => &line[..idx],
            None => line,
        };
        let trimmed = stripped.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.contains('@') {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            out.push(lower);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        Config, DiscourseConfig, matches_tag_filter, parse_emails, render_shell_template,
        selected_discourses, shell_quote, validate_ssh_target,
    };

    #[test]
    fn tag_filter_matches_case_insensitively() {
        let forum = DiscourseConfig {
            name: "forum".to_string(),
            tags: Some(vec!["Production".to_string()]),
            ..DiscourseConfig::default()
        };
        assert!(matches_tag_filter(&forum, &["production".to_string()]));
        assert!(!matches_tag_filter(&forum, &["staging".to_string()]));
    }

    #[test]
    fn selected_discourses_rejects_empty_tags() {
        let config = Config {
            discourse: vec![DiscourseConfig {
                name: "forum".to_string(),
                ..DiscourseConfig::default()
            }],
            ..Config::default()
        };
        let err = selected_discourses(&config, None, Some("")).unwrap_err();
        assert!(err.to_string().contains("at least one non-empty tag"));
        let err = selected_discourses(&config, None, Some(",;")).unwrap_err();
        assert!(err.to_string().contains("at least one non-empty tag"));
    }

    #[test]
    fn selected_discourses_filters_by_tag_when_no_name_given() {
        let config = Config {
            discourse: vec![
                DiscourseConfig {
                    name: "prod".to_string(),
                    tags: Some(vec!["production".to_string()]),
                    ..DiscourseConfig::default()
                },
                DiscourseConfig {
                    name: "stage".to_string(),
                    tags: Some(vec!["staging".to_string()]),
                    ..DiscourseConfig::default()
                },
            ],
            ..Config::default()
        };
        let matched = selected_discourses(&config, None, Some("production")).unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "prod");

        let all = selected_discourses(&config, None, None).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn selected_discourses_by_name_ignores_tags() {
        let config = Config {
            discourse: vec![DiscourseConfig {
                name: "forum".to_string(),
                ..DiscourseConfig::default()
            }],
            ..Config::default()
        };
        let matched = selected_discourses(&config, Some("forum"), None).unwrap();
        assert_eq!(matched.len(), 1);
        assert!(selected_discourses(&config, Some("missing"), None).is_err());
    }

    #[test]
    fn shell_quotes_embedded_single_quotes() {
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn shell_template_quotes_injected_commands() {
        assert_eq!(
            render_shell_template("git clone {url}", &[("url", "repo; rm -rf /")]),
            "git clone 'repo; rm -rf /'"
        );
    }

    #[test]
    fn shell_template_does_not_rescan_substituted_text() {
        // The plugin/theme call sites pass one value under two keys. A value
        // that itself looks like the second placeholder must not be reopened
        // by a later pass, or its quoting splits and the tail escapes.
        let rendered = render_shell_template(
            "git clone {url}",
            &[("url", "{name}; id #"), ("name", "{name}; id #")],
        );
        assert_eq!(rendered, r"git clone '{name}; id #'");
    }

    #[test]
    fn shell_template_leaves_unknown_placeholders_alone() {
        assert_eq!(
            render_shell_template("echo {url} {other}", &[("url", "x")]),
            "echo 'x' {other}"
        );
    }

    #[test]
    fn validates_ssh_targets() {
        assert!(validate_ssh_target("discourse@example.com").is_ok());
        assert!(validate_ssh_target("").is_err());
        assert!(validate_ssh_target("-oProxyCommand=evil").is_err());
        assert!(validate_ssh_target("host another-argument").is_err());
    }

    #[test]
    fn parses_one_per_line() {
        let got = parse_emails("alice@example.com\nbob@example.com\n");
        assert_eq!(got, vec!["alice@example.com", "bob@example.com"]);
    }

    #[test]
    fn skips_blanks_and_comments() {
        let got = parse_emails(
            "\
# onboarding list
alice@example.com

# new hires below
bob@example.com # bob in marketing
",
        );
        assert_eq!(got, vec!["alice@example.com", "bob@example.com"]);
    }

    #[test]
    fn dedupes_preserving_first_seen_order() {
        let got = parse_emails("alice@example.com\nbob@example.com\nALICE@example.com");
        assert_eq!(got, vec!["alice@example.com", "bob@example.com"]);
    }

    #[test]
    fn rejects_lines_without_at() {
        let got = parse_emails("not_an_email\nalice@example.com");
        assert_eq!(got, vec!["alice@example.com"]);
    }

    #[test]
    fn lowercases_emails() {
        let got = parse_emails("Alice@Example.com");
        assert_eq!(got, vec!["alice@example.com"]);
    }

    #[test]
    fn handles_crlf_line_endings() {
        let got = parse_emails("alice@example.com\r\nbob@example.com\r\n");
        assert_eq!(got, vec!["alice@example.com", "bob@example.com"]);
    }
}

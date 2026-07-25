// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Append-only update log: one TSV line per `dsc update` pass per forum, so a
//! fleet round is auditable and re-runnable without repeating the day's work.
//! See spec/update-log.md.

use crate::cli::UpdateLogFormat;
use anyhow::{Result, anyhow};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
fn now_epoch() -> i64 {
    chrono::Utc::now().timestamp()
}
fn iso_to_epoch(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

/// Outcome category recorded for one forum's update pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    Updated,
    Current,
    SkippedRecent,
    SkippedRebuild,
    Failed,
}

impl LogKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LogKind::Updated => "updated",
            LogKind::Current => "current",
            LogKind::SkippedRecent => "skipped-recent",
            LogKind::SkippedRebuild => "skipped-rebuild",
            LogKind::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Option<LogKind> {
        Some(match s {
            "updated" => LogKind::Updated,
            "current" => LogKind::Current,
            "skipped-recent" => LogKind::SkippedRecent,
            "skipped-rebuild" => LogKind::SkippedRebuild,
            "failed" => LogKind::Failed,
            _ => return None,
        })
    }

    /// A pass that confirmed the forum is up to date - counts for skip-recent.
    fn is_success(self) -> bool {
        matches!(self, LogKind::Updated | LogKind::Current)
    }
}

#[derive(Debug, Serialize)]
pub struct LogRecord {
    pub timestamp: String,
    pub forum: String,
    pub outcome: String,
    pub from_version: String,
    pub to_version: String,
    pub detail: String,
}

/// Resolve the log path: `$DSC_UPDATE_LOG`, else `$XDG_STATE_HOME/dsc/update.log`
/// (default `~/.local/state/dsc/update.log`).
pub fn log_path() -> PathBuf {
    if let Some(p) = std::env::var_os("DSC_UPDATE_LOG") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".local/state")
        });
    base.join("dsc").join("update.log")
}

/// TSV-safe field: blanks become `-`, embedded tabs/newlines become spaces.
fn field(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        "-".to_string()
    } else {
        s.replace(['\t', '\n', '\r'], " ")
    }
}

/// Append one record. Best-effort: a logging failure must never break an update,
/// so errors are swallowed (with a one-line stderr note).
pub fn append(forum: &str, kind: LogKind, from: &str, to: &str, detail: &str) {
    let ts = now_iso();
    let line = format!(
        "{}\t{}\t{}\t{}\t{}\t{}\n",
        ts,
        field(forum),
        kind.as_str(),
        field(from),
        field(to),
        field(detail),
    );
    if let Err(e) = try_append(&line) {
        eprintln!("warning: could not write update log: {e}");
    }
}

fn try_append(line: &str) -> Result<()> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

fn parse_line(line: &str) -> Option<LogRecord> {
    let mut it = line.splitn(6, '\t');
    let timestamp = it.next()?.to_string();
    let forum = it.next()?.to_string();
    let outcome = it.next()?.to_string();
    LogKind::parse(&outcome)?; // ignore malformed lines
    Some(LogRecord {
        timestamp,
        forum,
        outcome,
        from_version: it.next().unwrap_or("-").to_string(),
        to_version: it.next().unwrap_or("-").to_string(),
        detail: it.next().unwrap_or("-").to_string(),
    })
}

pub fn read_records() -> Vec<LogRecord> {
    let Ok(content) = fs::read_to_string(log_path()) else {
        return Vec::new();
    };
    content.lines().filter_map(parse_line).collect()
}

/// Parse a duration like `24h`, `6h`, `7d`, `30m`, `90s`. A bare number is hours.
pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: u64 = num
        .parse()
        .map_err(|_| anyhow!("invalid duration '{s}' (try 24h, 7d, 30m)"))?;
    let secs = match unit {
        "" | "h" => n * 3600,
        "s" => n,
        "m" => n * 60,
        "d" => n * 86400,
        other => {
            return Err(anyhow!(
                "invalid duration unit '{other}' in '{s}' (use s/m/h/d)"
            ));
        }
    };
    Ok(Duration::from_secs(secs))
}

/// Was `forum` successfully updated (or confirmed current) within `window`?
pub fn updated_within(forum: &str, window: Duration) -> bool {
    let cutoff = now_epoch().saturating_sub(window.as_secs() as i64);
    read_records()
        .iter()
        .filter(|r| r.forum == forum && LogKind::parse(&r.outcome).is_some_and(LogKind::is_success))
        .filter_map(|r| iso_to_epoch(&r.timestamp))
        .any(|epoch| epoch >= cutoff)
}

fn version_cell(r: &LogRecord) -> String {
    match (r.from_version.as_str(), r.to_version.as_str()) {
        ("-", "-") => "-".to_string(),
        (f, "-") => f.to_string(),
        ("-", t) => t.to_string(),
        (f, t) if f == t => t.to_string(),
        (f, t) => format!("{f} → {t}"),
    }
}

/// Render the log. `latest` collapses to one row per forum (its most recent
/// record); `since` windows the output.
pub fn render(latest: bool, since: Option<Duration>, format: UpdateLogFormat) -> Result<()> {
    let mut records = read_records();

    if let Some(window) = since {
        let cutoff = now_epoch().saturating_sub(window.as_secs() as i64);
        records.retain(|r| iso_to_epoch(&r.timestamp).is_some_and(|e| e >= cutoff));
    }

    if latest {
        // Keep the last record per forum; BTreeMap gives stable forum ordering.
        use std::collections::BTreeMap;
        let mut by_forum: BTreeMap<String, LogRecord> = BTreeMap::new();
        for r in records {
            by_forum.insert(r.forum.clone(), r);
        }
        records = by_forum.into_values().collect();
    }

    match format {
        UpdateLogFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&records)?);
        }
        UpdateLogFormat::Md => {
            println!("| when (UTC) | forum | outcome | version |");
            println!("|---|---|---|---|");
            for r in &records {
                println!(
                    "| {} | {} | {} | {} |",
                    r.timestamp,
                    r.forum,
                    r.outcome,
                    version_cell(r)
                );
            }
        }
        UpdateLogFormat::Text => {
            if records.is_empty() {
                println!("No update log entries found ({}).", log_path().display());
                return Ok(());
            }
            let fw = records
                .iter()
                .map(|r| r.forum.len())
                .max()
                .unwrap_or(5)
                .max(5);
            let ow = records
                .iter()
                .map(|r| r.outcome.len())
                .max()
                .unwrap_or(7)
                .max(7);
            for r in &records {
                println!(
                    "{}  {:<fw$}  {:<ow$}  {}",
                    r.timestamp,
                    r.forum,
                    r.outcome,
                    version_cell(r),
                    fw = fw,
                    ow = ow,
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_units() {
        assert_eq!(parse_duration("24h").unwrap(), Duration::from_secs(86400));
        assert_eq!(parse_duration("24").unwrap(), Duration::from_secs(86400)); // bare = hours
        assert_eq!(parse_duration("7d").unwrap(), Duration::from_secs(604800));
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
        assert!(parse_duration("5y").is_err());
        assert!(parse_duration("abc").is_err());
    }

    #[test]
    fn line_round_trips() {
        let line = "2026-07-01T09:12:03Z\tbawmedical\tupdated\t2026.6.0\t2026.7.0\t-";
        let r = parse_line(line).expect("parse");
        assert_eq!(r.forum, "bawmedical");
        assert_eq!(r.outcome, "updated");
        assert_eq!(r.from_version, "2026.6.0");
        assert_eq!(version_cell(&r), "2026.6.0 → 2026.7.0");
    }

    #[test]
    fn field_is_tsv_safe() {
        assert_eq!(field("  "), "-");
        assert_eq!(field("a\tb\nc"), "a b c");
        assert_eq!(field("plain"), "plain");
    }

    #[test]
    fn malformed_lines_are_skipped() {
        assert!(parse_line("not a log line").is_none());
        assert!(parse_line("2026-07-01T09:12:03Z\tfoo\tbogus-outcome\t-\t-\t-").is_none());
    }

    #[test]
    fn is_success_only_for_updated_and_current() {
        assert!(LogKind::Updated.is_success());
        assert!(LogKind::Current.is_success());
        assert!(!LogKind::Failed.is_success());
        assert!(!LogKind::SkippedRecent.is_success());
    }
}

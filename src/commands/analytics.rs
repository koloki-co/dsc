// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! `dsc analytics` — community-health snapshot per `spec/analytics.md`.
//!
//! Three modes share one data path:
//!
//! - **single window** (`--since 30d` alone): one column.
//! - **compare** (`--since 30d --compare`): two columns (current, previous).
//! - **snapshot** (`--snapshot`): N columns, default `24h,7d,30d,1y`.
//!
//! Internally every mode is "list of windows + a report cache". The cache
//! is populated by spawning one thread per `(report_id, window)` pair so
//! a snapshot of N=4 windows × 9 reports completes in roughly the time
//! of the slowest single call rather than 36× sequential.

use crate::api::{AdminReport, DiscourseClient};
use crate::cli::AnalyticsFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::parse_since_cutoff;
use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Utc};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::io::{self, IsTerminal};
use std::sync::{Arc, Mutex};
use std::thread;

const SCHEMA_VERSION: u32 = 1;

/// All the report IDs the analytics command might fetch. Listed once so
/// the cache populator can fan out without us forgetting one.
const REPORT_IDS: &[&str] = &[
    "topics",
    "posts",
    "likes",
    "flags",
    "new_contributors",
    "trust_level_growth",
    "time_to_first_response",
    "topics_with_no_response",
    "moderators_activity",
];

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn analytics(
    config: &Config,
    discourse_name: &str,
    since: &str,
    compare: bool,
    snapshot: bool,
    periods: Option<&str>,
    section_filter: SectionFilter,
    mut format: AnalyticsFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let now = Utc::now();

    // Resolve windows per mode. Order matters: position 0 is the
    // "primary" column (the one shown alone in single-window mode); the
    // rest are comparison/snapshot columns in left-to-right reading order.
    let windows = if snapshot {
        let raw = periods.unwrap_or("24h,7d,30d,1y");
        parse_periods(raw, now)?
    } else if compare {
        let cur = window_from_since(since, now)?;
        let prev = previous_window_of(&cur);
        vec![cur, prev]
    } else {
        vec![window_from_since(since, now)?]
    };

    let column_headers: Vec<String> = if snapshot {
        windows.iter().map(|w| w.label.clone()).collect()
    } else if compare {
        vec!["current".to_string(), "previous".to_string()]
    } else {
        vec!["value".to_string()]
    };

    // Auto-fall-through `table` → `text` on non-TTY stdout so cron-piped
    // output stays parseable.
    if matches!(format, AnalyticsFormat::Table) && !io::stdout().is_terminal() {
        format = AnalyticsFormat::Text;
    }

    let cache = populate_cache(&client, &windows)?;
    let report = build_report(
        discourse_name,
        &windows,
        &column_headers,
        section_filter,
        snapshot,
        &cache,
    );
    render(&report, format)
}

// ---------------------------------------------------------------------------
// Window helpers
// ---------------------------------------------------------------------------

fn window_from_since(since: &str, now: DateTime<Utc>) -> Result<Window> {
    let cutoff = parse_since_cutoff(since)?;
    let (start, end) = if cutoff <= now {
        (cutoff, now)
    } else {
        (now, cutoff)
    };
    Ok(Window {
        since: start,
        until: end,
        label: since.to_string(),
        clamped: false,
    })
}

fn previous_window_of(w: &Window) -> Window {
    let len = w.duration();
    Window {
        since: w.since - len,
        until: w.since,
        label: w.label.clone(),
        clamped: false,
    }
}

fn parse_periods(raw: &str, now: DateTime<Utc>) -> Result<Vec<Window>> {
    let mut out = Vec::new();
    for piece in raw.split(',') {
        let p = piece.trim();
        if p.is_empty() {
            continue;
        }
        out.push(window_from_since(p, now)?);
    }
    if out.is_empty() {
        anyhow::bail!("--periods must contain at least one duration");
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionFilter {
    All,
    Growth,
    Activity,
    Health,
}

#[derive(Clone, Debug, Serialize)]
struct Window {
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    label: String,
    clamped: bool,
}

impl Window {
    fn iso_date_since(&self) -> String {
        format_yyyy_mm_dd(&self.since)
    }
    fn iso_date_until(&self) -> String {
        format_yyyy_mm_dd(&self.until)
    }
    fn duration(&self) -> Duration {
        self.until - self.since
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Direction {
    Up,
    Down,
    Neither,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Unit {
    Count,
    Percent,
    Minutes,
    Hours,
    Ratio,
    PerThousandPosts,
}

#[derive(Clone, Debug, Serialize)]
struct Metric {
    label: String,
    key: String,
    /// One slot per column. `None` means the metric is genuinely
    /// undefined for that window (e.g. zero-topic divisor); see
    /// `not_implemented` for "we haven't built this yet" markers.
    values: Vec<Option<f64>>,
    desirable: Direction,
    unit: Unit,
    not_implemented: bool,
}

impl Metric {
    fn new(label: &str, key: &str, desirable: Direction, unit: Unit, n: usize) -> Self {
        Self {
            label: label.to_string(),
            key: key.to_string(),
            values: vec![None; n],
            desirable,
            unit,
            not_implemented: false,
        }
    }
    fn with_values(mut self, v: Vec<Option<f64>>) -> Self {
        self.values = v;
        self
    }
    fn stub(mut self) -> Self {
        self.not_implemented = true;
        self
    }
    /// % delta from values[1] → values[0]. Used in compare mode.
    fn delta_pct(&self) -> Option<f64> {
        match (
            self.values.first().copied().flatten(),
            self.values.get(1).copied().flatten(),
        ) {
            (Some(c), Some(p)) if p != 0.0 => Some(((c - p) / p) * 100.0),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct AnalyticsReport {
    schema: u32,
    discourse: String,
    snapshot: bool,
    windows: Vec<Window>,
    column_headers: Vec<String>,
    growth: Option<Vec<Metric>>,
    activity: Option<Vec<Metric>>,
    health: Option<Vec<Metric>>,
}

// ---------------------------------------------------------------------------
// Concurrent report cache
// ---------------------------------------------------------------------------

/// Maps `(report_id, window_index)` to an optional `AdminReport`. None
/// means Discourse returned a tolerable error (404/403/500) and the
/// metric should render as `—`.
type ReportCache = HashMap<(String, usize), Option<AdminReport>>;

/// Max in-flight HTTP requests at any moment. Above this, observation on
/// dhi-discourse showed nginx 429s even for single-window mode. The
/// cross-cutting 429 retry would catch them but slow the run dramatically;
/// staying below the burst limit is faster and more polite. 4 is empirical.
const ANALYTICS_PARALLELISM: usize = 4;

fn populate_cache(client: &DiscourseClient, windows: &[Window]) -> Result<ReportCache> {
    let cache: Arc<Mutex<ReportCache>> = Arc::new(Mutex::new(HashMap::new()));

    // Build the full task list, then dispatch with a bounded worker pool.
    // We could do "parallel-within-window, sequential-between-window" but
    // that lets each window finish its slowest call before the next one
    // can start — pointless idle time. A flat task queue keeps the workers
    // saturated.
    let tasks: Vec<(String, usize, String, String)> = windows
        .iter()
        .enumerate()
        .flat_map(|(w_idx, window)| {
            let start = window.iso_date_since();
            let end = window.iso_date_until();
            REPORT_IDS
                .iter()
                .map(move |id| (id.to_string(), w_idx, start.clone(), end.clone()))
        })
        .collect();
    let queue = Arc::new(Mutex::new(tasks.into_iter()));

    thread::scope(|scope| {
        for _ in 0..ANALYTICS_PARALLELISM {
            let client = client.clone();
            let cache = cache.clone();
            let queue = queue.clone();
            scope.spawn(move || {
                loop {
                    let next = { queue.lock().ok().and_then(|mut q| q.next()) };
                    let Some((id, w_idx, start, end)) = next else {
                        break;
                    };
                    let value = fetch_optional(&client, &id, &start, &end);
                    if let Ok(mut guard) = cache.lock() {
                        guard.insert((id, w_idx), value);
                    }
                }
            });
        }
    });

    Ok(Arc::try_unwrap(cache)
        .map_err(|_| anyhow::anyhow!("cache still has live references"))?
        .into_inner()
        .unwrap_or_default())
}

fn report_at<'a>(cache: &'a ReportCache, id: &str, w: usize) -> Option<&'a AdminReport> {
    cache.get(&(id.to_string(), w)).and_then(|opt| opt.as_ref())
}

/// Per-window total for a single report. None when the report was
/// missing OR Discourse returned no data.
fn totals_for(cache: &ReportCache, id: &str, n_windows: usize) -> Vec<Option<f64>> {
    (0..n_windows)
        .map(|w| report_at(cache, id, w).map(|r: &AdminReport| r.current_total()))
        .collect()
}

fn averages_for(cache: &ReportCache, id: &str, n_windows: usize) -> Vec<Option<f64>> {
    (0..n_windows)
        .map(|w| report_at(cache, id, w).and_then(|r: &AdminReport| r.average))
        .collect()
}

fn ratio_per_window(num: &[Option<f64>], den: &[Option<f64>]) -> Vec<Option<f64>> {
    num.iter()
        .zip(den.iter())
        .map(|(n, d)| match (n, d) {
            (Some(n), Some(d)) if *d > 0.0 => Some(n / d),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Section construction
// ---------------------------------------------------------------------------

fn build_report(
    discourse: &str,
    windows: &[Window],
    column_headers: &[String],
    filter: SectionFilter,
    snapshot: bool,
    cache: &ReportCache,
) -> AnalyticsReport {
    let n = windows.len();
    let growth = if matches!(filter, SectionFilter::All | SectionFilter::Growth) {
        Some(build_growth(cache, n))
    } else {
        None
    };
    let activity = if matches!(filter, SectionFilter::All | SectionFilter::Activity) {
        Some(build_activity(cache, n))
    } else {
        None
    };
    let health = if matches!(filter, SectionFilter::All | SectionFilter::Health) {
        Some(build_health(cache, n))
    } else {
        None
    };
    AnalyticsReport {
        schema: SCHEMA_VERSION,
        discourse: discourse.to_string(),
        snapshot,
        windows: windows.to_vec(),
        column_headers: column_headers.to_vec(),
        growth,
        activity,
        health,
    }
}

fn build_growth(cache: &ReportCache, n: usize) -> Vec<Metric> {
    vec![
        Metric::new(
            "new contributors",
            "new_contributors",
            Direction::Up,
            Unit::Count,
            n,
        )
        .with_values(totals_for(cache, "new_contributors", n)),
        Metric::new(
            "reactivated users",
            "reactivated_users",
            Direction::Up,
            Unit::Count,
            n,
        )
        .stub(),
        Metric::new(
            "lost regulars",
            "lost_regulars",
            Direction::Down,
            Unit::Count,
            n,
        )
        .stub(),
        Metric::new(
            "net active change",
            "net_active_change",
            Direction::Up,
            Unit::Count,
            n,
        )
        .stub(),
        Metric::new(
            "trust-level promotions",
            "trust_level_promotions",
            Direction::Up,
            Unit::Count,
            n,
        )
        .with_values(totals_for(cache, "trust_level_growth", n)),
    ]
}

fn build_activity(cache: &ReportCache, n: usize) -> Vec<Metric> {
    let mut out = Vec::new();

    let topics = totals_for(cache, "topics", n);
    let posts = totals_for(cache, "posts", n);
    let no_response = totals_for(cache, "topics_with_no_response", n);

    out.push(
        Metric::new(
            "topics created",
            "topics_created",
            Direction::Up,
            Unit::Count,
            n,
        )
        .with_values(topics.clone()),
    );
    out.push(
        Metric::new(
            "posts created",
            "posts_created",
            Direction::Up,
            Unit::Count,
            n,
        )
        .with_values(posts.clone()),
    );
    out.push(
        Metric::new(
            "posts per topic",
            "posts_per_topic",
            Direction::Up,
            Unit::Ratio,
            n,
        )
        .with_values(ratio_per_window(&posts, &topics)),
    );
    out.push(
        Metric::new(
            "unique posters",
            "unique_posters",
            Direction::Up,
            Unit::Count,
            n,
        )
        .stub(),
    );
    out.push(
        Metric::new(
            "top-10 share",
            "top_10_share",
            Direction::Down,
            Unit::Percent,
            n,
        )
        .stub(),
    );

    let coverage: Vec<Option<f64>> = topics
        .iter()
        .zip(no_response.iter())
        .map(|(t, nr)| match (t, nr) {
            (Some(t), Some(nr)) if *t > 0.0 => Some(((t - nr) / t) * 100.0),
            _ => None,
        })
        .collect();
    out.push(
        Metric::new(
            "reply coverage",
            "reply_coverage",
            Direction::Up,
            Unit::Percent,
            n,
        )
        .with_values(coverage),
    );

    out.push(
        Metric::new(
            "median time to first reply",
            "median_time_to_first_reply",
            Direction::Down,
            Unit::Minutes,
            n,
        )
        .with_values(averages_for(cache, "time_to_first_response", n)),
    );

    out
}

fn build_health(cache: &ReportCache, n: usize) -> Vec<Metric> {
    let mut out = Vec::new();
    let likes = totals_for(cache, "likes", n);
    let posts = totals_for(cache, "posts", n);
    let mods = totals_for(cache, "moderators_activity", n);

    out.push(
        Metric::new(
            "likes per post",
            "likes_per_post",
            Direction::Up,
            Unit::Ratio,
            n,
        )
        .with_values(ratio_per_window(&likes, &posts)),
    );
    out.push(
        Metric::new(
            "returning poster rate",
            "returning_poster_rate",
            Direction::Up,
            Unit::Percent,
            n,
        )
        .stub(),
    );
    out.push(
        Metric::new(
            "flags raised",
            "flags_raised",
            Direction::Down,
            Unit::Count,
            n,
        )
        .with_values(totals_for(cache, "flags", n)),
    );
    out.push(
        Metric::new(
            "flag resolution time",
            "flag_resolution_time",
            Direction::Down,
            Unit::Hours,
            n,
        )
        .stub(),
    );

    let mar: Vec<Option<f64>> = mods
        .iter()
        .zip(posts.iter())
        .map(|(m, p)| match (m, p) {
            (Some(m), Some(p)) if *p > 0.0 => Some((m / p) * 1000.0),
            _ => None,
        })
        .collect();
    out.push(
        Metric::new(
            "moderator action rate",
            "moderator_action_rate",
            Direction::Neither,
            Unit::PerThousandPosts,
            n,
        )
        .with_values(mar),
    );
    out.push(
        Metric::new(
            "solo-thread rate",
            "solo_thread_rate",
            Direction::Down,
            Unit::Percent,
            n,
        )
        .stub(),
    );

    out
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

fn render(report: &AnalyticsReport, format: AnalyticsFormat) -> Result<()> {
    match format {
        AnalyticsFormat::Text => render_text(report),
        AnalyticsFormat::Table => render_table(report),
        AnalyticsFormat::Json => render_json(report),
        AnalyticsFormat::Yaml => render_yaml(report),
        AnalyticsFormat::Markdown => render_markdown(report, false),
        AnalyticsFormat::MarkdownTable => render_markdown(report, true),
        AnalyticsFormat::Csv => render_csv(report),
    }
}

fn render_text(report: &AnalyticsReport) -> Result<()> {
    print_header_text(report);
    let compare_mode = !report.snapshot && report.column_headers.len() == 2;
    for (name, metrics) in iter_sections(report) {
        println!();
        println!("{}", name);
        let label_w = metrics
            .iter()
            .map(|m| m.label.chars().count())
            .max()
            .unwrap_or(0)
            .max(20);
        let cols = report.column_headers.len();
        let val_w = column_widths(metrics, cols);
        for m in metrics {
            print!("  {}", pad_right(&m.label, label_w));
            for (c, w) in val_w.iter().enumerate() {
                let s = format_value(
                    m.values.get(c).copied().flatten(),
                    m.unit,
                    m.not_implemented,
                );
                print!("  {}", right_align(&s, *w));
            }
            if compare_mode {
                let pct = m
                    .delta_pct()
                    .map(|p| format!("({:+.0}%)", p))
                    .unwrap_or_default();
                print!("  {}", pct);
            }
            println!();
        }
    }
    Ok(())
}

fn render_table(report: &AnalyticsReport) -> Result<()> {
    print_header_text(report);
    let cols = report.column_headers.len();
    let compare_mode = !report.snapshot && cols == 2;

    for (name, metrics) in iter_sections(report) {
        println!();
        println!("{}", name);

        let label_w = metrics
            .iter()
            .map(|m| m.label.chars().count())
            .max()
            .unwrap_or(0)
            .max(6)
            .max("metric".len());
        let mut col_w = column_widths(metrics, cols);
        // Headers may be wider than any cell.
        for (i, h) in report.column_headers.iter().enumerate() {
            let hw = h.chars().count();
            if hw > col_w[i] {
                col_w[i] = hw;
            }
        }
        let pct_w = if compare_mode { 7 } else { 0 };

        // Top border
        let mut widths: Vec<usize> = std::iter::once(label_w)
            .chain(col_w.iter().copied())
            .collect();
        if compare_mode {
            widths.push(pct_w);
        }
        println!("{}", border_line('┌', '┬', '┐', &widths));

        // Header row
        print!("│ {} ", pad_right("metric", label_w));
        for (i, h) in report.column_headers.iter().enumerate() {
            print!("│ {} ", center(h, col_w[i]));
        }
        if compare_mode {
            print!("│ {} ", center("Δ", pct_w));
        }
        println!("│");

        // Header separator
        println!("{}", border_line('├', '┼', '┤', &widths));

        for m in metrics {
            print!("│ {} ", pad_right(&m.label, label_w));
            for (c, w) in col_w.iter().enumerate() {
                let s = format_value(
                    m.values.get(c).copied().flatten(),
                    m.unit,
                    m.not_implemented,
                );
                print!("│ {} ", right_align(&s, *w));
            }
            if compare_mode {
                let pct = m
                    .delta_pct()
                    .map(|p| format!("{:+.0}%", p))
                    .unwrap_or_else(|| "—".to_string());
                print!("│ {} ", right_align(&pct, pct_w));
            }
            println!("│");
        }

        println!("{}", border_line('└', '┴', '┘', &widths));
    }
    Ok(())
}

fn print_header_text(report: &AnalyticsReport) {
    if report.snapshot {
        let now = Utc::now();
        println!(
            "analytics for {} — snapshot at {} UTC",
            report.discourse,
            now.format("%Y-%m-%d %H:%M")
        );
    } else {
        let w = &report.windows[0];
        println!(
            "analytics for {} — {} ({} → {})",
            report.discourse,
            w.label,
            w.iso_date_since(),
            w.iso_date_until()
        );
        if w.clamped {
            println!("(window clamped — install is younger than --since)");
        }
    }
}

fn render_json(report: &AnalyticsReport) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&report_to_json(report))?);
    Ok(())
}

fn render_yaml(report: &AnalyticsReport) -> Result<()> {
    println!("{}", serde_yaml::to_string(&report_to_json(report))?);
    Ok(())
}

fn render_markdown(report: &AnalyticsReport, table: bool) -> Result<()> {
    let cols = report.column_headers.len();
    let compare_mode = !report.snapshot && cols == 2;
    println!("# analytics for {}", report.discourse);
    println!();
    if report.snapshot {
        println!(
            "Snapshot at **{}**",
            Utc::now().format("%Y-%m-%d %H:%M UTC")
        );
    } else {
        let w = &report.windows[0];
        println!(
            "Window: **{}** ({} → {})",
            w.label,
            w.iso_date_since(),
            w.iso_date_until()
        );
    }

    for (name, metrics) in iter_sections(report) {
        println!();
        println!("## {}", name);
        println!();
        if table {
            print!("| metric |");
            for h in &report.column_headers {
                print!(" {} |", h);
            }
            if compare_mode {
                print!(" Δ |");
            }
            println!();
            print!("| --- |");
            for _ in 0..cols {
                print!(" ---: |");
            }
            if compare_mode {
                print!(" ---: |");
            }
            println!();
            for m in metrics {
                print!("| {} |", m.label);
                for c in 0..cols {
                    let s = format_value(
                        m.values.get(c).copied().flatten(),
                        m.unit,
                        m.not_implemented,
                    );
                    print!(" {} |", s);
                }
                if compare_mode {
                    let pct = m
                        .delta_pct()
                        .map(|p| format!("{:+.0}%", p))
                        .unwrap_or_else(|| "—".to_string());
                    print!(" {} |", pct);
                }
                println!();
            }
        } else {
            for m in metrics {
                print!("- **{}** —", m.label);
                for (i, h) in report.column_headers.iter().enumerate() {
                    let s = format_value(
                        m.values.get(i).copied().flatten(),
                        m.unit,
                        m.not_implemented,
                    );
                    if cols == 1 {
                        print!(" {}", s);
                    } else {
                        print!(" {}: {}", h, s);
                        if i + 1 < cols {
                            print!(",");
                        }
                    }
                }
                if compare_mode && let Some(p) = m.delta_pct() {
                    print!(" (`{:+.0}%`)", p);
                }
                println!();
            }
        }
    }
    Ok(())
}

fn render_csv(report: &AnalyticsReport) -> Result<()> {
    let mut writer = csv::Writer::from_writer(io::stdout());
    let mut header: Vec<String> = vec!["section".into(), "metric".into()];
    for h in &report.column_headers {
        header.push(h.clone());
    }
    header.push("desirable_direction".into());
    header.push("unit".into());
    writer.write_record(&header)?;

    let cols = report.column_headers.len();
    for (name, metrics) in iter_sections(report) {
        for m in metrics {
            let mut row: Vec<String> = vec![name.into(), m.label.clone()];
            for c in 0..cols {
                row.push(
                    m.values
                        .get(c)
                        .copied()
                        .flatten()
                        .map(|v| format!("{}", v))
                        .unwrap_or_default(),
                );
            }
            row.push(
                match m.desirable {
                    Direction::Up => "up",
                    Direction::Down => "down",
                    Direction::Neither => "neither",
                }
                .into(),
            );
            row.push(unit_str(m.unit).into());
            writer.write_record(&row)?;
        }
    }
    writer.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Render helpers
// ---------------------------------------------------------------------------

fn iter_sections(report: &AnalyticsReport) -> Vec<(&'static str, &[Metric])> {
    let mut v: Vec<(&'static str, &[Metric])> = Vec::new();
    if let Some(g) = &report.growth {
        v.push(("growth", g));
    }
    if let Some(a) = &report.activity {
        v.push(("activity", a));
    }
    if let Some(h) = &report.health {
        v.push(("health", h));
    }
    v
}

fn fetch_optional(
    client: &DiscourseClient,
    report_id: &str,
    start: &str,
    end: &str,
) -> Option<AdminReport> {
    match client.fetch_admin_report(report_id, start, end) {
        Ok(r) => Some(r),
        Err(err) => {
            let msg = err.to_string();
            // Tolerate per-report 404/403/500. The cache slot stays None
            // and the metric renders as `—`.
            let known_missing = msg.contains(" 404 ")
                || msg.contains(" 403 ")
                || msg.contains(" 500 ")
                || msg.contains("not found");
            if known_missing {
                None
            } else {
                eprintln!(
                    "[analytics] warning fetching report '{}': {}",
                    report_id, err
                );
                None
            }
        }
    }
}

fn column_widths(metrics: &[Metric], cols: usize) -> Vec<usize> {
    (0..cols)
        .map(|c| {
            metrics
                .iter()
                .map(|m| {
                    visual_width(&format_value(
                        m.values.get(c).copied().flatten(),
                        m.unit,
                        m.not_implemented,
                    ))
                })
                .max()
                .unwrap_or(0)
                .max(6)
        })
        .collect()
}

fn visual_width(s: &str) -> usize {
    s.chars().count()
}

fn pad_right(s: &str, width: usize) -> String {
    let w = visual_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

fn right_align(s: &str, width: usize) -> String {
    let w = visual_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - w), s)
    }
}

fn center(s: &str, width: usize) -> String {
    let w = visual_width(s);
    if w >= width {
        return s.to_string();
    }
    let total = width - w;
    let left = total / 2;
    let right = total - left;
    format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
}

fn border_line(start: char, mid: char, end: char, widths: &[usize]) -> String {
    let mut out = String::new();
    out.push(start);
    for (i, w) in widths.iter().enumerate() {
        for _ in 0..(*w + 2) {
            out.push('─');
        }
        out.push(if i + 1 == widths.len() { end } else { mid });
    }
    out
}

fn unit_str(u: Unit) -> &'static str {
    match u {
        Unit::Count => "count",
        Unit::Percent => "percent",
        Unit::Minutes => "minutes",
        Unit::Hours => "hours",
        Unit::Ratio => "ratio",
        Unit::PerThousandPosts => "per_1k_posts",
    }
}

fn format_value(v: Option<f64>, unit: Unit, not_impl: bool) -> String {
    if not_impl {
        return "— (n/i)".to_string();
    }
    let v = v.map(|x| if x == 0.0 { 0.0 } else { x });
    match (v, unit) {
        (None, _) => "—".to_string(),
        (Some(x), Unit::Count) => format_count(x),
        (Some(x), Unit::Percent) => format!("{:.0}%", x),
        (Some(x), Unit::Minutes) => format_minutes(x),
        (Some(x), Unit::Hours) => format!("{:.1}h", x),
        (Some(x), Unit::Ratio) => format!("{:.1}", x),
        (Some(x), Unit::PerThousandPosts) => format!("{:.1} / 1k", x),
    }
}

/// Integer count with thousand separators (commas) for readability.
fn format_count(x: f64) -> String {
    let n = x as i64;
    let neg = n < 0;
    let digits = n.unsigned_abs().to_string();
    // Walk the digit string from the right, inserting a comma every 3
    // characters except at the very start.
    let bytes: Vec<u8> = digits.into_bytes();
    let mut out = String::with_capacity(bytes.len() + bytes.len() / 3);
    let len = bytes.len();
    for (i, b) in bytes.iter().enumerate() {
        let from_right = len - i;
        if i > 0 && from_right.is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    if neg {
        out.insert(0, '-');
    }
    out
}

fn format_minutes(x: f64) -> String {
    if x >= 60.0 {
        let h = x / 60.0;
        format!("{:.1}h", h)
    } else {
        format!("{:.0}m", x)
    }
}

fn format_yyyy_mm_dd(d: &DateTime<Utc>) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
}

// ---------------------------------------------------------------------------
// JSON serialisation
// ---------------------------------------------------------------------------

fn report_to_json(report: &AnalyticsReport) -> Value {
    let mut top = Map::new();
    top.insert("schema".to_string(), json!(report.schema));
    top.insert("discourse".to_string(), json!(report.discourse));
    top.insert("snapshot".to_string(), json!(report.snapshot));
    top.insert(
        "windows".to_string(),
        Value::Array(
            report
                .windows
                .iter()
                .map(|w| {
                    json!({
                        "label": w.label,
                        "since": w.since.to_rfc3339(),
                        "until": w.until.to_rfc3339(),
                    })
                })
                .collect(),
        ),
    );
    for (name, metrics) in iter_sections(report) {
        top.insert(
            name.to_string(),
            section_to_json(metrics, &report.column_headers),
        );
    }
    Value::Object(top)
}

fn section_to_json(metrics: &[Metric], headers: &[String]) -> Value {
    let mut out = Map::new();
    for m in metrics {
        let mut entry = Map::new();
        let mut values = Map::new();
        for (i, h) in headers.iter().enumerate() {
            values.insert(h.clone(), float_or_null(m.values.get(i).copied().flatten()));
        }
        entry.insert("values".to_string(), Value::Object(values));
        entry.insert(
            "desirable".to_string(),
            json!(match m.desirable {
                Direction::Up => "up",
                Direction::Down => "down",
                Direction::Neither => "neither",
            }),
        );
        entry.insert("unit".to_string(), json!(unit_str(m.unit)));
        if m.not_implemented {
            entry.insert("not_implemented".to_string(), json!(true));
        }
        out.insert(m.key.clone(), Value::Object(entry));
    }
    Value::Object(out)
}

fn float_or_null(v: Option<f64>) -> Value {
    match v {
        None => Value::Null,
        Some(x) if x.is_finite() => json!(x),
        _ => Value::Null,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_delta_pct_works_on_compare_layout() {
        let m = Metric::new("x", "x", Direction::Up, Unit::Count, 2)
            .with_values(vec![Some(80.0), Some(100.0)]);
        assert_eq!(m.delta_pct(), Some(-20.0));
    }

    #[test]
    fn metric_delta_pct_none_when_previous_zero() {
        let m = Metric::new("x", "x", Direction::Up, Unit::Count, 2)
            .with_values(vec![Some(10.0), Some(0.0)]);
        assert!(m.delta_pct().is_none());
    }

    #[test]
    fn metric_delta_pct_none_for_single_window() {
        let m = Metric::new("x", "x", Direction::Up, Unit::Count, 1).with_values(vec![Some(10.0)]);
        assert!(m.delta_pct().is_none());
    }

    #[test]
    fn ratio_per_window_handles_zero_and_missing() {
        let n = vec![Some(10.0), Some(20.0), None];
        let d = vec![Some(2.0), Some(0.0), Some(5.0)];
        let r = ratio_per_window(&n, &d);
        assert_eq!(r, vec![Some(5.0), None, None]);
    }

    #[test]
    fn format_value_em_dash_for_none() {
        assert_eq!(format_value(None, Unit::Count, false), "—");
        assert_eq!(format_value(Some(42.0), Unit::Count, true), "— (n/i)");
    }

    #[test]
    fn format_count_inserts_thousand_separators() {
        assert_eq!(format_count(0.0), "0");
        assert_eq!(format_count(42.0), "42");
        assert_eq!(format_count(1_234.0), "1,234");
        assert_eq!(format_count(12_345.0), "12,345");
        assert_eq!(format_count(1_234_567.0), "1,234,567");
        assert_eq!(format_count(-1_500.0), "-1,500");
    }

    #[test]
    fn format_minutes_rolls_to_hours() {
        assert_eq!(format_minutes(45.0), "45m");
        assert_eq!(format_minutes(90.0), "1.5h");
    }

    #[test]
    fn parse_periods_default_set() {
        let now = Utc::now();
        let ws = parse_periods("24h,7d,30d,1y", now).unwrap();
        assert_eq!(ws.len(), 4);
        assert_eq!(ws[0].label, "24h");
        assert_eq!(ws[3].label, "1y");
    }

    #[test]
    fn parse_periods_skips_blanks() {
        let now = Utc::now();
        let ws = parse_periods("7d, ,30d", now).unwrap();
        assert_eq!(ws.len(), 2);
    }

    #[test]
    fn parse_periods_rejects_empty() {
        let now = Utc::now();
        assert!(parse_periods("", now).is_err());
    }

    #[test]
    fn previous_window_is_immediately_preceding() {
        let now = Utc::now();
        let cur = window_from_since("7d", now).unwrap();
        let prev = previous_window_of(&cur);
        assert_eq!(prev.until, cur.since);
        assert_eq!(prev.duration(), cur.duration());
    }

    #[test]
    fn border_line_lengths_match_widths() {
        let line = border_line('┌', '┬', '┐', &[6, 4]);
        // Each column is width+2 dashes, plus the four corners.
        let dashes = line.chars().filter(|c| *c == '─').count();
        assert_eq!(dashes, (6 + 2) + (4 + 2));
        assert!(line.starts_with('┌'));
        assert!(line.ends_with('┐'));
        assert!(line.contains('┬'));
    }
}

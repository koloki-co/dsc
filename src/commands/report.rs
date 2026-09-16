// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! `dsc report <name>` — a single raw Discourse admin report.
//!
//! Distinct from `dsc analytics`: no cross-report derivation, ratios, or
//! community-health framing, just a thin pass-through/formatter over the
//! same `/admin/reports/{id}.json` endpoint `dsc analytics` already wraps.
//! `<name>` is any report id Discourse accepts on this forum (e.g.
//! `signups`, `topics`, `posts`, `likes`) — see `spec/commands/analytics.md`
//! for the set `dsc` already trusts.

use crate::api::DiscourseClient;
use crate::cli::ListFormat;
use crate::commands::common::{emit_result, ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::parse_since_cutoff;
use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
struct ReportView {
    report_id: String,
    since: String,
    start_date: String,
    end_date: String,
    total: f64,
    average: Option<f64>,
    higher_is_better: Option<bool>,
    data: Value,
}

pub fn report(
    config: &Config,
    discourse_name: &str,
    report_id: &str,
    since: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let now = Utc::now();
    let cutoff = parse_since_cutoff(since)?;
    let (start, end) = if cutoff <= now {
        (cutoff, now)
    } else {
        (now, cutoff)
    };
    let start_date = start.format("%Y-%m-%d").to_string();
    let end_date = end.format("%Y-%m-%d").to_string();

    let admin_report = client.fetch_admin_report(report_id, &start_date, &end_date)?;
    let view = ReportView {
        report_id: report_id.to_string(),
        since: since.to_string(),
        start_date,
        end_date,
        total: admin_report.current_total(),
        average: admin_report.average,
        higher_is_better: admin_report.higher_is_better,
        data: admin_report.data.clone(),
    };
    let text = render_text(&view);
    emit_result(format, &view, &text)
}

fn render_text(view: &ReportView) -> String {
    let mut out = String::new();
    out.push_str(&format!("report: {}\n", view.report_id));
    out.push_str(&format!(
        "window: {} to {} (since {})\n",
        view.start_date, view.end_date, view.since
    ));
    out.push_str(&format!("total: {}\n", format_number(view.total)));
    match view.average {
        Some(avg) => out.push_str(&format!("average: {}\n", format_number(avg))),
        None => out.push_str("average: —\n"),
    }
    out.push('\n');
    render_points(&view.data, 0, &mut out);
    out
}

/// Discourse reports come in two shapes: a flat series of `{x, y}` points,
/// or a stacked chart whose entries each carry their own nested `data`
/// series (see `AdminReport`'s doc comment in `src/api/reports.rs`). Walk
/// both, indenting nested series under their label.
fn render_points(data: &Value, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    if let Value::Array(items) = data {
        for item in items {
            if let Some(inner) = item.get("data") {
                let label = item
                    .get("label")
                    .and_then(|l| l.as_str())
                    .unwrap_or("series");
                out.push_str(&format!("{indent}[{label}]\n"));
                render_points(inner, depth + 1, out);
            } else if let Some(x) = item.get("x") {
                let x = x.as_str().unwrap_or_default();
                let y = item
                    .get("y")
                    .map(format_json_number)
                    .unwrap_or_else(|| "—".to_string());
                out.push_str(&format!("{indent}{x}  {y}\n"));
            }
        }
    }
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{:.2}", n)
    }
}

fn format_json_number(v: &Value) -> String {
    match v {
        Value::Number(n) => n
            .as_f64()
            .map(format_number)
            .unwrap_or_else(|| n.to_string()),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "—".to_string(),
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_text_flat_report() {
        let view = ReportView {
            report_id: "signups".into(),
            since: "30d".into(),
            start_date: "2026-08-17".into(),
            end_date: "2026-09-16".into(),
            total: 8.0,
            average: None,
            higher_is_better: Some(true),
            data: serde_json::json!([
                {"x": "2026-09-01", "y": 3},
                {"x": "2026-09-02", "y": 5}
            ]),
        };
        let text = render_text(&view);
        assert!(text.contains("report: signups"));
        assert!(text.contains("total: 8"));
        assert!(text.contains("average: —"));
        assert!(text.contains("2026-09-01  3"));
        assert!(text.contains("2026-09-02  5"));
    }

    #[test]
    fn render_text_stacked_report_indents_series() {
        let view = ReportView {
            report_id: "trust_level_growth".into(),
            since: "30d".into(),
            start_date: "2026-08-17".into(),
            end_date: "2026-09-16".into(),
            total: 3.0,
            average: None,
            higher_is_better: None,
            data: serde_json::json!([
                {"label": "TL1", "data": [{"x": "2026-09-01", "y": 2}]},
                {"label": "TL2", "data": [{"x": "2026-09-01", "y": 1}]}
            ]),
        };
        let text = render_text(&view);
        assert!(text.contains("[TL1]"));
        assert!(text.contains("[TL2]"));
        assert!(text.contains("  2026-09-01  2"));
    }

    #[test]
    fn render_text_shows_average_when_present() {
        let view = ReportView {
            report_id: "time_to_first_response".into(),
            since: "7d".into(),
            start_date: "2026-09-09".into(),
            end_date: "2026-09-16".into(),
            total: 0.0,
            average: Some(4.5),
            higher_is_better: Some(false),
            data: serde_json::json!([]),
        };
        let text = render_text(&view);
        assert!(text.contains("average: 4.50"));
    }
}

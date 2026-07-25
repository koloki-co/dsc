// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Wrapper around `/admin/reports/{report_id}.json`.
//!
//! Each Discourse report, when called with `start_date` + `end_date`, returns
//! both the requested window (`data`) AND the immediately preceding window
//! of equal length (`prev_data`) in a single response — so `--compare` does
//! not require a second API call for metrics that come straight from a
//! report.

use super::client::DiscourseClient;
use super::error::http_error;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One day-bucket from a flat (non-stacked) report's `data` array.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ReportPoint {
    #[serde(default)]
    pub x: String,
    #[serde(default)]
    pub y: f64,
}

/// Discourse's `average` emits as a number, `false`, or null depending on
/// whether the report has a meaningful average. Coerce false/null/missing
/// to `None`.
fn deserialize_lenient_optional_f64<'de, D>(de: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(de)?;
    match v {
        serde_json::Value::Number(n) => Ok(n.as_f64()),
        serde_json::Value::String(s) => Ok(s.parse::<f64>().ok()),
        _ => Ok(None),
    }
}

/// Raw report payload, distilled from `/admin/reports/{id}.json`. Only the
/// fields `dsc analytics` actually reads are deserialised — Discourse
/// emits a lot more (axis labels, chart modes, descriptions) that we don't
/// care about.
///
/// `data` and `prev_data` are kept as raw `serde_json::Value` because
/// Discourse uses two different shapes:
///
/// 1. **Flat counter reports** (`signups`, `topics`, `posts`, `likes`,
///    `flags`, `new_contributors`): `data: [{x: date, y: number}, ...]`.
/// 2. **Stacked-chart reports** (`trust_level_growth`):
///    `data: [{req: "tl1_reached", label: "...", data: [{x, y}, ...]}, ...]`.
///
/// `current_total()` walks both shapes and returns the right sum.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AdminReport {
    #[serde(default, alias = "type")]
    pub report_type: String,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub prev_data: Option<Value>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub prev_start_date: Option<String>,
    #[serde(default)]
    pub prev_end_date: Option<String>,
    /// Some reports (e.g. `time_to_first_response`) emit an `average`
    /// scalar that is more meaningful than summing the daily points.
    /// Discourse occasionally emits this as `false` when no average is
    /// computable; we coerce that to None.
    #[serde(default, deserialize_with = "deserialize_lenient_optional_f64")]
    pub average: Option<f64>,
    /// Whether higher-is-better — Discourse marks this on each report, and
    /// we use it to set the default `desirable` direction when our spec
    /// doesn't already pin one.
    #[serde(default)]
    pub higher_is_better: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ReportEnvelope {
    report: AdminReport,
}

impl AdminReport {
    /// Total for the current window.
    ///
    /// Walks the `data` value and sums every `y` it can find. Handles both
    /// the flat counter shape (`[{x, y}, ...]`) and the stacked-chart shape
    /// (`[{data: [{x, y}, ...]}, ...]`). Coerces non-numeric `y` (Discourse
    /// occasionally emits `false`/null/string for empty cells) to 0.
    pub fn current_total(&self) -> f64 {
        sum_data(&self.data)
    }

    /// Total for the previous-window, when present.
    pub fn previous_total(&self) -> Option<f64> {
        self.prev_data.as_ref().map(sum_data)
    }
}

fn sum_data(v: &Value) -> f64 {
    match v {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                // Stacked-chart wrapper: {req, label, color, data: [{x, y}, ...]}
                // → recurse into the inner data array.
                if let Some(inner) = item.get("data") {
                    sum_data(inner)
                } else if let Some(y) = item.get("y") {
                    coerce_f64(y)
                } else {
                    0.0
                }
            })
            .sum(),
        _ => 0.0,
    }
}

fn coerce_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.parse().unwrap_or(0.0),
        Value::Bool(_) | Value::Null => 0.0,
        _ => 0.0,
    }
}

impl DiscourseClient {
    /// Fetch a single admin report. `start` and `end` are ISO-8601 date
    /// strings (YYYY-MM-DD) — the format Discourse's report controller
    /// expects.
    pub fn fetch_admin_report(
        &self,
        report_id: &str,
        start: &str,
        end: &str,
    ) -> Result<AdminReport> {
        // The report id is taken straight from the spec list; sanity-check
        // it matches the same regex Discourse enforces server-side.
        if !report_id_is_valid(report_id) {
            return Err(anyhow::anyhow!(
                "invalid report id {:?} — must match /^[a-z0-9_]+$/",
                report_id
            ));
        }
        let path = format!(
            "/admin/reports/{}.json?start_date={}&end_date={}",
            report_id, start, end
        );
        let response = self.get(&path)?;
        let status = response.status();
        let text = response.text().context("reading admin report response")?;
        if !status.is_success() {
            return Err(http_error(
                &format!("admin report {} request", report_id),
                status,
                &text,
            ));
        }
        let env: ReportEnvelope = serde_json::from_str(&text)
            .with_context(|| format!("parsing admin report {} response", report_id))?;
        Ok(env.report)
    }
}

fn report_id_is_valid(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_id_validation() {
        assert!(report_id_is_valid("signups"));
        assert!(report_id_is_valid("time_to_first_response"));
        assert!(!report_id_is_valid(""));
        assert!(!report_id_is_valid("Signups"));
        assert!(!report_id_is_valid("../../../../etc/passwd"));
        assert!(!report_id_is_valid("topics; rm -rf"));
    }

    fn parse_report(json: &str) -> AdminReport {
        let envelope: serde_json::Value = serde_json::from_str(json).unwrap();
        let report = envelope.get("report").cloned().unwrap_or(envelope);
        serde_json::from_value(report).unwrap()
    }

    #[test]
    fn current_total_sums_flat_data() {
        let r = parse_report(
            r#"{
              "type": "signups",
              "data": [{"x":"2026-04-01","y":3},{"x":"2026-04-02","y":5},{"x":"2026-04-03","y":0}]
            }"#,
        );
        assert_eq!(r.current_total(), 8.0);
        assert_eq!(r.previous_total(), None);
    }

    #[test]
    fn previous_total_when_prev_data_present() {
        let r = parse_report(
            r#"{
              "type": "posts",
              "data": [{"x":"2026-04-01","y":10}],
              "prev_data": [{"x":"2026-03-01","y":4},{"x":"2026-03-02","y":6}]
            }"#,
        );
        assert_eq!(r.current_total(), 10.0);
        assert_eq!(r.previous_total(), Some(10.0));
    }

    #[test]
    fn current_total_handles_stacked_chart() {
        // trust_level_growth shape: data is an array of series, each with
        // its own inner `data: [{x, y}, ...]`. Total is sum across all.
        let r = parse_report(
            r#"{
              "type": "trust_level_growth",
              "data": [
                {"req": "tl1_reached", "label": "TL1", "data": [{"x":"2026-04-01","y":2},{"x":"2026-04-02","y":3}]},
                {"req": "tl2_reached", "label": "TL2", "data": [{"x":"2026-04-01","y":1}]},
                {"req": "tl3_reached", "label": "TL3", "data": []},
                {"req": "tl4_reached", "label": "TL4", "data": [{"x":"2026-04-02","y":1}]}
              ]
            }"#,
        );
        assert_eq!(r.current_total(), 7.0);
    }

    #[test]
    fn current_total_zero_for_non_array_data() {
        // Discourse occasionally emits `data: false` or `data: null` for
        // genuinely-empty results (no permission, no data, etc.).
        let r = parse_report(r#"{"type": "x", "data": false}"#);
        assert_eq!(r.current_total(), 0.0);
        let r = parse_report(r#"{"type": "x", "data": null}"#);
        assert_eq!(r.current_total(), 0.0);
    }

    #[test]
    fn current_total_coerces_non_numeric_y() {
        let r = parse_report(
            r#"{
              "type": "x",
              "data": [{"x":"2026-04-01","y":false},{"x":"2026-04-02","y":"5"},{"x":"2026-04-03","y":null}]
            }"#,
        );
        assert_eq!(r.current_total(), 5.0);
    }
}

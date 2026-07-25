// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use reqwest::header::HeaderMap;
use serde_json::Value;
use std::time::Duration;

pub(crate) const DEFAULT_BACKOFF: Duration = Duration::from_secs(5);
pub(crate) const RETRY_BUFFER: Duration = Duration::from_secs(1);

pub(crate) fn parse_rate_limit_wait(headers: &HeaderMap, body: &str) -> Duration {
    if let Some(val) = headers.get(reqwest::header::RETRY_AFTER)
        && let Ok(s) = val.to_str()
        && let Ok(secs) = s.trim().parse::<u64>()
        && secs > 0
    {
        return Duration::from_secs(secs);
    }
    if let Ok(val) = serde_json::from_str::<Value>(body)
        && let Some(secs) = val
            .get("extras")
            .and_then(|e| e.get("wait_seconds"))
            .and_then(|w| w.as_u64())
        && secs > 0
    {
        return Duration::from_secs(secs);
    }
    if let Some(secs) = extract_retry_seconds_from_text(body) {
        return Duration::from_secs(secs);
    }
    DEFAULT_BACKOFF
}

pub(crate) fn extract_retry_seconds_from_text(body: &str) -> Option<u64> {
    let lower = body.to_ascii_lowercase();
    for needle in ["retry again in ", "retry in ", "wait "] {
        if let Some(pos) = lower.find(needle) {
            let tail = &lower[pos + needle.len()..];
            let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(secs) = digits.parse::<u64>()
                && secs > 0
            {
                return Some(secs);
            }
        }
    }
    None
}

pub(crate) fn summarize_rate_limit_body(body: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(body)
        && let Some(errs) = val.get("errors").and_then(|e| e.as_array())
    {
        let joined: Vec<String> = errs
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        if !joined.is_empty() {
            return joined.join("; ");
        }
    }
    let first_line = body.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        "429 Too Many Requests".to_string()
    } else {
        first_line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_retry_seconds_from_text, parse_rate_limit_wait, summarize_rate_limit_body,
    };
    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
    use std::time::Duration;

    #[test]
    fn parses_retry_after_header() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("7"));
        assert_eq!(parse_rate_limit_wait(&headers, ""), Duration::from_secs(7));
    }

    #[test]
    fn parses_extras_wait_seconds_from_body() {
        let body = r#"{"errors":["Slow down"],"extras":{"wait_seconds":4}}"#;
        assert_eq!(
            parse_rate_limit_wait(&HeaderMap::new(), body),
            Duration::from_secs(4)
        );
    }

    #[test]
    fn parses_retry_seconds_from_text_body() {
        let body = "Slow down, you're making too many requests. Please retry again in 4 seconds. Error code: ip_10_secs_limit.";
        assert_eq!(extract_retry_seconds_from_text(body), Some(4));
    }

    #[test]
    fn falls_back_to_default_when_nothing_parseable() {
        let body = "<html><body>429 Too Many Requests</body></html>";
        assert_eq!(
            parse_rate_limit_wait(&HeaderMap::new(), body),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn summarizes_json_errors() {
        let body = r#"{"errors":["Slow down, too many requests."]}"#;
        assert_eq!(
            summarize_rate_limit_body(body),
            "Slow down, too many requests."
        );
    }

    #[test]
    fn summarizes_html_body() {
        let body = "<html><head><title>429 Too Many Requests</title></head></html>";
        assert_eq!(
            summarize_rate_limit_body(body),
            "<html><head><title>429 Too Many Requests</title></head></html>"
        );
    }
}

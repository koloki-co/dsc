// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Functional coverage for `dsc report <discourse> <name>` against a mocked
//! `/admin/reports/{id}.json`. See `spec/commands/analytics.md`'s "Data
//! sources" section for the endpoint shape this reuses.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

/// A mock Discourse that answers any `/admin/reports/*.json` request with a
/// fixed payload, or a 404 (simulating an unknown/renamed report id).
fn start_mock(report_json: String, not_found: bool) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let requests = Arc::new(AtomicUsize::new(0));
    let thread_requests = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &report_json, not_found, &thread_requests);
        }
    });
    (format!("http://{addr}"), requests)
}

fn handle(mut stream: TcpStream, report_json: &str, not_found: bool, requests: &AtomicUsize) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }

    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();

    let (status_line, body) = if path.starts_with("/admin/reports/") {
        requests.fetch_add(1, Ordering::Relaxed);
        if not_found {
            (
                "HTTP/1.1 404 Not Found",
                "{\"errors\":[\"not found\"]}".to_string(),
            )
        } else {
            ("HTTP/1.1 200 OK", report_json.to_string())
        }
    } else {
        ("HTTP/1.1 404 Not Found", "{}".to_string())
    };
    let response = format!(
        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn flat_signups_report() -> String {
    r#"{"report":{"type":"signups","data":[{"x":"2026-09-01","y":3},{"x":"2026-09-02","y":5}],"start_date":"2026-08-17","end_date":"2026-09-16","higher_is_better":true}}"#.to_string()
}

fn stacked_trust_level_report() -> String {
    r#"{"report":{"type":"trust_level_growth","data":[
        {"req":"tl1_reached","label":"TL1","data":[{"x":"2026-09-01","y":2}]},
        {"req":"tl2_reached","label":"TL2","data":[{"x":"2026-09-01","y":1}]}
    ]}}"#
        .to_string()
}

fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    )
}

#[test]
fn report_prints_flat_report_as_text_and_json() {
    let (url, requests) = start_mock(flat_signups_report(), false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["report", "alpha", "signups", "--since", "30d"], &config);
    assert!(
        output.status.success(),
        "report failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("report: signups"));
    assert!(text.contains("total: 8"));
    assert!(text.contains("2026-09-01  3"));
    assert!(text.contains("2026-09-02  5"));
    assert_eq!(requests.load(Ordering::Relaxed), 1);

    let json_output = run_dsc(&["report", "alpha", "signups", "--format", "json"], &config);
    assert!(json_output.status.success());
    let view: serde_json::Value = serde_json::from_slice(&json_output.stdout).expect("report JSON");
    assert_eq!(view["report_id"], "signups");
    assert_eq!(view["total"], 8.0);
    assert_eq!(view["higher_is_better"], true);
    assert_eq!(view["data"][0]["x"], "2026-09-01");
}

#[test]
fn report_indents_stacked_series_in_text_output() {
    let (url, _) = start_mock(stacked_trust_level_report(), false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["report", "alpha", "trust_level_growth"], &config);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("[TL1]"));
    assert!(text.contains("[TL2]"));
    assert!(text.contains("total: 3"));
}

#[test]
fn report_surfaces_a_clear_error_for_an_unknown_report_id() {
    let (url, _) = start_mock(flat_signups_report(), true);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["report", "alpha", "not_a_real_report"], &config);
    assert!(!output.status.success());
}

#[test]
fn report_rejects_a_malformed_report_id_before_any_request() {
    let (url, requests) = start_mock(flat_signups_report(), false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["report", "alpha", "../../etc/passwd"], &config);
    assert!(!output.status.success());
    assert_eq!(
        requests.load(Ordering::Relaxed),
        0,
        "invalid report id must be rejected client-side before any HTTP call"
    );
}

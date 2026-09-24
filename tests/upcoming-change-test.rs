// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Functional coverage for `dsc upcoming-change list|show` against a mocked
//! `/admin/config/upcoming-changes.json` endpoint. See
//! `spec/commands/upcoming-changes-and-setting-upload.md` for the API shapes
//! captured against Discourse source.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tempfile::TempDir;

/// A mock Discourse that answers `/admin/config/upcoming-changes.json` with a
/// fixed payload, or a 404 when `not_found` is set (simulating an older
/// Discourse version without the endpoint). Records whether every request
/// carried the required `X-Requested-With: XMLHttpRequest` header.
fn start_mock(list_json: String, not_found: bool) -> (String, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let requests = Arc::new(AtomicUsize::new(0));
    let saw_xhr_header = Arc::new(AtomicBool::new(true));
    let thread_requests = Arc::clone(&requests);
    let thread_saw_xhr_header = Arc::clone(&saw_xhr_header);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(
                stream,
                &list_json,
                not_found,
                &thread_requests,
                &thread_saw_xhr_header,
            );
        }
    });
    (format!("http://{addr}"), requests, saw_xhr_header)
}

fn handle(
    mut stream: TcpStream,
    list_json: &str,
    not_found: bool,
    requests: &AtomicUsize,
    saw_xhr_header: &AtomicBool,
) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
    let mut content_length = 0usize;
    let mut has_xhr_header = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("x-requested-with:") && lower.contains("xmlhttprequest") {
            has_xhr_header = true;
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

    let (status_line, body) = if not_found {
        (
            "HTTP/1.1 404 Not Found",
            "{\"errors\":[\"not found\"]}".to_string(),
        )
    } else if path.starts_with("/admin/config/upcoming-changes.json") {
        requests.fetch_add(1, Ordering::Relaxed);
        if !has_xhr_header {
            saw_xhr_header.store(false, Ordering::Relaxed);
        }
        ("HTTP/1.1 200 OK", list_json.to_string())
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

fn sample_list() -> String {
    r#"{"upcoming_changes":[
        {
            "setting": "enable_generated_llms_txt",
            "humanized_name": "Enable generated llms txt",
            "description": "Generates a concise /llms.txt when no custom file is uploaded.",
            "value": true,
            "upcoming_change": {
                "status": "beta",
                "impact": "feature,all_members",
                "impact_type": "feature",
                "impact_role": "all_members",
                "enabled_for": "everyone"
            },
            "plugin": null,
            "depends_on": null,
            "depends_on_humanized_names": null,
            "dependents": [],
            "depends_on_met": true,
            "overriding_defaults": true,
            "groups": null
        },
        {
            "setting": "some_other_change",
            "value": false,
            "upcoming_change": {"status": "experimental", "enabled_for": "staff"},
            "depends_on_met": false
        }
    ]}"#
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
fn list_prints_every_upcoming_change_and_sends_the_xhr_header() {
    let (url, requests, saw_xhr_header) = start_mock(sample_list(), false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(
        &["upcoming-change", "list", "alpha", "--format", "json"],
        &config,
    );
    assert!(
        output.status.success(),
        "upcoming-change list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let changes: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("upcoming-change list JSON");
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0]["setting"], "enable_generated_llms_txt");
    assert_eq!(changes[0]["value"], true);

    let text_output = run_dsc(&["upcoming-change", "list", "alpha"], &config);
    assert!(text_output.status.success());
    let text = String::from_utf8_lossy(&text_output.stdout);
    assert!(text.contains("enable_generated_llms_txt"));
    assert!(text.contains("some_other_change"));

    assert_eq!(requests.load(Ordering::Relaxed), 2);
    assert!(
        saw_xhr_header.load(Ordering::Relaxed),
        "list must send X-Requested-With: XMLHttpRequest"
    );
}

#[test]
fn show_selects_the_exact_named_change() {
    let (url, _, _) = start_mock(sample_list(), false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(
        &[
            "upcoming-change",
            "show",
            "alpha",
            "enable_generated_llms_txt",
        ],
        &config,
    );
    assert!(
        output.status.success(),
        "upcoming-change show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("enable_generated_llms_txt"));
    assert!(text.contains("beta"));
    assert!(text.contains("everyone"));

    let json_output = run_dsc(
        &[
            "upcoming-change",
            "show",
            "alpha",
            "enable_generated_llms_txt",
            "--format",
            "json",
        ],
        &config,
    );
    assert!(json_output.status.success());
    let detail: serde_json::Value =
        serde_json::from_slice(&json_output.stdout).expect("upcoming-change show JSON");
    assert_eq!(detail["setting"], "enable_generated_llms_txt");
    assert_eq!(detail["depends_on_met"], true);
}

#[test]
fn show_fails_clearly_for_an_absent_setting_name() {
    let (url, _, _) = start_mock(sample_list(), false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(
        &["upcoming-change", "show", "alpha", "does_not_exist"],
        &config,
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does_not_exist"),
        "expected the absent name in the error: {stderr}"
    );
}

#[test]
fn commands_report_a_clear_error_when_the_endpoint_is_unavailable() {
    let (url, _, _) = start_mock(sample_list(), true);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["upcoming-change", "list", "alpha"], &config);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Upcoming Changes"),
        "expected an Upcoming Changes-specific 404 hint, got: {stderr}"
    );
}

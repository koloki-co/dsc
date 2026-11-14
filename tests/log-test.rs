// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn log_staff_returns_a_json_array_without_mutating_the_forum() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_log_staff: fetch one staff action log entry as JSON");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );

    let output = run_dsc(
        &[
            "log", "staff", &test.name, "--limit", "1", "--format", "json",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "log staff failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("log staff --format json must emit JSON");
    assert!(parsed.is_array(), "expected JSON array, got: {stdout}");
}

/// Minimal mock Discourse recording every request path and answering the
/// staff action log endpoint with an empty page.
fn start_staff_log_mock() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let paths = Arc::new(Mutex::new(Vec::new()));
    let thread_paths = Arc::clone(&paths);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line == "\r\n" || line == "\n" {
                    break;
                }
            }
            let path = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
            thread_paths.lock().unwrap().push(path);
            let body = r#"{"staff_action_logs":[],"total_rows_staff_action_logs":0}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        }
    });
    (format!("http://{addr}"), paths)
}

fn mock_config(dir: &TempDir, baseurl: &str) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"k\"\napi_username = \"system\"\n"
        ),
    )
}

#[test]
fn log_staff_until_sends_an_inclusive_end_date_alongside_start_date() {
    let (baseurl, paths) = start_staff_log_mock();
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &baseurl);

    let output = run_dsc(
        &[
            "log",
            "staff",
            "mock",
            "--acting-user",
            "alice",
            "--since",
            "2026-10-01",
            "--until",
            "2026-10-31T23:59:59+01:00",
            "-f",
            "json",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "log staff failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let paths = paths.lock().unwrap();
    assert_eq!(paths.len(), 1, "expected exactly one request: {paths:?}");
    assert_eq!(
        paths[0],
        "/admin/logs/staff_action_logs.json?limit=50&acting_user=alice&start_date=2026-10-01T00%3A00%3A00Z&end_date=2026-10-31T22%3A59%3A59Z"
    );
}

#[test]
fn log_staff_rejects_reversed_or_imprecise_windows_without_requesting() {
    let (baseurl, paths) = start_staff_log_mock();
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &baseurl);

    for args in [
        &["--since", "2026-11-02", "--until", "2026-11-01T00:00:00Z"][..],
        &["--until", "2026-11-01"][..],
        &["--until", "7d"][..],
    ] {
        let mut full = vec!["log", "staff", "mock"];
        full.extend_from_slice(args);
        let output = run_dsc(&full, &config_path);
        assert!(
            !output.status.success(),
            "expected failure for {args:?}, stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    assert!(
        paths.lock().unwrap().is_empty(),
        "invalid windows must not reach the forum"
    );
}

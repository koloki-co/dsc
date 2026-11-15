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

/// Mock answering `acting_user=bob` and `target_user=bob` with overlapping
/// pages (entry 5 is a self-action present in both), recording every path.
fn start_user_union_mock() -> (String, Arc<Mutex<Vec<String>>>) {
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
            let row = |id: u64, day: u8, actor: &str, target: &str| {
                format!(
                    r#"{{"id":{id},"action_name":"delete_post","acting_user":{{"id":1,"username":"{actor}"}},"target_user":{target},"created_at":"2026-11-0{day}T00:00:00.000Z"}}"#
                )
            };
            let bob = r#"{"id":2,"username":"bob"}"#;
            let rows = if path.contains("acting_user=bob") {
                vec![row(5, 5, "bob", bob), row(2, 2, "bob", "null")]
            } else if path.contains("target_user=bob") {
                vec![
                    row(6, 6, "alice", bob),
                    row(5, 5, "bob", bob),
                    row(1, 1, "alice", bob),
                ]
            } else {
                Vec::new()
            };
            thread_paths.lock().unwrap().push(path);
            let body = format!(r#"{{"staff_action_logs":[{}]}}"#, rows.join(","));
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

#[test]
fn log_staff_user_unions_actor_and_target_streams_with_shared_filters() {
    let (baseurl, paths) = start_user_union_mock();
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &baseurl);

    let output = run_dsc(
        &[
            "log",
            "staff",
            "mock",
            "--user",
            "bob",
            "--action",
            "delete_post",
            "--since",
            "2026-11-01",
            "--until",
            "2026-11-30T00:00:00Z",
            "--limit",
            "3",
            "-f",
            "json",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "log staff --user failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    let ids: Vec<u64> = parsed
        .as_array()
        .expect("array")
        .iter()
        .map(|entry| entry["id"].as_u64().unwrap())
        .collect();
    assert_eq!(
        ids,
        [6, 5, 2],
        "deduplicated, newest first, truncated to limit"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("older entries may exist"),
        "a full target stream must warn of possible truncation: {stderr}"
    );

    let paths = paths.lock().unwrap();
    let window = "action_name=delete_post";
    let dates = "start_date=2026-11-01T00%3A00%3A00Z&end_date=2026-11-30T00%3A00%3A00Z";
    assert_eq!(
        *paths,
        [
            format!("/admin/logs/staff_action_logs.json?limit=3&{window}&acting_user=bob&{dates}"),
            format!("/admin/logs/staff_action_logs.json?limit=3&{window}&target_user=bob&{dates}"),
        ]
    );
}

#[test]
fn log_staff_user_conflicts_with_actor_and_target_filters() {
    let (baseurl, paths) = start_staff_log_mock();
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &baseurl);

    for flag in ["--acting-user", "--target-user"] {
        let output = run_dsc(
            &["log", "staff", "mock", "--user", "bob", flag, "alice"],
            &config_path,
        );
        assert!(
            !output.status.success(),
            "--user with {flag} must be refused"
        );
    }
    assert!(paths.lock().unwrap().is_empty(), "no request may be made");
}

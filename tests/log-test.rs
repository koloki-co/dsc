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

/// Mock answering every staff-log request with two rows: one carrying context,
/// linkage IDs and the personal-data fields `email`/`ip_address`, and one with
/// no linkage at all.
fn start_context_mock() -> (String, Arc<Mutex<Vec<String>>>) {
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
            let body = r#"{"staff_action_logs":[
                {"id":9,"action_name":"delete_post","action":12,"acting_user":{"id":1,"username":"alice"},
                 "target_user":{"id":2,"username":"bob"},"subject":null,"details":"post deleted",
                 "context":"user deletion cascade","topic_id":42,"post_id":420,"category_id":null,
                 "reviewable_id":null,"custom_type":null,"expires_at":null,
                 "email":"bob@example.invalid","ip_address":"192.0.2.1",
                 "created_at":"2026-11-02T00:00:00.000Z"},
                {"id":8,"action_name":"change_site_setting","acting_user":{"id":1,"username":"alice"},
                 "subject":"title","created_at":"2026-11-01T00:00:00.000Z"}
            ]}"#;
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

const LINKAGE_KEYS: &[&str] = &[
    "context",
    "topic_id",
    "post_id",
    "category_id",
    "reviewable_id",
    "action",
    "custom_type",
    "expires_at",
];

#[test]
fn log_staff_default_json_omits_context_and_personal_data() {
    let (baseurl, paths) = start_context_mock();
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &baseurl);

    let output = run_dsc(&["log", "staff", "mock", "-f", "json"], &config_path);
    assert!(
        output.status.success(),
        "log staff failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    let row = parsed[0].as_object().expect("object row");
    let mut keys: Vec<&str> = row.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "acting_user",
            "action_name",
            "created_at",
            "details",
            "id",
            "new_value",
            "previous_value",
            "subject",
            "target_user"
        ],
        "default output shape must be unchanged"
    );
    assert_eq!(paths.lock().unwrap().len(), 1, "one GET only");
}

#[test]
fn log_staff_include_context_adds_linkage_but_never_email_or_ip() {
    let (baseurl, paths) = start_context_mock();
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &baseurl);

    for format in ["json", "yaml"] {
        let output = run_dsc(
            &["log", "staff", "mock", "--include-context", "-f", format],
            &config_path,
        );
        assert!(
            output.status.success(),
            "log staff --include-context failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("example.invalid") && !stdout.contains("192.0.2.1"),
            "email/IP must never be emitted ({format}): {stdout}"
        );
        if format != "json" {
            assert!(
                stdout.contains("context: user deletion cascade"),
                "{stdout}"
            );
            continue;
        }
        let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON output");
        let rows = parsed.as_array().expect("array");
        assert_eq!(rows[0]["context"], "user deletion cascade");
        assert_eq!(rows[0]["topic_id"], 42);
        assert_eq!(rows[0]["post_id"], 420);
        assert_eq!(rows[0]["action"], 12);
        assert_eq!(rows[0]["id"], 9, "default fields are retained");
        for key in LINKAGE_KEYS {
            assert!(
                rows[1].get(*key).is_some_and(serde_json::Value::is_null),
                "absent {key} must be an explicit null, not dropped: {}",
                rows[1]
            );
        }
        for row in rows {
            assert!(row.get("email").is_none() && row.get("ip_address").is_none());
        }
    }

    let text = run_dsc(&["log", "staff", "mock", "--include-context"], &config_path);
    assert!(text.status.success());
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(stdout.contains("[topic=42 post=420]"), "{stdout}");
    assert!(
        stdout.contains("    context: user deletion cascade"),
        "{stdout}"
    );
    assert!(!stdout.contains("192.0.2.1"), "{stdout}");
    assert_eq!(paths.lock().unwrap().len(), 3, "one GET per invocation");
}

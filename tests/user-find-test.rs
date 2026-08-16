// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Coverage for `dsc user find` - the GDPR "which forum has this person"
//! fan-out lookup across every configured forum.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use tempfile::TempDir;

/// A mock Discourse that answers every `/admin/users/list/all.json` request
/// with a fixed user-search payload (or, if `status` is not 200, an error
/// status).
fn start_mock(status: u16, users_json: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, status, &users_json);
        }
    });
    format!("http://{addr}")
}

fn handle(mut stream: TcpStream, status: u16, users_json: &str) {
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

    let (status_line, body) = if status == 200 {
        ("HTTP/1.1 200 OK", format!("[{users_json}]"))
    } else {
        (
            "HTTP/1.1 503 Service Unavailable",
            r#"{"error":"unavailable"}"#.to_string(),
        )
    };
    let response = format!(
        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn user(id: i64, username: &str, email: &str) -> String {
    format!(r#"{{"id":{id},"username":"{username}","email":"{email}"}}"#)
}

#[test]
fn find_matches_by_exact_email_across_every_forum() {
    let alpha_url = start_mock(200, user(42, "jane_d", "jane@example.com"));
    let beta_url = start_mock(200, user(7, "unrelated", "someone-else@example.com"));

    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n\n[[discourse]]\nname = \"beta\"\nbaseurl = \"{beta_url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );

    let output = run_dsc(
        &["user", "find", "jane@example.com", "--format", "json"],
        &config,
    );
    assert!(
        output.status.success(),
        "user find failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("user find JSON");
    assert_eq!(
        rows.len(),
        1,
        "only the exact email match should be reported: {rows:?}"
    );
    assert_eq!(rows[0]["forum"], "alpha");
    assert_eq!(rows[0]["username"], "jane_d");
    assert_eq!(rows[0]["id"], 42);

    let text_output = run_dsc(&["user", "find", "jane@example.com"], &config);
    assert!(text_output.status.success());
    let text = String::from_utf8_lossy(&text_output.stdout);
    assert!(text.contains("alpha") && text.contains("jane_d"));
    assert!(!text.contains("unrelated"));
}

#[test]
fn find_reports_no_match_when_nobody_has_the_address() {
    let alpha_url = start_mock(200, user(1, "someone", "someone@example.com"));

    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );

    let output = run_dsc(&["user", "find", "missing@example.com"], &config);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("No account found"));
}

#[test]
fn find_reports_per_forum_failures_without_losing_other_matches() {
    let alpha_url = start_mock(200, user(42, "jane_d", "jane@example.com"));
    let down_url = start_mock(503, String::new());

    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n\n[[discourse]]\nname = \"down\"\nbaseurl = \"{down_url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );

    let output = run_dsc(
        &["user", "find", "jane@example.com", "--format", "json"],
        &config,
    );
    assert!(
        !output.status.success(),
        "user find should fail overall when one forum errors"
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("user find JSON");
    assert_eq!(rows.len(), 1, "the healthy forum's match should survive");
    assert_eq!(rows[0]["forum"], "alpha");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("down"),
        "stderr should name the failing forum: {stderr}"
    );
}

#[test]
fn find_rejects_a_value_without_an_at_sign() {
    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"alpha\"\nbaseurl = \"http://127.0.0.1:1\"\napikey = \"k\"\napi_username = \"tester\"\n",
    );

    let output = run_dsc(&["user", "find", "not-an-email"], &config);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid email"), "stderr: {stderr}");
}

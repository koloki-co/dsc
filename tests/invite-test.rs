// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Success-path coverage for `dsc invite send`/`invite bulk`. The shared
//! `--dry-run` no-mutation registry already proves these never hit the
//! server under `--dry-run`; these tests prove the real request each one
//! sends encodes `--group`/`--topic`/`--message` correctly, since a silent
//! encoding bug here would otherwise go undetected.

mod common;
use common::*;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

struct CapturedRequest {
    method: String,
    path: String,
    body: String,
}

struct MockResponse {
    status: &'static str,
    body: &'static str,
}

fn start_mock(responses: Vec<MockResponse>) -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_requests = Arc::clone(&requests);
    std::thread::spawn(move || {
        let mut responses = VecDeque::from(responses);
        for stream in listener.incoming().flatten() {
            let Some(response) = responses.pop_front() else {
                break;
            };
            handle_request(stream, response, &thread_requests);
        }
    });
    (format!("http://{addr}"), requests)
}

fn handle_request(
    mut stream: TcpStream,
    response: MockResponse,
    requests: &Mutex<Vec<CapturedRequest>>,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("request header");
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body_bytes = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body_bytes).expect("request body");
    }
    let body = String::from_utf8_lossy(&body_bytes).into_owned();

    requests
        .lock()
        .expect("request log")
        .push(CapturedRequest { method, path, body });

    let raw = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.status,
        response.body.len(),
        response.body
    );
    stream.write_all(raw.as_bytes()).expect("mock response");
}

fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{url}\"\napikey = \"key\"\napi_username = \"system\"\n"
        ),
    )
}

/// Decode one `application/x-www-form-urlencoded` body into a lookup map.
fn decode_form(body: &str) -> std::collections::HashMap<String, String> {
    body.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode_form_component(key), decode_form_component(value))
        })
        .collect()
}

fn decode_form_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn invite_send_posts_groups_topic_and_message() {
    let (url, requests) = start_mock(vec![MockResponse {
        status: "200 OK",
        body: r#"{"invite":{"id":7,"link":"https://example.test/invites/abc","email":"alice@example.com"}}"#,
    }]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &[
            "invite",
            "send",
            "mock",
            "alice@example.com",
            "--group",
            "3",
            "--group",
            "5",
            "--topic",
            "42",
            "--message",
            "Welcome!",
        ],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Invited alice@example.com"));

    let seen = requests.lock().expect("request log");
    let create = seen
        .iter()
        .find(|r| r.method == "POST" && r.path == "/invites.json")
        .expect("expected a POST /invites.json request");
    let form = decode_form(&create.body);
    assert_eq!(
        form.get("email").map(String::as_str),
        Some("alice@example.com")
    );
    assert_eq!(form.get("group_ids").map(String::as_str), Some("3,5"));
    assert_eq!(form.get("topic_id").map(String::as_str), Some("42"));
    assert_eq!(
        form.get("custom_message").map(String::as_str),
        Some("Welcome!")
    );
}

#[test]
fn invite_send_rejects_an_invalid_email_before_any_request() {
    let (url, requests) = start_mock(vec![]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["invite", "send", "mock", "not-an-email"],
        &config_for(&url, &dir),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid email"));
    assert!(
        requests.lock().expect("request log").is_empty(),
        "an invalid email must not reach the server"
    );
}

#[test]
fn invite_bulk_invites_every_address_and_reports_the_summary() {
    let (url, requests) = start_mock(vec![
        MockResponse {
            status: "200 OK",
            body: r#"{"invite":{"id":1,"email":"alice@example.com"}}"#,
        },
        MockResponse {
            status: "200 OK",
            body: r#"{"invite":{"id":2,"email":"bob@example.com"}}"#,
        },
    ]);
    let dir = TempDir::new().expect("tempdir");
    let emails = dir.path().join("emails.txt");
    std::fs::write(&emails, "alice@example.com\nbob@example.com\n").expect("write emails");

    let output = run_dsc(
        &[
            "invite",
            "bulk",
            "mock",
            emails.to_str().unwrap(),
            "--group",
            "9",
        ],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("invited=2, failed=0"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let seen = requests.lock().expect("request log");
    let creates: Vec<_> = seen
        .iter()
        .filter(|r| r.method == "POST" && r.path == "/invites.json")
        .collect();
    assert_eq!(creates.len(), 2);
    for create in &creates {
        let form = decode_form(&create.body);
        assert_eq!(form.get("group_ids").map(String::as_str), Some("9"));
    }
    let emails_sent: std::collections::HashSet<_> = creates
        .iter()
        .map(|c| {
            decode_form(&c.body)
                .get("email")
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        emails_sent,
        std::collections::HashSet::from([
            "alice@example.com".to_string(),
            "bob@example.com".to_string()
        ])
    );
}

#[test]
fn invite_bulk_continues_past_a_failure_and_exits_non_zero() {
    let (url, requests) = start_mock(vec![
        MockResponse {
            status: "200 OK",
            body: r#"{"invite":{"id":1,"email":"alice@example.com"}}"#,
        },
        MockResponse {
            status: "422 Unprocessable Entity",
            body: r#"{"errors":["Email has already been used"]}"#,
        },
    ]);
    let dir = TempDir::new().expect("tempdir");
    let emails = dir.path().join("emails.txt");
    std::fs::write(&emails, "alice@example.com\nbob@example.com\n").expect("write emails");

    let output = run_dsc(
        &["invite", "bulk", "mock", emails.to_str().unwrap()],
        &config_for(&url, &dir),
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("invited=1, failed=1"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let seen = requests.lock().expect("request log");
    assert_eq!(
        seen.iter()
            .filter(|r| r.method == "POST" && r.path == "/invites.json")
            .count(),
        2,
        "both emails must be attempted even after the first failure"
    );
}

#[test]
fn invite_bulk_rejects_an_empty_input_before_any_request() {
    let (url, requests) = start_mock(vec![]);
    let dir = TempDir::new().expect("tempdir");
    let emails = dir.path().join("emails.txt");
    std::fs::write(&emails, "# just a comment\n\n").expect("write emails");

    let output = run_dsc(
        &["invite", "bulk", "mock", emails.to_str().unwrap()],
        &config_for(&url, &dir),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no email addresses found"));
    assert!(requests.lock().expect("request log").is_empty());
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Success-path coverage for `dsc user`'s moderation commands. The shared
//! `--dry-run` no-mutation registry already proves these never hit the
//! server under `--dry-run`; these tests prove the *real* request each one
//! sends is the one Discourse's admin API actually expects - exact verb,
//! path, and form body - since a silent body-encoding bug here would
//! otherwise go undetected against real community members.

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

/// Starts a mock Discourse that replies to each request in order with the
/// corresponding entry in `responses`, capturing method/path/body for
/// later assertion.
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

const USER_DETAIL_BODY: &str = r#"{"user":{"id":42,"username":"troublemaker"}}"#;

#[test]
fn user_suspend_sends_the_exact_until_and_reason() {
    let (url, requests) = start_mock(vec![
        MockResponse {
            status: "200 OK",
            body: USER_DETAIL_BODY,
        },
        MockResponse {
            status: "200 OK",
            body: "{}",
        },
    ]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &[
            "user",
            "suspend",
            "mock",
            "troublemaker",
            "--until",
            "2027-01-01T00:00:00Z",
            "--reason",
            "spam",
        ],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let seen = requests.lock().expect("request log");
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/u/troublemaker.json");
    assert_eq!(seen[1].method, "PUT");
    assert_eq!(seen[1].path, "/admin/users/42/suspend.json");
    let form = decode_form(&seen[1].body);
    assert_eq!(
        form.get("suspend_until").map(String::as_str),
        Some("2027-01-01T00:00:00Z")
    );
    assert_eq!(form.get("reason").map(String::as_str), Some("spam"));
}

#[test]
fn user_unsuspend_sends_no_body() {
    let (url, requests) = start_mock(vec![
        MockResponse {
            status: "200 OK",
            body: USER_DETAIL_BODY,
        },
        MockResponse {
            status: "200 OK",
            body: "{}",
        },
    ]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["user", "unsuspend", "mock", "troublemaker"],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let seen = requests.lock().expect("request log");
    assert_eq!(seen[1].method, "PUT");
    assert_eq!(seen[1].path, "/admin/users/42/unsuspend.json");
}

#[test]
fn user_silence_omits_empty_until_and_reason() {
    let (url, requests) = start_mock(vec![
        MockResponse {
            status: "200 OK",
            body: USER_DETAIL_BODY,
        },
        MockResponse {
            status: "200 OK",
            body: "{}",
        },
    ]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["user", "silence", "mock", "troublemaker"],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let seen = requests.lock().expect("request log");
    assert_eq!(seen[1].method, "PUT");
    assert_eq!(seen[1].path, "/admin/users/42/silence.json");
    assert!(
        seen[1].body.is_empty(),
        "no --until/--reason must send no form fields, got {:?}",
        seen[1].body
    );
}

#[test]
fn user_silence_sends_supplied_until_and_reason() {
    let (url, requests) = start_mock(vec![
        MockResponse {
            status: "200 OK",
            body: USER_DETAIL_BODY,
        },
        MockResponse {
            status: "200 OK",
            body: "{}",
        },
    ]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &[
            "user",
            "silence",
            "mock",
            "troublemaker",
            "--until",
            "2027-02-01T00:00:00Z",
            "--reason",
            "flooding",
        ],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let seen = requests.lock().expect("request log");
    let form = decode_form(&seen[1].body);
    assert_eq!(
        form.get("silenced_till").map(String::as_str),
        Some("2027-02-01T00:00:00Z")
    );
    assert_eq!(form.get("reason").map(String::as_str), Some("flooding"));
}

#[test]
fn user_promote_admin_hits_grant_admin() {
    let (url, requests) = start_mock(vec![
        MockResponse {
            status: "200 OK",
            body: USER_DETAIL_BODY,
        },
        MockResponse {
            status: "200 OK",
            body: "{}",
        },
    ]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["user", "promote", "mock", "troublemaker", "--role", "admin"],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let seen = requests.lock().expect("request log");
    assert_eq!(seen[1].method, "PUT");
    assert_eq!(seen[1].path, "/admin/users/42/grant_admin.json");
}

#[test]
fn user_demote_moderator_hits_revoke_moderation() {
    let (url, requests) = start_mock(vec![
        MockResponse {
            status: "200 OK",
            body: USER_DETAIL_BODY,
        },
        MockResponse {
            status: "200 OK",
            body: "{}",
        },
    ]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &[
            "user",
            "demote",
            "mock",
            "troublemaker",
            "--role",
            "moderator",
        ],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let seen = requests.lock().expect("request log");
    assert_eq!(seen[1].method, "PUT");
    assert_eq!(seen[1].path, "/admin/users/42/revoke_moderation.json");
}

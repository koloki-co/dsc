// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Success-path coverage for `dsc tag rename`. The shared `--dry-run`
//! no-mutation registry already proves this never hits the server under
//! `--dry-run`; this test proves the real rename PUT targets the exact old
//! tag's URL and asks for the exact new name, which is what actually
//! preserves topic associations (`PUT /tag/:old.json` with `tag.id: :new`
//! renames the existing tag record in place rather than creating a new one).

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

const TAGS_LIST_BODY: &str =
    r#"{"tags":[{"id":1,"text":"old-name","count":5},{"id":2,"text":"other","count":1}]}"#;

#[test]
fn rename_sends_a_put_to_the_old_tags_url_naming_the_new_tag() {
    let (url, requests) = start_mock(vec![
        MockResponse {
            status: "200 OK",
            body: TAGS_LIST_BODY,
        },
        MockResponse {
            status: "200 OK",
            body: "{}",
        },
    ]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["tag", "rename", "mock", "old-name", "new-name"],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Renamed tag 'old-name' -> 'new-name'")
    );

    let seen = requests.lock().expect("request log");
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/tags.json");
    assert_eq!(seen[1].method, "PUT");
    assert_eq!(
        seen[1].path, "/tag/old-name.json",
        "the rename must target the OLD tag's own URL, not a newly created one"
    );
    let value: serde_json::Value = serde_json::from_str(&seen[1].body).expect("parse PUT body");
    assert_eq!(
        value.get("tag").and_then(|t| t.get("id")),
        Some(&serde_json::Value::String("new-name".to_string())),
        "the PUT body must ask Discourse to rename to the new name, not re-send the old one"
    );
}

#[test]
fn rename_refuses_when_the_old_tag_does_not_exist() {
    let (url, requests) = start_mock(vec![MockResponse {
        status: "200 OK",
        body: TAGS_LIST_BODY,
    }]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["tag", "rename", "mock", "no-such-tag", "new-name"],
        &config_for(&url, &dir),
    );
    assert!(!output.status.success());
    let seen = requests.lock().expect("request log");
    assert_eq!(
        seen.len(),
        1,
        "a missing old tag must refuse before any PUT"
    );
}

#[test]
fn rename_refuses_when_the_new_name_already_exists() {
    let (url, requests) = start_mock(vec![MockResponse {
        status: "200 OK",
        body: TAGS_LIST_BODY,
    }]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["tag", "rename", "mock", "old-name", "other"],
        &config_for(&url, &dir),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
    let seen = requests.lock().expect("request log");
    assert_eq!(
        seen.len(),
        1,
        "a rename that would merge tags must refuse before any PUT"
    );
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! `dsc topic close` / `dsc topic reopen` (R90) against a mock Discourse.
//!
//! Covers the contract from `TopicsController#status`: the only write is
//! `PUT /t/{id}/status.json` with `status=closed` and `enabled=true|false`,
//! dry-run sends nothing, an already-matching state is a no-op, a deleted
//! topic is refused, archived status is never touched, and the post-write
//! re-fetch catches a server that reports success without changing state.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const TOPIC_ID: u64 = 42;

struct Mock {
    closed: bool,
    archived: bool,
    deleted: bool,
    /// When false, the status PUT returns 200 but leaves state unchanged.
    apply_writes: bool,
    /// Every request other than a GET, as `METHOD path body`.
    writes: Vec<String>,
}

fn start_mock(closed: bool, archived: bool) -> (String, Arc<Mutex<Mock>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let state = Arc::new(Mutex::new(Mock {
        closed,
        archived,
        deleted: false,
        apply_writes: true,
        writes: Vec::new(),
    }));
    let thread_state = Arc::clone(&state);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &thread_state);
        }
    });
    (format!("http://{addr}"), state)
}

fn handle(mut stream: TcpStream, state: &Mutex<Mock>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).expect("request body");
    }
    let body = String::from_utf8_lossy(&body).into_owned();

    let topic_path = format!("/t/{TOPIC_ID}.json");
    let status_path = format!("/t/{TOPIC_ID}/status.json");
    let (status, response) = {
        let mut mock = state.lock().expect("mock state");
        if method != "GET" {
            mock.writes.push(format!("{method} {path} {body}"));
        }
        match (method.as_str(), path.as_str()) {
            ("GET", p) if p == topic_path => {
                let mut topic = serde_json::json!({
                    "id": TOPIC_ID,
                    "title": "Old docs",
                    "slug": "old-docs",
                    "posts_count": 3,
                    "closed": mock.closed,
                    "archived": mock.archived,
                    "post_stream": {"posts": [], "stream": []},
                });
                if mock.deleted {
                    topic["deleted_at"] = serde_json::json!("2026-11-01T00:00:00.000Z");
                }
                ("200 OK", topic.to_string())
            }
            ("PUT", p) if p == status_path => {
                if mock.apply_writes {
                    if body.contains("enabled=true") {
                        mock.closed = true;
                    } else if body.contains("enabled=false") {
                        mock.closed = false;
                    }
                }
                (
                    "200 OK",
                    r#"{"success":"OK","topic_status_update":null}"#.to_string(),
                )
            }
            _ => ("404 Not Found", "{}".to_string()),
        }
    };
    let raw = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    );
    let _ = stream.write_all(raw.as_bytes());
}

fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{url}\"\napikey = \"key\"\napi_username = \"system\"\n"
        ),
    )
}

fn run_ok(args: &[&str], config: &std::path::Path) -> String {
    let output = run_dsc(args, config);
    assert!(
        output.status.success(),
        "dsc {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn close_sends_only_the_closed_status_and_matches_its_plan() {
    let (url, state) = start_mock(false, true);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let id = TOPIC_ID.to_string();

    let plan = run_ok(&["--dry-run", "topic", "close", "mock", &id], &config);
    assert!(
        state.lock().unwrap().writes.is_empty(),
        "dry-run must not write"
    );
    assert!(
        plan.contains("would close topic 42 \"Old docs\" (PUT /t/42/status.json status=closed enabled=true; archived: yes, unchanged)"),
        "unexpected plan:\n{plan}"
    );

    let out = run_ok(&["topic", "close", "mock", &id], &config);
    let mock = state.lock().unwrap();
    assert_eq!(
        mock.writes,
        vec!["PUT /t/42/status.json status=closed&enabled=true".to_string()],
        "exactly the planned PUT"
    );
    assert!(mock.closed);
    assert!(mock.archived, "archived must be preserved");
    assert!(out.contains("closed topic 42"), "unexpected output:\n{out}");
}

#[test]
fn reopen_sends_enabled_false() {
    let (url, state) = start_mock(true, false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    run_ok(&["topic", "reopen", "mock", &TOPIC_ID.to_string()], &config);
    let mock = state.lock().unwrap();
    assert_eq!(
        mock.writes,
        vec!["PUT /t/42/status.json status=closed&enabled=false".to_string()]
    );
    assert!(!mock.closed);
}

#[test]
fn already_matching_state_is_a_no_op() {
    let (url, state) = start_mock(true, false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let out = run_ok(&["topic", "close", "mock", &TOPIC_ID.to_string()], &config);
    assert!(out.contains("already closed; no change"), "{out}");

    state.lock().unwrap().closed = false;
    let out = run_ok(&["topic", "reopen", "mock", &TOPIC_ID.to_string()], &config);
    assert!(out.contains("already open; no change"), "{out}");
    assert!(state.lock().unwrap().writes.is_empty());
}

#[test]
fn deleted_topic_is_refused_without_writing() {
    let (url, state) = start_mock(false, false);
    state.lock().unwrap().deleted = true;
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["topic", "close", "mock", &TOPIC_ID.to_string()], &config);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("is deleted"), "{stderr}");
    assert!(state.lock().unwrap().writes.is_empty());
}

#[test]
fn unapplied_write_fails_post_write_verification() {
    let (url, state) = start_mock(false, false);
    state.lock().unwrap().apply_writes = false;
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["topic", "close", "mock", &TOPIC_ID.to_string()], &config);
    assert!(
        !output.status.success(),
        "must fail when state did not change"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("is still open"), "{stderr}");
}

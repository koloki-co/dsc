// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! `dsc topic move` (R87) against a mock Discourse.
//!
//! Covers the source-derived contract from `TopicsController#update`: the only
//! write is `PUT /t/{id}.json` with `category_id=<id>`, dry-run sends nothing,
//! a move to the current category is a no-op, destinations resolve by ID or
//! unique slug (nested subcategories included, ambiguous slugs refused), a
//! category-rule rejection is surfaced, and the post-write re-fetch catches a
//! server that reports success without moving the topic.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const TOPIC_ID: u64 = 42;

struct Mock {
    category_id: u64,
    deleted: bool,
    /// When false, the PUT returns 200 but leaves the category unchanged.
    apply_writes: bool,
    /// When true, the PUT is rejected with a 422 category-rule error.
    reject: bool,
    /// Every request other than a GET, as `METHOD path body`.
    writes: Vec<String>,
}

fn start_mock(category_id: u64) -> (String, Arc<Mutex<Mock>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let state = Arc::new(Mutex::new(Mock {
        category_id,
        deleted: false,
        apply_writes: true,
        reject: false,
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

fn categories_json() -> String {
    serde_json::json!({"category_list": {"categories": [
        {"id": 5, "name": "Finance", "slug": "finance", "subcategory_list": [
            {"id": 11, "name": "Archive", "slug": "archive", "parent_category_id": 5}
        ]},
        {"id": 9, "name": "Business", "slug": "business", "subcategory_list": [
            {"id": 12, "name": "Archive", "slug": "archive", "parent_category_id": 9},
            {"id": 13, "name": "Entities", "slug": "entities", "parent_category_id": 9}
        ]}
    ]}})
    .to_string()
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
    let (status, response) = {
        let mut mock = state.lock().expect("mock state");
        if method != "GET" {
            mock.writes.push(format!("{method} {path} {body}"));
        }
        match (method.as_str(), path.as_str()) {
            ("GET", p) if p == topic_path => {
                let mut topic = serde_json::json!({
                    "id": TOPIC_ID,
                    "title": "Entity A details",
                    "slug": "entity-a-details",
                    "posts_count": 4,
                    "category_id": mock.category_id,
                    "post_stream": {"posts": [], "stream": []},
                });
                if mock.deleted {
                    topic["deleted_at"] = serde_json::json!("2026-11-01T00:00:00.000Z");
                }
                ("200 OK", topic.to_string())
            }
            ("GET", p) if p.starts_with("/categories.json") => ("200 OK", categories_json()),
            ("PUT", p) if p == topic_path => {
                if mock.reject {
                    (
                        "422 Unprocessable Entity",
                        r#"{"errors":["This category requires at least 1 tag"]}"#.to_string(),
                    )
                } else {
                    if mock.apply_writes
                        && let Some(id) = body
                            .strip_prefix("category_id=")
                            .and_then(|v| v.parse().ok())
                    {
                        mock.category_id = id;
                    }
                    (
                        "200 OK",
                        r#"{"basic_topic":{"id":42,"title":"Entity A details"}}"#.to_string(),
                    )
                }
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

fn run_err(args: &[&str], config: &std::path::Path) -> String {
    let output = run_dsc(args, config);
    assert!(
        !output.status.success(),
        "dsc {args:?} unexpectedly succeeded"
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn move_by_id_sends_only_category_id_and_matches_its_plan() {
    let (url, state) = start_mock(5);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let id = TOPIC_ID.to_string();

    let plan = run_ok(&["--dry-run", "topic", "move", "mock", &id, "9"], &config);
    assert!(
        state.lock().unwrap().writes.is_empty(),
        "dry-run must not write"
    );
    assert!(
        plan.contains("would move topic 42 \"Entity A details\" from \"Finance\" (5) to \"Business\" (9) (PUT /t/42.json category_id=9)"),
        "unexpected plan:\n{plan}"
    );

    let out = run_ok(&["topic", "move", "mock", &id, "9"], &config);
    let mock = state.lock().unwrap();
    assert_eq!(
        mock.writes,
        vec!["PUT /t/42.json category_id=9".to_string()],
        "exactly the planned PUT"
    );
    assert_eq!(mock.category_id, 9);
    assert!(
        out.contains(
            "moved topic 42 \"Entity A details\" from \"Finance\" (5) to \"Business\" (9)"
        ),
        "unexpected output:\n{out}"
    );
}

#[test]
fn move_by_unique_subcategory_slug_with_json_output() {
    let (url, state) = start_mock(5);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let out = run_ok(
        &[
            "topic",
            "move",
            "mock",
            &TOPIC_ID.to_string(),
            "entities",
            "--format",
            "json",
        ],
        &config,
    );
    assert_eq!(
        state.lock().unwrap().writes,
        vec!["PUT /t/42.json category_id=13".to_string()]
    );
    let value: serde_json::Value = serde_json::from_str(&out).expect("json output");
    assert_eq!(value["status"], "moved");
    assert_eq!(value["topic_id"], 42);
    assert_eq!(value["from"]["id"], 5);
    assert_eq!(value["from"]["name"], "Finance");
    assert_eq!(value["to"]["id"], 13);
    assert_eq!(value["to"]["name"], "Entities");
    assert_eq!(value["posts_count"], 4);
}

#[test]
fn same_category_is_a_no_op() {
    let (url, state) = start_mock(9);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let out = run_ok(
        &["topic", "move", "mock", &TOPIC_ID.to_string(), "business"],
        &config,
    );
    assert!(
        out.contains("already in \"Business\" (9); no change"),
        "{out}"
    );
    assert!(state.lock().unwrap().writes.is_empty());
}

#[test]
fn unknown_and_ambiguous_destinations_are_refused_without_writing() {
    let (url, state) = start_mock(5);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let id = TOPIC_ID.to_string();

    let stderr = run_err(&["topic", "move", "mock", &id, "777"], &config);
    assert!(stderr.contains("777"), "{stderr}");
    let stderr = run_err(&["topic", "move", "mock", &id, "nope"], &config);
    assert!(stderr.contains("nope"), "{stderr}");
    let stderr = run_err(&["topic", "move", "mock", &id, "archive"], &config);
    assert!(
        stderr.contains("ambiguous") && stderr.contains("11, 12"),
        "{stderr}"
    );
    assert!(state.lock().unwrap().writes.is_empty());
}

#[test]
fn deleted_topic_is_refused_without_writing() {
    let (url, state) = start_mock(5);
    state.lock().unwrap().deleted = true;
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let stderr = run_err(
        &["topic", "move", "mock", &TOPIC_ID.to_string(), "9"],
        &config,
    );
    assert!(stderr.contains("is deleted"), "{stderr}");
    assert!(state.lock().unwrap().writes.is_empty());
}

#[test]
fn category_rule_rejection_is_surfaced() {
    let (url, state) = start_mock(5);
    state.lock().unwrap().reject = true;
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let stderr = run_err(
        &["topic", "move", "mock", &TOPIC_ID.to_string(), "9"],
        &config,
    );
    assert!(stderr.contains("requires at least 1 tag"), "{stderr}");
    assert_eq!(state.lock().unwrap().category_id, 5);
}

#[test]
fn unapplied_write_fails_post_write_verification() {
    let (url, state) = start_mock(5);
    state.lock().unwrap().apply_writes = false;
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let stderr = run_err(
        &["topic", "move", "mock", &TOPIC_ID.to_string(), "9"],
        &config,
    );
    assert!(
        stderr.contains("is in \"Finance\" (5) rather than \"Business\" (9)"),
        "{stderr}"
    );
}

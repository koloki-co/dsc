// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! R84 regression tests: `dsc topic list <forum> --deleted` must parse the
//! topic-list `tags` shapes Discourse actually returns - current releases send
//! tag objects (`{"id","name","slug"}`), older ones plain strings - across
//! pagination, and must fail with a shape-only error (no payload echo) on a
//! tag shape it does not recognise.
//!
//! Field driver: `dsc 0.24.0` against Discourse `2026.11.0-latest.1` exited 1
//! with `invalid type: map, expected a string at line 1 column 4700`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;

fn start_mock(handler: impl Fn(&str) -> (u16, String) + Send + Sync + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, handler.as_ref());
        }
    });
    format!("http://{addr}")
}

fn handle(mut stream: TcpStream, handler: &(dyn Fn(&str) -> (u16, String) + Send + Sync)) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if !matches!(reader.read_line(&mut line), Ok(n) if n > 0) || line == "\r\n" || line == "\n"
        {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    if content_length > 0 {
        let mut discard = vec![0u8; content_length];
        let _ = reader.read_exact(&mut discard);
    }
    let (code, body) = handler(&path);
    let response = format!(
        "HTTP/1.1 {code} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn make_config(baseurl: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let config = dir.path().join("dsc.toml");
    std::fs::write(
        &config,
        format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"mock-key\"\napi_username = \"tester\"\n"
        ),
    )
    .expect("write config");
    (dir, config)
}

fn run_dsc(args: &[&str], config: &PathBuf) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dsc"))
        .args(args)
        .env("DSC_CONFIG", config)
        .output()
        .expect("running dsc")
}

/// Detail response for `/t/{id}.json`: soft-deleted, in category 3.
fn deleted_topic_detail(id: u64) -> String {
    serde_json::json!({
        "id": id,
        "title": format!("Topic {id}"),
        "category_id": 3,
        "deleted_at": "2026-11-10T09:00:00.000Z",
        "post_stream": { "posts": [], "stream": [] }
    })
    .to_string()
}

#[test]
fn deleted_topic_list_accepts_tag_objects_and_legacy_strings_across_pages() {
    let baseurl = start_mock(|path| {
        if path == "/latest.json?status=deleted&per_page=100" {
            // Current shape: map-valued tags, as on Discourse 2026.11.
            let body = serde_json::json!({
                "topic_list": {
                    "topics": [{
                        "id": 11,
                        "title": "Changelog",
                        "slug": "changelog",
                        "posts_count": 4,
                        "category_id": 3,
                        "tags": [
                            {"id": 1, "name": "maintenance", "slug": "maintenance"},
                            {"id": 2, "slug": "ops"}
                        ]
                    }],
                    "more_topics_url": "/latest?status=deleted&per_page=100&page=1"
                }
            });
            (200, body.to_string())
        } else if path.starts_with("/latest.json?status=deleted") && path.contains("page=1") {
            // Legacy shape: plain string tags.
            let body = serde_json::json!({
                "topic_list": {
                    "topics": [{
                        "id": 12,
                        "title": "Old changelog",
                        "slug": "old-changelog",
                        "posts_count": 1,
                        "category_id": 3,
                        "tags": ["legacy"]
                    }]
                }
            });
            (200, body.to_string())
        } else if let Some(id) = path
            .strip_prefix("/t/")
            .and_then(|rest| rest.strip_suffix(".json"))
            .and_then(|id| id.parse::<u64>().ok())
        {
            (200, deleted_topic_detail(id))
        } else {
            (404, r#"{"errors":["not found"]}"#.to_string())
        }
    });
    let (_dir, config) = make_config(&baseurl);
    let output = run_dsc(
        &[
            "topic",
            "list",
            "mock",
            "--deleted",
            "changelog",
            "--format",
            "json",
        ],
        &config,
    );
    assert!(
        output.status.success(),
        "deleted-topic list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("deleted-topic list JSON");
    let rows = listed.as_array().expect("array output");
    assert_eq!(rows.len(), 2, "both pages should contribute: {listed}");
    assert_eq!(rows[0]["id"], 11);
    assert_eq!(rows[0]["tags"], serde_json::json!(["maintenance", "ops"]));
    assert_eq!(rows[1]["id"], 12);
    assert_eq!(rows[1]["tags"], serde_json::json!(["legacy"]));
}

#[test]
fn deleted_topic_list_rejects_unrecognised_tag_shape_without_echoing_it() {
    let baseurl = start_mock(|path| {
        if path.starts_with("/latest.json?status=deleted") {
            let body = serde_json::json!({
                "topic_list": {
                    "topics": [{
                        "id": 11,
                        "title": "Changelog",
                        "tags": [{"id": 1, "label": "secret-tag-label"}]
                    }]
                }
            });
            (200, body.to_string())
        } else {
            (404, r#"{"errors":["not found"]}"#.to_string())
        }
    });
    let (_dir, config) = make_config(&baseurl);
    let output = run_dsc(&["topic", "list", "mock", "--deleted"], &config);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "malformed tags must fail");
    assert!(output.stdout.is_empty(), "no partial output on failure");
    assert!(
        stderr.contains("parsing deleted-topic list response")
            && stderr.contains("unsupported topic tag object"),
        "expected a shape-specific parse error, got: {stderr}"
    );
    assert!(
        !stderr.contains("secret-tag-label"),
        "error must not echo payload content: {stderr}"
    );
}

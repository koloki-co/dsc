// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! `dsc topic tag` / `dsc topic untag` against the current single-topic
//! response shape, where `/t/{id}.json` may return `tags` as objects
//! (`{"id","name","slug"}`) rather than plain strings (see R84).
//!
//! Both commands replace the topic's *whole* tag list with one PUT, so the
//! current tags they read first must survive the round-trip. Before the fix,
//! object-shaped tags were silently dropped while reading, so `topic tag`
//! would PUT only the new tag - deleting every existing tag from the topic -
//! and `topic untag` reported the tag as absent. Each test also checks the
//! `--dry-run` plan names exactly the list the real run sends.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const TOPIC_ID: u64 = 42;

struct Mock {
    /// The topic's current tags, served as tag objects on `GET /t/42.json`.
    tags: Vec<String>,
    /// Every `tags[]` list sent by a `PUT /t/42.json`, in order.
    puts: Vec<Vec<String>>,
}

fn start_mock(initial: &[&str]) -> (String, Arc<Mutex<Mock>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let state = Arc::new(Mutex::new(Mock {
        tags: initial.iter().map(|t| t.to_string()).collect(),
        puts: Vec::new(),
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
    let (status, response) = {
        let mut mock = state.lock().expect("mock state");
        match (method.as_str(), path.as_str()) {
            ("GET", p) if p == topic_path => ("200 OK", topic_body(&mock.tags)),
            ("PUT", p) if p == topic_path => {
                let sent = form_tags(&body);
                mock.tags = sent.clone();
                mock.puts.push(sent);
                ("200 OK", "{}".to_string())
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

/// A current-shape topic body: every tag is an object, not a string.
fn topic_body(tags: &[String]) -> String {
    let objects: Vec<serde_json::Value> = tags
        .iter()
        .enumerate()
        .map(|(i, name)| serde_json::json!({"id": i + 1, "name": name, "slug": name}))
        .collect();
    serde_json::json!({"id": TOPIC_ID, "title": "t", "tags": objects}).to_string()
}

fn form_tags(body: &str) -> Vec<String> {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(k, _)| *k == "tags%5B%5D" || *k == "tags[]")
        .map(|(_, v)| v.replace('+', " "))
        .filter(|v| !v.is_empty())
        .collect()
}

fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{url}\"\napikey = \"key\"\napi_username = \"system\"\n"
        ),
    )
}

/// Parse `[dry-run] would set tags on topic 42 to: [a, b]` into its list.
fn planned_tags(stdout: &str) -> Vec<String> {
    let prefix = format!("[dry-run] would set tags on topic {TOPIC_ID} to: [");
    let line = stdout
        .lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("no dry-run plan line in:\n{stdout}"));
    let inner = line.strip_suffix(']').expect("plan line ends with ]");
    if inner.is_empty() {
        return Vec::new();
    }
    inner.split(", ").map(str::to_string).collect()
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
fn topic_tag_keeps_existing_object_shaped_tags_and_matches_its_plan() {
    let (url, state) = start_mock(&["bug", "ops"]);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let id = TOPIC_ID.to_string();

    let plan = planned_tags(&run_ok(
        &["--dry-run", "topic", "tag", "mock", &id, "urgent"],
        &config,
    ));
    assert!(
        state.lock().unwrap().puts.is_empty(),
        "dry-run must not PUT"
    );

    run_ok(&["topic", "tag", "mock", &id, "urgent"], &config);
    let mock = state.lock().unwrap();
    assert_eq!(mock.puts.len(), 1, "exactly one PUT");
    assert_eq!(
        mock.puts[0],
        vec!["bug", "ops", "urgent"],
        "existing tags must be kept, not wiped"
    );
    assert_eq!(plan, mock.puts[0], "dry-run plan must match the real PUT");
}

#[test]
fn topic_untag_finds_an_object_shaped_tag_and_matches_its_plan() {
    let (url, state) = start_mock(&["bug", "ops", "urgent"]);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let id = TOPIC_ID.to_string();

    let plan = planned_tags(&run_ok(
        &["--dry-run", "topic", "untag", "mock", &id, "ops"],
        &config,
    ));
    assert!(
        state.lock().unwrap().puts.is_empty(),
        "dry-run must not PUT"
    );

    run_ok(&["topic", "untag", "mock", &id, "ops"], &config);
    let mock = state.lock().unwrap();
    assert_eq!(mock.puts.len(), 1, "exactly one PUT");
    assert_eq!(mock.puts[0], vec!["bug", "urgent"]);
    assert_eq!(plan, mock.puts[0], "dry-run plan must match the real PUT");
}

#[test]
fn topic_tag_refuses_an_unrecognised_tag_shape_without_writing() {
    // A tag entry this boundary does not understand must stop the command,
    // because the follow-up PUT would otherwise delete it from the topic.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let puts = Arc::new(Mutex::new(0usize));
    let thread_puts = Arc::clone(&puts);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            if line.starts_with("PUT") {
                *thread_puts.lock().unwrap() += 1;
            }
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                    break;
                }
            }
            let body = r#"{"id":42,"tags":[{"id":1,"name":"bug"},{"id":2,"label":"mystery"}]}"#;
            let raw = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(raw.as_bytes());
        }
    });
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&format!("http://{addr}"), &dir);
    let output = run_dsc(&["topic", "tag", "mock", "42", "urgent"], &config);
    assert!(!output.status.success(), "an unknown tag shape must fail");
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("mystery"),
        "the error must not echo payload content"
    );
    assert_eq!(*puts.lock().unwrap(), 0, "no PUT may be sent");
}

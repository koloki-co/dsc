// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Functional coverage for `dsc board list|show|pull` against a mocked
//! `/boards/api/*` plugin API. See `spec/commands/boards.md` for the API
//! shapes captured against a live forum.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use tempfile::TempDir;

/// A mock Discourse that answers `/boards/api/boards.json` and
/// `/boards/api/boards/:id.json` with fixed payloads, or a 404 when
/// `not_found` is set (simulating a disabled/unlicensed Boards plugin).
fn start_mock(list_json: String, show_json: String, not_found: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &list_json, &show_json, not_found);
        }
    });
    format!("http://{addr}")
}

fn handle(mut stream: TcpStream, list_json: &str, show_json: &str, not_found: bool) {
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

    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();

    let (status_line, body) = if not_found {
        (
            "HTTP/1.1 404 Not Found",
            "{\"errors\":[\"not found\"]}".to_string(),
        )
    } else if path.starts_with("/boards/api/boards.json") {
        ("HTTP/1.1 200 OK", list_json.to_string())
    } else if path.starts_with("/boards/api/boards/") {
        ("HTTP/1.1 200 OK", show_json.to_string())
    } else if path.starts_with("/about.json") {
        (
            "HTTP/1.1 200 OK",
            r#"{"about":{"version":"2026.9.0"}}"#.to_string(),
        )
    } else {
        ("HTTP/1.1 404 Not Found", "{}".to_string())
    };
    let response = format!(
        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn sample_list() -> String {
    r#"{"boards":[{"id":3,"name":"Roadmap","slug":"roadmap","tag_names":["discourse"],"can_manage":true}]}"#.to_string()
}

fn sample_show() -> String {
    r#"{
        "board": {"id":3,"name":"Roadmap","slug":"roadmap","tag_names":["discourse"],"can_manage":true},
        "columns": [
            {
                "id": 10,
                "title": "Backlog",
                "color": "2f7ed8",
                "cards": [
                    {"id": 100, "board_id": 3, "column_id": 10, "card_type": "floater", "title": "Write spec", "notes": "draft"},
                    {"id": 101, "board_id": 3, "column_id": 10, "card_type": "topic", "title": null, "topic_id": 1261, "topic": {"id": 1261, "title": "Discuss roadmap"}}
                ]
            }
        ]
    }"#.to_string()
}

fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    )
}

#[test]
fn board_list_prints_accessible_boards() {
    let url = start_mock(sample_list(), sample_show(), false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["board", "list", "alpha", "--format", "json"], &config);
    assert!(
        output.status.success(),
        "board list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let boards: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("board list JSON");
    assert_eq!(boards.len(), 1);
    assert_eq!(boards[0]["id"], 3);
    assert_eq!(boards[0]["name"], "Roadmap");

    let text_output = run_dsc(&["board", "list", "alpha"], &config);
    assert!(text_output.status.success());
    let text = String::from_utf8_lossy(&text_output.stdout);
    assert!(text.contains("Roadmap"));
    assert!(text.contains("acl:manage"));
}

#[test]
fn board_show_renders_columns_and_distinguishes_card_types() {
    let url = start_mock(sample_list(), sample_show(), false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["board", "show", "alpha", "3"], &config);
    assert!(
        output.status.success(),
        "board show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("Backlog"));
    assert!(text.contains("Write spec"));
    assert!(text.contains("Discuss roadmap"));
    assert!(text.contains("topic_id:1261"));

    let json_output = run_dsc(
        &["board", "show", "alpha", "3", "--format", "json"],
        &config,
    );
    assert!(json_output.status.success());
    let detail: serde_json::Value =
        serde_json::from_slice(&json_output.stdout).expect("board show JSON");
    assert_eq!(detail["board"]["id"], 3);
    assert_eq!(detail["columns"][0]["cards"].as_array().unwrap().len(), 2);
}

#[test]
fn board_pull_snapshots_board_including_floater_cards() {
    let url = start_mock(sample_list(), sample_show(), false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let out_path = dir.path().join("board.yaml");

    let output = run_dsc(
        &["board", "pull", "alpha", "3", out_path.to_str().unwrap()],
        &config,
    );
    assert!(
        output.status.success(),
        "board pull failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out_path.exists());
    let content = std::fs::read_to_string(&out_path).unwrap();
    let snapshot: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    assert_eq!(snapshot["version"], 1);
    assert_eq!(snapshot["board_id"], 3);
    assert_eq!(snapshot["board_name"], "Roadmap");
    assert_eq!(snapshot["discourse_version"], "2026.9.0");
    assert!(snapshot["pulled_at"].is_string());
    let cards = snapshot["columns"][0]["cards"].as_sequence().unwrap();
    assert_eq!(cards.len(), 2, "floater card must survive the snapshot");
    assert_eq!(cards[0]["card_type"], "floater");
    assert_eq!(cards[1]["card_type"], "topic");

    // Refuses to overwrite without --force.
    let output = run_dsc(
        &["board", "pull", "alpha", "3", out_path.to_str().unwrap()],
        &config,
    );
    assert!(!output.status.success());

    let output = run_dsc(
        &[
            "board",
            "pull",
            "alpha",
            "3",
            out_path.to_str().unwrap(),
            "--force",
        ],
        &config,
    );
    assert!(output.status.success());
}

#[test]
fn board_pull_writes_json_when_path_ends_json() {
    let url = start_mock(sample_list(), sample_show(), false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let out_path = dir.path().join("board.json");

    let output = run_dsc(
        &["board", "pull", "alpha", "3", out_path.to_str().unwrap()],
        &config,
    );
    assert!(output.status.success());
    let content = std::fs::read_to_string(&out_path).unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
    assert_eq!(snapshot["board_id"], 3);
}

#[test]
fn board_commands_report_a_clear_error_when_plugin_is_disabled() {
    let url = start_mock(sample_list(), sample_show(), true);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["board", "list", "alpha"], &config);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Boards") && stderr.contains("Business/Enterprise"),
        "expected a Boards-specific 404 hint, got: {stderr}"
    );
}

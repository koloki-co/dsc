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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// A mock Discourse that answers `/boards/api/boards.json` and
/// `/boards/api/boards/:id.json` with fixed payloads, or a 404 when
/// `not_found` is set (simulating a disabled/unlicensed Boards plugin).
fn start_mock(list_json: String, show_json: String, not_found: bool) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let show_requests = Arc::new(AtomicUsize::new(0));
    let thread_show_requests = Arc::clone(&show_requests);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(
                stream,
                &list_json,
                &show_json,
                not_found,
                &thread_show_requests,
            );
        }
    });
    (format!("http://{addr}"), show_requests)
}

fn handle(
    mut stream: TcpStream,
    list_json: &str,
    show_json: &str,
    not_found: bool,
    show_requests: &AtomicUsize,
) {
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
        show_requests.fetch_add(1, Ordering::Relaxed);
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
        "board": {"id":3,"name":"Roadmap","slug":"roadmap","category_ids":[9,4],"tag_ids":[7],"tag_names":["discourse"],"anonymous_can_read":true,"require_confirmation":true,"show_tags":true,"card_style":"detailed","show_topic_thumbnail":true,"can_manage":true,"created_by":{"username":"admin"},"acl":[{"id":"admins","type":"group","permission":"owner"}],"server_only":"ignored"},
        "columns": [
            {
                "id": 10,
                "title": "Backlog",
                "icon": "list",
                "position": 65536,
                "default_sort": "priority",
                "tag_id": 7,
                "tag_name": "discourse",
                "move_to_category_id": 4,
                "move_to_assigned": "alice",
                "move_to_status": "open",
                "color": "2f7ed8",
                "server_only": "ignored",
                "cards": [
                    {"id": 100, "board_id": 3, "column_id": 10, "card_type": "floater", "position": 65536, "title": "Write spec", "notes": "draft", "tag_ids": [8,7], "tags": [{"id":8,"name":"writing","slug":"writing"},{"id":7,"name":"discourse","slug":"discourse"}], "created_at": "2026-09-01T00:00:00Z", "assigned_to": {"type":"User","username":"alice","avatar_template":"/user_avatar/alice/{size}/1.png"}, "server_only": "ignored"},
                    {"id": 101, "board_id": 3, "column_id": 10, "card_type": "topic", "position": 131072, "title": null, "topic_id": 1261, "topic": {"id": 1261, "title": "Discuss roadmap", "posts_count": 4}}
                ]
            }
        ]
    }"#.to_string()
}

fn recency_show() -> String {
    r#"{
        "board": {"id":3,"name":"Roadmap"},
        "columns": [{
            "id":10,
            "title":"Recently active",
            "default_sort":"recency",
            "cards":[
                {"id":102,"card_type":"floater","position":131072,"title":"New activity"},
                {"id":101,"card_type":"floater","position":65536,"title":"Earlier priority"}
            ]
        }]
    }"#
    .to_string()
}

fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    )
}

#[derive(Default)]
struct ScaffoldMock {
    boards: Vec<serde_json::Value>,
    creates: Vec<serde_json::Value>,
    list_requests: usize,
    malformed_create_response: bool,
}

fn start_scaffold_mock(state: ScaffoldMock) -> (String, Arc<Mutex<ScaffoldMock>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind scaffold mock");
    let addr = listener.local_addr().expect("scaffold mock addr");
    let state = Arc::new(Mutex::new(state));
    let thread_state = Arc::clone(&state);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle_scaffold(stream, &thread_state);
        }
    });
    (format!("http://{addr}"), state)
}

fn handle_scaffold(mut stream: TcpStream, state: &Arc<Mutex<ScaffoldMock>>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone scaffold stream"));
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
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).expect("read scaffold body");
    let method = request_line.split_whitespace().next().unwrap_or("");
    let path = request_line.split_whitespace().nth(1).unwrap_or("");
    let (status, response) = match (method, path) {
        ("GET", "/boards/api/boards.json") => {
            let mut state = state.lock().expect("lock scaffold state");
            state.list_requests += 1;
            (
                "200 OK",
                serde_json::json!({ "boards": state.boards }).to_string(),
            )
        }
        ("POST", "/boards/api/boards.json") => {
            let request: serde_json::Value = serde_json::from_slice(&body).expect("scaffold JSON");
            let mut state = state.lock().expect("lock scaffold state");
            state.creates.push(request.clone());
            let mut board = request["board"].clone();
            board["id"] = serde_json::json!(42);
            for (index, column) in board["columns"]
                .as_array_mut()
                .expect("scaffold columns")
                .iter_mut()
                .enumerate()
            {
                column["id"] = serde_json::json!(index + 1);
            }
            if state.malformed_create_response {
                let mut malformed = board.clone();
                malformed["columns"] = serde_json::json!([]);
                (
                    "201 Created",
                    serde_json::json!({ "board": malformed }).to_string(),
                )
            } else {
                state.boards.push(board.clone());
                (
                    "201 Created",
                    serde_json::json!({ "board": board }).to_string(),
                )
            }
        }
        _ => ("404 Not Found", "{}".to_string()),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("write scaffold response");
}

#[test]
fn board_list_prints_accessible_boards() {
    let (url, _) = start_mock(sample_list(), sample_show(), false);
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
    let (url, _) = start_mock(sample_list(), sample_show(), false);
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
    assert_eq!(
        detail["columns"][0]["cards"][0]["tags"][0]["name"],
        "writing"
    );
}

#[test]
fn board_pull_snapshots_board_including_floater_cards() {
    let (url, show_requests) = start_mock(sample_list(), sample_show(), false);
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
    assert_eq!(show_requests.load(Ordering::Relaxed), 1);
    let content = std::fs::read_to_string(&out_path).unwrap();
    let snapshot: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    assert_eq!(snapshot["version"], 1);
    assert_eq!(snapshot["board_id"], 3);
    assert_eq!(snapshot["board_name"], "Roadmap");
    assert_eq!(snapshot["discourse_version"], "2026.9.0");
    assert!(snapshot["pulled_at"].is_string());
    assert_eq!(snapshot["provenance"]["source"], url);
    assert_eq!(
        snapshot["provenance"]["exporter_version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(snapshot["board"]["category_ids"][0], 4);
    assert_eq!(snapshot["board"]["category_ids"][1], 9);
    assert_eq!(snapshot["board"]["require_confirmation"], true);
    assert!(snapshot["board"].get("id").is_none());
    assert!(snapshot["board"].get("tag_ids").is_none());
    assert!(snapshot["board"].get("can_manage").is_none());
    assert!(snapshot["board"].get("anonymous_can_read").is_none());
    assert!(snapshot["board"].get("acl").is_none());
    assert!(snapshot["board"].get("server_only").is_none());
    assert!(snapshot["columns"][0].get("position").is_none());
    assert!(snapshot["columns"][0].get("tag_id").is_none());
    assert!(snapshot["columns"][0].get("server_only").is_none());
    let cards = snapshot["columns"][0]["cards"].as_sequence().unwrap();
    assert_eq!(cards.len(), 2, "floater card must survive the snapshot");
    assert_eq!(cards[0]["card_type"], "floater");
    assert_eq!(cards[0]["tags"][0], "discourse");
    assert_eq!(cards[0]["tags"][1], "writing");
    assert_eq!(cards[0]["assigned_to"]["type"], "User");
    assert_eq!(cards[0]["assigned_to"]["username"], "alice");
    assert!(cards[0].get("position").is_none());
    assert!(cards[0].get("tag_ids").is_none());
    assert!(cards[0].get("board_id").is_none());
    assert!(cards[0].get("column_id").is_none());
    assert!(cards[0].get("created_at").is_none());
    assert!(cards[0].get("server_only").is_none());
    assert_eq!(cards[1]["card_type"], "topic");
    assert!(cards[1].get("topic").is_none());

    // Refuses to overwrite without --force.
    let output = run_dsc(
        &["board", "pull", "alpha", "3", out_path.to_str().unwrap()],
        &config,
    );
    assert!(!output.status.success());
    assert_eq!(
        show_requests.load(Ordering::Relaxed),
        1,
        "overwrite refusal must happen before the side-effectful detail GET"
    );

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
    assert_eq!(show_requests.load(Ordering::Relaxed), 2);
}

#[test]
fn board_pull_writes_json_when_path_ends_json() {
    let (url, _) = start_mock(sample_list(), sample_show(), false);
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
fn board_pull_canonicalizes_recency_cards_by_persisted_position() {
    let (url, _) = start_mock(sample_list(), recency_show(), false);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let out_path = dir.path().join("board.yaml");

    let output = run_dsc(
        &["board", "pull", "alpha", "3", out_path.to_str().unwrap()],
        &config,
    );
    assert!(output.status.success());
    let content = std::fs::read_to_string(&out_path).unwrap();
    let snapshot: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    let cards = snapshot["columns"][0]["cards"].as_sequence().unwrap();
    assert_eq!(cards[0]["id"], 101);
    assert_eq!(cards[1]["id"], 102);
    assert!(cards[0].get("position").is_none());
}

#[test]
fn board_commands_report_a_clear_error_when_plugin_is_disabled() {
    let (url, _) = start_mock(sample_list(), sample_show(), true);
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

#[test]
fn board_templates_are_offline_and_list_the_standard_columns() {
    let dir = TempDir::new().expect("tempdir");
    let missing_config = dir.path().join("does-not-exist.toml");
    let output = run_dsc(&["board", "templates", "--format", "json"], &missing_config);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let templates: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("templates JSON");
    assert_eq!(templates[0]["name"], "to-do");
    assert_eq!(templates[0]["default_name"], "To-do");
    assert_eq!(
        templates[0]["columns"],
        serde_json::json!(["Ideas", "Backlog", "In progress", "Completed", "Reference"])
    );
}

#[test]
fn board_scaffold_dry_run_plans_the_exact_transaction_without_posting() {
    let (url, state) = start_scaffold_mock(ScaffoldMock::default());
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let output = run_dsc(
        &[
            "--dry-run",
            "board",
            "scaffold",
            "alpha",
            "to-do",
            "--format",
            "json",
        ],
        &config,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("scaffold plan JSON");
    assert_eq!(plan["action"], "planned");
    assert_eq!(plan["slug"], "to-do");
    assert_eq!(
        plan["request"],
        serde_json::json!({"board":{"name":"To-do","slug":"to-do","columns":[{"title":"Ideas"},{"title":"Backlog"},{"title":"In progress"},{"title":"Completed"},{"title":"Reference"}],"acl":[]}})
    );
    let state = state.lock().expect("lock state");
    assert_eq!(state.list_requests, 1);
    assert!(state.creates.is_empty(), "dry-run must not POST");
}

#[test]
fn board_scaffold_creates_once_then_preserves_the_existing_board() {
    let (url, state) = start_scaffold_mock(ScaffoldMock::default());
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let first = run_dsc(
        &["board", "scaffold", "alpha", "to-do", "--format", "json"],
        &config,
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let created: serde_json::Value = serde_json::from_slice(&first.stdout).expect("create JSON");
    assert_eq!(created["action"], "created");
    assert_eq!(created["board_id"], 42);

    let second = run_dsc(
        &["board", "scaffold", "alpha", "to-do", "--format", "json"],
        &config,
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let unchanged: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("unchanged JSON");
    assert_eq!(unchanged["action"], "unchanged");
    let state = state.lock().expect("lock state");
    assert_eq!(
        state.creates.len(),
        1,
        "a matching board must not be adopted or posted again"
    );
}

#[test]
fn board_scaffold_refuses_a_conflicting_existing_slug_without_posting() {
    let (url, state) = start_scaffold_mock(ScaffoldMock {
        boards: vec![serde_json::json!({
            "id": 7,
            "name": "To-do",
            "slug": "to-do",
            "columns": [{"id": 1, "title": "Different"}],
        })],
        ..Default::default()
    });
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let output = run_dsc(&["board", "scaffold", "alpha", "to-do"], &config);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("different layout"));
    assert!(state.lock().expect("lock state").creates.is_empty());
}

#[test]
fn board_scaffold_names_an_unavailable_boards_plugin_before_posting() {
    let (url, _) = start_mock(sample_list(), sample_show(), true);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let output = run_dsc(&["board", "scaffold", "alpha", "to-do"], &config);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Boards") && stderr.contains("Business/Enterprise"),
        "expected a Boards-specific unavailable hint, got: {stderr}"
    );
}

#[test]
fn board_scaffold_does_not_report_success_when_create_verification_fails() {
    let (url, state) = start_scaffold_mock(ScaffoldMock {
        malformed_create_response: true,
        ..Default::default()
    });
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let output = run_dsc(&["board", "scaffold", "alpha", "to-do"], &config);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("verification failed"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Created board"));
    assert_eq!(state.lock().expect("lock state").creates.len(), 1);
}

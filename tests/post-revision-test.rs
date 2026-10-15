// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Functional coverage for `dsc post revision list|show|revert` against a
//! stateful mock of Discourse's post revision routes.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[derive(Default)]
struct State {
    /// Highest revision number; revisions 2..=latest exist.
    latest: u64,
    /// When true the revert endpoint succeeds without creating a revision.
    ignore_revert: bool,
    /// When true the revert endpoint answers 422.
    noop_revert: bool,
    log: Vec<String>,
}

fn revision_json(n: u64, latest: u64) -> String {
    let prev = if n > 2 { n - 1 } else { 0 };
    let prev = if prev == 0 {
        "null".to_string()
    } else {
        prev.to_string()
    };
    format!(
        r#"{{"post_id":5,"created_at":"2026-01-0{}T00:00:00Z","username":"editor","edit_reason":"r{n}","current_revision":{n},"first_revision":2,"previous_revision":{prev},"next_revision":null,"last_revision":{latest},"version_count":{latest},"body_changes":{{"side_by_side_markdown":"<tr><td><del>old</del><ins>new</ins></td></tr>"}},"title_changes":{{"inline":""}}}}"#,
        n.min(9)
    )
}

fn start_mock(state: Arc<Mutex<State>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &state);
        }
    });
    format!("http://{addr}")
}

fn handle(mut stream: TcpStream, state: &Mutex<State>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
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
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut st = state.lock().unwrap();
    st.log.push(format!("{method} {path}"));
    let (status, out) = if method == "GET" && path == "/posts/5.json" {
        (
            "200 OK",
            r#"{"id":5,"topic_id":9,"post_number":1}"#.to_string(),
        )
    } else if method == "GET" && path == "/posts/5/revisions/latest.json" {
        ("200 OK", revision_json(st.latest, st.latest))
    } else if method == "GET" && path.starts_with("/posts/5/revisions/") {
        let n: u64 = path
            .trim_start_matches("/posts/5/revisions/")
            .trim_end_matches(".json")
            .parse()
            .unwrap_or(0);
        if (2..=st.latest).contains(&n) {
            ("200 OK", revision_json(n, st.latest))
        } else {
            ("404 Not Found", r#"{"errors":["not found"]}"#.to_string())
        }
    } else if method == "PUT" && path.ends_with("/revert.json") {
        if st.noop_revert {
            (
                "422 Unprocessable Entity",
                r#"{"errors":["no change"]}"#.to_string(),
            )
        } else {
            if !st.ignore_revert {
                st.latest += 1;
            }
            ("200 OK", r#"{"post":{"id":5}}"#.to_string())
        }
    } else {
        ("404 Not Found", "{}".to_string())
    };
    drop(st);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
        out.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn setup(latest: u64) -> (Arc<Mutex<State>>, TempDir, std::path::PathBuf) {
    let state = Arc::new(Mutex::new(State {
        latest,
        ..Default::default()
    }));
    let url = start_mock(Arc::clone(&state));
    let dir = TempDir::new().unwrap();
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );
    (state, dir, config)
}

fn puts(state: &Mutex<State>) -> Vec<String> {
    state
        .lock()
        .unwrap()
        .log
        .iter()
        .filter(|l| l.starts_with("PUT"))
        .cloned()
        .collect()
}

#[test]
fn list_follows_previous_links_newest_first() {
    let (_s, _d, config) = setup(4);
    let out = run_dsc(
        &["post", "revision", "list", "alpha", "5", "-f", "json"],
        &config,
    );
    assert!(out.status.success());
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).expect("json rows");
    let nums: Vec<u64> = rows
        .iter()
        .map(|r| r["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(nums, vec![4, 3, 2]);
    assert_eq!(rows[0]["changed_fields"], serde_json::json!(["body"]));
}

#[test]
fn show_prints_markdown_diff_without_html() {
    let (_s, _d, config) = setup(3);
    let out = run_dsc(&["post", "revision", "show", "alpha", "5", "3"], &config);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("[-old-]{+new+}"), "{text}");
    assert!(!text.contains("<td>"), "{text}");
}

#[test]
fn show_missing_revision_fails() {
    let (_s, _d, config) = setup(3);
    let out = run_dsc(&["post", "revision", "show", "alpha", "5", "40"], &config);
    assert!(!out.status.success());
}

#[test]
fn revert_refuses_revision_one_without_requests() {
    let (s, _d, config) = setup(3);
    let out = run_dsc(
        &["post", "revision", "revert", "alpha", "5", "1", "--yes"],
        &config,
    );
    assert!(!out.status.success());
    assert!(s.lock().unwrap().log.is_empty());
}

#[test]
fn revert_requires_yes() {
    let (s, _d, config) = setup(3);
    let out = run_dsc(&["post", "revision", "revert", "alpha", "5", "2"], &config);
    assert!(!out.status.success());
    assert!(puts(&s).is_empty());
}

#[test]
fn dry_run_sends_no_put() {
    let (s, _d, config) = setup(3);
    let out = run_dsc(
        &["-n", "post", "revision", "revert", "alpha", "5", "2"],
        &config,
    );
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("PUT /posts/5/revisions/2/revert.json"),
        "{text}"
    );
    assert!(puts(&s).is_empty());
}

#[test]
fn live_revert_sends_exact_put_and_verifies() {
    let (s, _d, config) = setup(3);
    let out = run_dsc(
        &["post", "revision", "revert", "alpha", "5", "2", "--yes"],
        &config,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(puts(&s), vec!["PUT /posts/5/revisions/2/revert.json"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("now at revision 4"));
}

#[test]
fn revert_without_new_revision_fails_verification() {
    let (s, _d, config) = setup(3);
    s.lock().unwrap().ignore_revert = true;
    let out = run_dsc(
        &["post", "revision", "revert", "alpha", "5", "2", "--yes"],
        &config,
    );
    assert!(!out.status.success());
}

#[test]
fn noop_revert_surfaces_server_error() {
    let (s, _d, config) = setup(3);
    s.lock().unwrap().noop_revert = true;
    let out = run_dsc(
        &["post", "revision", "revert", "alpha", "5", "2", "--yes"],
        &config,
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("422"));
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! R52/P26 regression tests: a server that never terminates a "list
//! everything" pagination loop (always returns a fresh, unique continuation)
//! must not be followed forever. Each of these drives one of the five
//! uncapped loops identified by the performance audit past its budget and
//! asserts the command fails with a clear "exceeded" error rather than
//! hanging or growing memory without bound.
//!
//! Self-contained mock, deliberately not shared with `request-budget-test.rs`
//! (whose `get_body` dispatcher returns one fixed, cycle-safe body per route
//! and has no notion of an ever-advancing continuation page).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

/// Starts a mock HTTP server that answers every GET via `handler(path) ->
/// (status_code, body)`. Only GET is exercised by these tests.
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
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line == "\n" {
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
    let status_line = if code == 404 {
        "404 Not Found"
    } else {
        "200 OK"
    };
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status_line,
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn run_dsc(args: &[&str], config: &Path) -> (String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .args(args)
        .env("DSC_CONFIG", config)
        .output()
        .expect("running dsc");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (combined, output.status.success())
}

fn make_config(baseurl: &str) -> (TempDir, std::path::PathBuf) {
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

// ─── P26: category topic-list pagination has a page budget ───────────────

#[test]
fn category_pagination_exceeding_the_page_budget_fails_with_a_clear_error() {
    let counter = Arc::new(AtomicUsize::new(0));
    let baseurl = start_mock(move |path| {
        if path.starts_with("/c/4.json") {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            (
                200,
                format!(r#"{{"topic_list":{{"topics":[],"more_topics_url":"/c/4?p={n}"}}}}"#),
            )
        } else {
            (200, "{}".to_string())
        }
    });
    let (_dir, config) = make_config(&baseurl);
    let out = TempDir::new().expect("tempdir");
    let target = out.path().join("category-4");
    let (output, ok) = run_dsc(
        &["category", "pull", "mock", "4", target.to_str().unwrap()],
        &config,
    );
    assert!(
        !ok,
        "category pull against a server that never stops paginating should fail, not hang or succeed"
    );
    assert!(
        output.contains("category pagination exceeded") && output.contains("pages"),
        "expected a clear pagination-budget error, got: {output}"
    );
}

// ─── P26: deleted-topic pagination has a page budget ──────────────────────

#[test]
fn deleted_topic_pagination_exceeding_the_page_budget_fails_with_a_clear_error() {
    let counter = Arc::new(AtomicUsize::new(0));
    let baseurl = start_mock(move |path| {
        if path.starts_with("/latest.json") {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            (
                200,
                format!(
                    r#"{{"topic_list":{{"topics":[],"more_topics_url":"/latest.json?status=deleted&page={n}"}}}}"#
                ),
            )
        } else {
            (200, "{}".to_string())
        }
    });
    let (_dir, config) = make_config(&baseurl);
    let (output, ok) = run_dsc(&["topic", "list", "mock", "--deleted"], &config);
    assert!(
        !ok,
        "deleted-topic list against a server that never stops paginating should fail, not hang or succeed"
    );
    assert!(
        output.contains("deleted-topic pagination exceeded") && output.contains("pages"),
        "expected a clear pagination-budget error, got: {output}"
    );
}

// ─── P26: private-message pagination has a page budget ───────────────────

#[test]
fn private_message_pagination_exceeding_the_page_budget_fails_with_a_clear_error() {
    let counter = Arc::new(AtomicUsize::new(0));
    let baseurl = start_mock(move |path| {
        if path.starts_with("/topics/private-messages/tester") {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            (
                200,
                format!(
                    r#"{{"topic_list":{{"topics":[],"more_topics_url":"/topics/private-messages/tester?page={n}"}}}}"#
                ),
            )
        } else {
            (200, "{}".to_string())
        }
    });
    let (_dir, config) = make_config(&baseurl);
    let (output, ok) = run_dsc(&["pm", "list", "mock", "tester"], &config);
    assert!(
        !ok,
        "PM list against a server that never stops paginating should fail, not hang or succeed"
    );
    assert!(
        output.contains("private-message pagination exceeded") && output.contains("pages"),
        "expected a clear pagination-budget error, got: {output}"
    );
}

// ─── P26: group fallback-listing pagination has a page budget ────────────

#[test]
fn group_fallback_pagination_exceeding_the_page_budget_fails_with_a_clear_error() {
    let counter = Arc::new(AtomicUsize::new(0));
    let baseurl = start_mock(move |path| {
        if path == "/admin/groups.json" {
            // 404 forces the fallback /groups.json pagination path.
            (404, r#"{"error":"not found"}"#.to_string())
        } else if path.starts_with("/groups.json") {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            (
                200,
                format!(
                    r#"{{"groups":[{{"id":1,"name":"g"}}],"load_more_groups":"/groups.json?page={n}"}}"#
                ),
            )
        } else {
            (200, "{}".to_string())
        }
    });
    let (_dir, config) = make_config(&baseurl);
    let (output, ok) = run_dsc(&["group", "list", "mock"], &config);
    assert!(
        !ok,
        "group list against a server that never stops paginating should fail, not hang or succeed"
    );
    assert!(
        output.contains("groups pagination exceeded") && output.contains("pages"),
        "expected a clear pagination-budget error, got: {output}"
    );
}

// ─── P22/P26: unbounded user-activity history has an item budget ─────────

#[test]
fn unbounded_user_activity_exceeding_the_item_budget_fails_with_a_clear_error() {
    // One response carrying more rows than the budget is enough to prove the
    // cap is enforced on cumulative collected items, not just request count.
    const OVER_BUDGET: usize = 100_001;
    let huge_page: String = (0..OVER_BUDGET)
        .map(|i| {
            format!(r#"{{"action_type":4,"created_at":"2026-01-01T00:00:00.000Z","topic_id":{i}}}"#)
        })
        .collect::<Vec<_>>()
        .join(",");
    let huge_body = format!(r#"{{"user_actions":[{huge_page}]}}"#);

    let baseurl = start_mock(move |path| {
        if path.starts_with("/user_actions.json") {
            (200, huge_body.clone())
        } else {
            (200, "{}".to_string())
        }
    });
    let (_dir, config) = make_config(&baseurl);
    // No --limit/--since: this is the "fetch everything" path the budget guards.
    let (output, ok) = run_dsc(&["user", "activity", "mock", "tester"], &config);
    assert!(
        !ok,
        "unbounded user activity against a server returning unbounded history should fail, not grow without limit"
    );
    assert!(
        output.contains("activity history exceeded") && output.contains("items"),
        "expected a clear item-budget error, got: {output}"
    );
}

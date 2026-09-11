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
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
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

fn run_dsc(args: &[&str], config: &Path) -> (String, bool) {
    // File-backed output avoids pipe backpressure while polling the deadline.
    let dir = TempDir::new().unwrap();
    let stdout = dir.path().join("stdout");
    let stderr = dir.path().join("stderr");
    let mut child = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .args(args)
        .env("DSC_CONFIG", config)
        .stdout(Stdio::from(std::fs::File::create(&stdout).unwrap()))
        .stderr(Stdio::from(std::fs::File::create(&stderr).unwrap()))
        .spawn()
        .expect("running dsc");
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("dsc {args:?} did not terminate within 60 seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stdout = std::fs::read_to_string(stdout).unwrap();
    let stderr = std::fs::read_to_string(stderr).unwrap();
    if !status.success() {
        assert!(stdout.is_empty(), "failed listing emitted partial stdout");
    }
    (format!("{stdout}{stderr}"), status.success())
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

fn activity_body(count: usize) -> String {
    let rows: Vec<_> = (0..count)
        .map(|id| {
            serde_json::json!({
                "action_type": 4, "created_at": "2026-01-01T00:00:00Z", "topic_id": id
            })
        })
        .collect();
    serde_json::json!({"user_actions": rows}).to_string()
}

#[test]
fn activity_short_pages_do_not_skip_rows_and_zero_limit_does_not_fetch() {
    let calls = Arc::new(AtomicUsize::new(0));
    let requests = calls.clone();
    let baseurl = start_mock(move |path| {
        requests.fetch_add(1, Ordering::SeqCst);
        if path.ends_with("offset=0") || path.ends_with("offset=10") {
            (200, activity_body(10))
        } else {
            (200, activity_body(0))
        }
    });
    let (_dir, config) = make_config(&baseurl);
    let (output, ok) = run_dsc(
        &[
            "user", "activity", "mock", "tester", "--limit", "0", "-f", "json",
        ],
        &config,
    );
    assert!(ok, "{output}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap(),
        serde_json::json!([])
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let (output, ok) = run_dsc(
        &["user", "activity", "mock", "tester", "-f", "json"],
        &config,
    );
    assert!(ok, "{output}");
    assert_eq!(
        serde_json::from_str::<Vec<serde_json::Value>>(&output)
            .unwrap()
            .len(),
        20
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn activity_exact_budget_succeeds_and_explicit_limit_can_exceed_it() {
    for (count, extra) in [
        (100_000, vec![]),
        (100_000, vec!["--since", "2025-01-01"]),
        (100_001, vec!["--limit", "100001"]),
    ] {
        let first = activity_body(50_000);
        let second = activity_body(count - 50_000);
        let cutoff = extra.contains(&"--since");
        let baseurl = start_mock(move |path| {
            let body = if path.ends_with("offset=0") {
                first.clone()
            } else if path.ends_with("offset=50000") {
                second.clone()
            } else if cutoff {
                activity_body(1).replace("2026-01-01", "2024-01-01")
            } else {
                activity_body(0)
            };
            (200, body)
        });
        let (_dir, config) = make_config(&baseurl);
        let mut args = vec!["user", "activity", "mock", "tester", "-f", "json"];
        args.extend(extra);
        let (output, ok) = run_dsc(&args, &config);
        assert!(ok, "{output}");
        assert_eq!(
            serde_json::from_str::<Vec<serde_json::Value>>(&output)
                .unwrap()
                .len(),
            count
        );
    }
}

#[test]
fn activity_since_does_not_bypass_item_budget_even_on_cutoff_page() {
    let mut body: serde_json::Value = serde_json::from_str(&activity_body(100_001)).unwrap();
    body["user_actions"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "action_type": 4, "created_at": "2024-01-01T00:00:00Z", "topic_id": 200000
        }));
    let baseurl = start_mock(move |_| (200, body.to_string()));
    let (_dir, config) = make_config(&baseurl);
    let (output, ok) = run_dsc(
        &[
            "user",
            "activity",
            "mock",
            "tester",
            "--since",
            "2025-01-01",
        ],
        &config,
    );
    assert!(!ok, "over-budget cutoff page must fail");
    assert!(
        output.contains("activity history exceeded 100000 items"),
        "{output}"
    );
}

#[test]
fn group_empty_page_terminates_despite_continuation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let requests = calls.clone();
    let baseurl = start_mock(move |path| {
        requests.fetch_add(1, Ordering::SeqCst);
        match path {
            "/admin/groups.json" => (404, "{}".into()),
            "/groups.json" => (
                200,
                r#"{"groups":[{"id":7,"name":"last"}],"load_more_groups":"/groups.json?page=1"}"#
                    .into(),
            ),
            _ => (
                200,
                r#"{"groups":[],"load_more_groups":"/groups.json?page=2"}"#.into(),
            ),
        }
    });
    let (_dir, config) = make_config(&baseurl);
    let (output, ok) = run_dsc(&["group", "list", "mock", "-f", "json"], &config);
    assert!(ok, "{output}");
    assert!(output.contains("last"), "result was lost: {output}");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn page_boundaries_cycles_and_later_errors_for_every_path_loop() {
    for (kind, route) in [
        ("category", "/c/4.json"),
        ("deleted", "/latest.json?status=deleted&per_page=100"),
        ("pm", "/topics/private-messages/tester.json"),
        ("groups", "/groups.json"),
    ] {
        for scenario in ["999", "1000", "1001", "cycle", "http", "json"] {
            let calls = Arc::new(AtomicUsize::new(0));
            let requests = calls.clone();
            let baseurl = start_mock(move |path| {
                if path == "/admin/groups.json" {
                    return (404, "{}".into());
                }
                let n = requests.fetch_add(1, Ordering::SeqCst) + 1;
                // Fail closed if the implementation loses its budget guard.
                if n > 1001 || (scenario == "http" && n == 2) {
                    return (403, r#"{"errors":["forbidden"]}"#.into());
                }
                if scenario == "json" && n == 2 {
                    return (200, "not json".into());
                }
                let next = if scenario == "cycle" {
                    Some(route.replace(".json", ""))
                } else if scenario.parse::<usize>().ok() == Some(n) {
                    None
                } else {
                    Some(format!(
                        "{route}{}page={n}",
                        if route.contains('?') { '&' } else { '?' }
                    ))
                };
                let body = if kind == "groups" {
                    serde_json::json!({"groups": [{"id": n,"name": "g"}], "load_more_groups": next})
                } else {
                    serde_json::json!({"topic_list": {"topics": [], "more_topics_url": next}})
                };
                (200, body.to_string())
            });
            let client = dsc::api::DiscourseClient::new(&dsc::config::DiscourseConfig {
                name: "mock".into(),
                baseurl,
                ..Default::default()
            })
            .unwrap();
            let result = match kind {
                "category" => client.fetch_category(4).map(|_| ()),
                "deleted" => client.list_deleted_topics(None).map(|_| ()),
                "pm" => client.list_private_messages("tester", "inbox").map(|_| ()),
                _ => client.fetch_groups().map(|_| ()),
            };
            let expected_calls = match scenario {
                "999" | "1000" => {
                    assert!(result.is_ok(), "{kind}/{scenario}: {result:?}");
                    scenario.parse().unwrap()
                }
                _ => {
                    let error = format!("{:#}", result.unwrap_err());
                    let expected = match scenario {
                        "1001" => "pagination exceeded 1000 pages",
                        "cycle" => "loop detected",
                        "http" => "403",
                        _ => "expected ident",
                    };
                    assert!(error.contains(expected), "{kind}/{scenario}: {error}");
                    match scenario {
                        "1001" => 1000,
                        "cycle" => 1,
                        _ => 2,
                    }
                }
            };
            assert_eq!(
                calls.load(Ordering::SeqCst),
                expected_calls,
                "{kind}/{scenario}"
            );
        }
    }
}

#[test]
fn activity_later_http_and_parse_errors_do_not_emit_partial_results() {
    for code in [403, 200] {
        let baseurl = start_mock(move |path| {
            if path.ends_with("offset=0") {
                (200, activity_body(10))
            } else {
                (code, "not json".into())
            }
        });
        let (_dir, config) = make_config(&baseurl);
        let (output, ok) = run_dsc(&["user", "activity", "mock", "tester"], &config);
        assert!(!ok);
        assert!(
            output.contains(if code == 403 {
                "403"
            } else {
                "parsing user actions"
            }),
            "{output}"
        );
    }
}

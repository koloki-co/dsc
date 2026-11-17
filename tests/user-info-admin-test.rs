// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Coverage for `dsc user info --admin` (R82): exact account resolution,
//! the allowlisted admin view, separately opted-in IP and email reads,
//! preserved null/unknown states, and the `--dry-run` refusal of the
//! audited email read.

mod common;
use common::*;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

struct MockResponse {
    status: &'static str,
    body: String,
}

fn ok(body: impl Into<String>) -> MockResponse {
    MockResponse {
        status: "200 OK",
        body: body.into(),
    }
}

/// A mock Discourse answering requests in order, recording `METHOD path`.
fn start_mock(responses: Vec<MockResponse>) -> (String, Arc<Mutex<Vec<String>>>) {
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
            handle(stream, response, &thread_requests);
        }
    });
    (format!("http://{addr}"), requests)
}

fn handle(mut stream: TcpStream, response: MockResponse, requests: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("request header");
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
    }
    requests
        .lock()
        .expect("request log")
        .push(format!("{method} {path}"));
    let raw = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.status,
        response.body.len(),
        response.body
    );
    let _ = stream.write_all(raw.as_bytes());
    let _ = stream.flush();
}

fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{url}\"\napikey = \"key\"\napi_username = \"system\"\n"
        ),
    )
}

fn public_profile() -> String {
    r#"{"user":{"id":42,"username":"Jane","name":"Jane Doe","email":null,"trust_level":2}}"#
        .to_string()
}

/// Admin detail carrying sensitive fields that must never leak by default.
/// `suspended_till` is an explicit null; `api_key_count` is absent.
fn admin_detail() -> String {
    r#"{
        "id": 42,
        "username": "Jane",
        "name": "Jane Doe",
        "active": true,
        "admin": false,
        "moderator": true,
        "trust_level": 2,
        "created_at": "2020-01-01T00:00:00.000Z",
        "last_seen_at": "2026-11-10T09:00:00.000Z",
        "suspended_till": null,
        "second_factor_enabled": false,
        "days_visited": 17,
        "email": "jane@example.com",
        "secondary_emails": ["alt@example.com"],
        "ip_address": "203.0.113.5",
        "registration_ip_address": "198.51.100.7",
        "single_sign_on_record": {"external_email": "jane@idp.example"},
        "external_ids": {"google": "abc"},
        "associated_accounts": [{"name": "google", "description": "jane@example.com"}],
        "user_fields": {"1": "private"}
    }"#
    .to_string()
}

fn run_json(args: &[&str], config: &std::path::Path) -> Value {
    let output = run_dsc(args, config);
    assert!(
        output.status.success(),
        "dsc {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

#[test]
fn admin_view_is_allowlisted_and_preserves_unknowns() {
    let (url, requests) = start_mock(vec![ok(public_profile()), ok(admin_detail())]);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let view = run_json(
        &[
            "user", "info", "mock", "jane", "--admin", "--format", "json",
        ],
        &config,
    );

    assert_eq!(
        *requests.lock().unwrap(),
        vec!["GET /u/jane.json", "GET /admin/users/42.json"]
    );
    assert_eq!(view["user_id"], 42);
    assert_eq!(view["username"], "Jane");
    let account = view["account"].as_object().expect("account object");
    assert_eq!(account["moderator"], true);
    assert_eq!(account["days_visited"], 17);
    assert_eq!(account["second_factor_enabled"], false);
    // Explicit null is preserved, not coerced or dropped.
    assert!(account.contains_key("suspended_till"));
    assert!(account["suspended_till"].is_null());
    // Absent fields are reported as unknown, not defaulted.
    assert!(!account.contains_key("api_key_count"));
    let unavailable: Vec<&str> = view["unavailable"]
        .as_array()
        .expect("unavailable list")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(unavailable.contains(&"api_key_count"));

    let rendered = view.to_string();
    for secret in [
        "jane@example.com",
        "alt@example.com",
        "203.0.113.5",
        "198.51.100.7",
        "jane@idp.example",
        "external_ids",
        "user_fields",
        "associated_accounts",
    ] {
        assert!(
            !rendered.contains(secret),
            "default admin view leaked {secret}: {rendered}"
        );
    }
    assert!(view.get("email").is_none());
    let endpoints = view["source"]["endpoints"].as_array().expect("endpoints");
    assert_eq!(endpoints.len(), 2);
    assert!(view["source"]["retrieved_at"].is_string());
}

#[test]
fn include_ip_adds_only_ip_fields() {
    let (url, requests) = start_mock(vec![ok(public_profile()), ok(admin_detail())]);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let view = run_json(
        &[
            "user",
            "info",
            "mock",
            "jane",
            "--admin",
            "--include-ip",
            "--format",
            "json",
        ],
        &config,
    );
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(view["account"]["ip_address"], "203.0.113.5");
    assert_eq!(view["account"]["registration_ip_address"], "198.51.100.7");
    assert!(!view.to_string().contains("jane@example.com"));
}

#[test]
fn include_email_uses_audited_endpoint_and_drops_associated_accounts() {
    let emails = r#"{"email":"jane@example.com","secondary_emails":[],"associated_accounts":[{"name":"google","description":"jane@example.com"}]}"#;
    let (url, requests) = start_mock(vec![ok(public_profile()), ok(admin_detail()), ok(emails)]);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let view = run_json(
        &[
            "user",
            "info",
            "mock",
            "jane",
            "--admin",
            "--include-email",
            "--format",
            "json",
        ],
        &config,
    );
    assert_eq!(
        *requests.lock().unwrap(),
        vec![
            "GET /u/jane.json",
            "GET /admin/users/42.json",
            "GET /u/Jane/emails.json"
        ]
    );
    let email = view["email"].as_object().expect("email section");
    assert_eq!(email["email"], "jane@example.com");
    assert_eq!(email["secondary_emails"], serde_json::json!([]));
    assert!(!email.contains_key("associated_accounts"));
    assert!(!email.contains_key("unconfirmed_emails"));
    let unavailable = view["unavailable"].as_array().expect("unavailable");
    assert!(unavailable.contains(&Value::from("email.unconfirmed_emails")));
    // IP stays out without its own opt-in.
    assert!(!view.to_string().contains("203.0.113.5"));
}

#[test]
fn dry_run_refuses_audited_email_read_before_any_request() {
    let (url, requests) = start_mock(vec![ok(public_profile()), ok(admin_detail())]);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(
        &[
            "--dry-run",
            "user",
            "info",
            "mock",
            "jane",
            "--admin",
            "--include-email",
        ],
        &config,
    );
    assert!(!output.status.success(), "dry-run email read must refuse");
    assert!(String::from_utf8_lossy(&output.stderr).contains("--include-email"));
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn sensitive_opt_ins_require_admin() {
    let dir = TempDir::new().expect("tempdir");
    let config = config_for("http://127.0.0.1:9", &dir);
    for flag in ["--include-email", "--include-ip"] {
        let output = run_dsc(&["user", "info", "mock", "jane", flag], &config);
        assert!(!output.status.success(), "{flag} without --admin must fail");
    }
}

#[test]
fn refuses_when_admin_detail_identifies_another_account() {
    let other = r#"{"id":42,"username":"someone_else","admin":false}"#;
    let (url, _requests) = start_mock(vec![ok(public_profile()), ok(other)]);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["user", "info", "mock", "jane", "--admin"], &config);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

#[test]
fn missing_admin_permission_fails_with_context() {
    let (url, _requests) = start_mock(vec![
        ok(public_profile()),
        MockResponse {
            status: "403 Forbidden",
            body: r#"{"errors":["You are not permitted to view the requested resource."]}"#
                .to_string(),
        },
    ]);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["user", "info", "mock", "jane", "--admin"], &config);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("admin API key"), "stderr: {stderr}");
}

#[test]
fn without_admin_the_public_contract_is_unchanged() {
    let (url, requests) = start_mock(vec![ok(public_profile())]);
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let view = run_json(
        &["user", "info", "mock", "jane", "--format", "json"],
        &config,
    );
    assert_eq!(*requests.lock().unwrap(), vec!["GET /u/jane.json"]);
    assert_eq!(view["id"], 42);
    assert!(view.get("account").is_none());
}

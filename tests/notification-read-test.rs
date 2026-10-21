// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Success-path coverage for `dsc notification read`. The shared `--dry-run`
//! no-mutation registry already proves this never hits the server under
//! `--dry-run`; these tests prove `--id`/`--type`/`--all` each send the
//! exact PUT Discourse expects, since a bug swapping which selector fires
//! would mark the wrong notification(s) read with no test catching it.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn start_mock() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_requests = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle_request(stream, &thread_requests);
        }
    });
    (format!("http://{addr}"), requests)
}

fn handle_request(mut stream: TcpStream, requests: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    requests
        .lock()
        .expect("request log")
        .push(request_line.trim().to_string());
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("request header");
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
    }
    let body = "{\"success\":\"OK\"}";
    let raw = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(raw.as_bytes()).expect("mock response");
}

fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{url}\"\napikey = \"key\"\napi_username = \"system\"\n"
        ),
    )
}

#[test]
fn read_by_id_sends_the_id_query_only() {
    let (url, requests) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["notification", "read", "mock", "--id", "123"],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let seen = requests.lock().expect("request log");
    assert_eq!(
        *seen,
        vec!["PUT /notifications/mark-read.json?id=123 HTTP/1.1".to_string()]
    );
}

#[test]
fn read_by_type_sends_dismiss_types_only() {
    let (url, requests) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["notification", "read", "mock", "--type", "liked,mentioned"],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let seen = requests.lock().expect("request log");
    assert_eq!(
        *seen,
        vec![
            "PUT /notifications/mark-read.json?dismiss_types=liked%2Cmentioned HTTP/1.1"
                .to_string()
        ]
    );
}

#[test]
fn read_all_sends_no_query_string() {
    let (url, requests) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["notification", "read", "mock", "--all"],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let seen = requests.lock().expect("request log");
    assert_eq!(
        *seen,
        vec!["PUT /notifications/mark-read.json HTTP/1.1".to_string()]
    );
}

#[test]
fn read_rejects_an_unknown_notification_type_before_any_request() {
    let (url, requests) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["notification", "read", "mock", "--type", "bogus"],
        &config_for(&url, &dir),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown notification type"));
    assert!(requests.lock().expect("request log").is_empty());
}

#[test]
fn read_rejects_no_selector_before_any_request() {
    let (url, requests) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(&["notification", "read", "mock"], &config_for(&url, &dir));
    assert!(!output.status.success());
    assert!(requests.lock().expect("request log").is_empty());
}

#[test]
fn read_rejects_combining_id_and_all_before_any_request() {
    let (url, requests) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["notification", "read", "mock", "--id", "1", "--all"],
        &config_for(&url, &dir),
    );
    assert!(!output.status.success());
    assert!(requests.lock().expect("request log").is_empty());
}

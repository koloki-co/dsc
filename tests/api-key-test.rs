// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

struct MockResponse {
    status: &'static str,
    body: &'static str,
}

fn start_mock(
    responses: Vec<MockResponse>,
) -> (String, Arc<Mutex<Vec<String>>>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_requests = Arc::clone(&requests);
    let handle = std::thread::spawn(move || {
        let mut responses = VecDeque::from(responses);
        while let Some(response) = responses.pop_front() {
            let (stream, _) = listener.accept().expect("accept mock request");
            handle_request(stream, response, &thread_requests);
        }
    });
    (format!("http://{addr}"), requests, handle)
}

fn handle_request(mut stream: TcpStream, response: MockResponse, requests: &Mutex<Vec<String>>) {
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
    let raw = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.status,
        response.body.len(),
        response.body
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

// Regression test: `dsc api-key revoke` used to send DELETE to the plain
// `/admin/api/keys/:id.json` endpoint - Discourse's *permanent* destroy
// route - instead of the reversible `POST .../revoke` endpoint, silently
// destroying the key record whenever a user asked only to revoke it.
#[test]
fn api_key_revoke_uses_reversible_revoke_endpoint() {
    let responses = vec![MockResponse {
        status: "200 OK",
        body: "{}",
    }];
    let (url, requests, handle) = start_mock(responses);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["api-key", "revoke", "mock", "42"],
        &config_for(&url, &dir),
    );
    handle.join().expect("mock thread");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Revoked api key id:42"));
    let seen = requests.lock().expect("request log");
    assert!(seen.contains(&"POST /admin/api/keys/42/revoke.json HTTP/1.1".to_string()));
    assert!(!seen.iter().any(|line| line.starts_with("DELETE ")));
}

#[test]
fn api_key_undo_revoke_reactivates_the_key() {
    let responses = vec![MockResponse {
        status: "200 OK",
        body: "{}",
    }];
    let (url, requests, handle) = start_mock(responses);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["api-key", "undo-revoke", "mock", "42"],
        &config_for(&url, &dir),
    );
    handle.join().expect("mock thread");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Undid revoke of api key id:42"));
    assert!(
        requests
            .lock()
            .expect("request log")
            .contains(&"POST /admin/api/keys/42/undo-revoke.json HTTP/1.1".to_string())
    );
}

#[test]
fn api_key_delete_permanently_removes_via_delete_verb() {
    let responses = vec![MockResponse {
        status: "200 OK",
        body: "{}",
    }];
    let (url, requests, handle) = start_mock(responses);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["api-key", "delete", "mock", "42"],
        &config_for(&url, &dir),
    );
    handle.join().expect("mock thread");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Permanently deleted api key id:42"));
    assert!(
        requests
            .lock()
            .expect("request log")
            .contains(&"DELETE /admin/api/keys/42.json HTTP/1.1".to_string())
    );
}

#[test]
fn api_key_delete_dry_run_makes_no_request() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"mock\"\nbaseurl = \"http://127.0.0.1:1\"\napikey = \"key\"\napi_username = \"system\"\n",
    );
    let output = run_dsc(&["-n", "api-key", "delete", "mock", "42"], &config_path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("would permanently delete api key id:42")
    );
}

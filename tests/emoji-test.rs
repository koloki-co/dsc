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

const PARTYTIME: &str = r#"[{"name":"partytime","url":"/uploads/partytime.png"}]"#;

#[test]
fn emoji_delete_uses_current_endpoint_and_verifies_absence() {
    let responses = vec![
        MockResponse {
            status: "404 Not Found",
            body: "{}",
        },
        MockResponse {
            status: "200 OK",
            body: PARTYTIME,
        },
        MockResponse {
            status: "200 OK",
            body: r#"{"success":"OK"}"#,
        },
        MockResponse {
            status: "404 Not Found",
            body: "{}",
        },
        MockResponse {
            status: "200 OK",
            body: "[]",
        },
    ];
    let (url, requests, handle) = start_mock(responses);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["emoji", "delete", "mock", "partytime"],
        &config_for(&url, &dir),
    );
    handle.join().expect("mock thread");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("deleted emoji partytime"));
    assert!(
        requests
            .lock()
            .unwrap()
            .contains(&"DELETE /admin/config/emoji/partytime.json HTTP/1.1".to_string())
    );
}

#[test]
fn emoji_delete_falls_back_to_legacy_endpoint() {
    let responses = vec![
        MockResponse {
            status: "200 OK",
            body: PARTYTIME,
        },
        MockResponse {
            status: "404 Not Found",
            body: "{}",
        },
        MockResponse {
            status: "200 OK",
            body: r#"{"success":"OK"}"#,
        },
        MockResponse {
            status: "200 OK",
            body: "[]",
        },
    ];
    let (url, requests, handle) = start_mock(responses);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["emoji", "delete", "mock", "partytime"],
        &config_for(&url, &dir),
    );
    handle.join().expect("mock thread");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        requests
            .lock()
            .unwrap()
            .contains(&"DELETE /admin/customize/emojis/partytime.json HTTP/1.1".to_string())
    );
}

#[test]
fn emoji_delete_rejects_false_success_when_name_remains() {
    let responses = vec![
        MockResponse {
            status: "200 OK",
            body: PARTYTIME,
        },
        MockResponse {
            status: "200 OK",
            body: r#"{"success":"OK"}"#,
        },
        MockResponse {
            status: "200 OK",
            body: PARTYTIME,
        },
    ];
    let (url, _, handle) = start_mock(responses);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["emoji", "delete", "mock", "partytime"],
        &config_for(&url, &dir),
    );
    handle.join().expect("mock thread");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("still present after Discourse accepted the delete"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("deleted emoji"));
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn emoji_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_emoji_list: list custom emojis");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["emoji", "list", &test.name], &config_path);
    assert!(output.status.success(), "emoji list failed");
    assert!(!output.stdout.is_empty(), "emoji list produced no output");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn emoji_list_inline() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_emoji_list_inline: list custom emojis inline");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["emoji", "list", "--inline", &test.name], &config_path);
    assert!(output.status.success(), "emoji list inline failed");
}

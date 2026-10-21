// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Success-path coverage for `dsc post delete`/`post move`. The shared
//! `--dry-run` no-mutation registry already proves these never hit the
//! server under `--dry-run`; these tests prove the real DELETE/POST fires
//! against the exact post/topic IDs, since a silent off-by-one or
//! swapped-argument bug would otherwise go undetected.

mod common;
use common::*;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

struct CapturedRequest {
    method: String,
    path: String,
    body: String,
}

struct MockResponse {
    status: &'static str,
    body: &'static str,
}

fn start_mock(responses: Vec<MockResponse>) -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
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
            handle_request(stream, response, &thread_requests);
        }
    });
    (format!("http://{addr}"), requests)
}

fn handle_request(
    mut stream: TcpStream,
    response: MockResponse,
    requests: &Mutex<Vec<CapturedRequest>>,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("request header");
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body_bytes = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body_bytes).expect("request body");
    }
    let body = String::from_utf8_lossy(&body_bytes).into_owned();

    requests
        .lock()
        .expect("request log")
        .push(CapturedRequest { method, path, body });

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

/// Decode one `application/x-www-form-urlencoded` body into a lookup map,
/// retaining every value for a repeated key (e.g. `post_ids[]`).
fn decode_form_multi(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode_form_component(key), decode_form_component(value))
        })
        .collect()
}

fn decode_form_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn post_delete_sends_a_delete_to_the_exact_post_id() {
    let (url, requests) = start_mock(vec![MockResponse {
        status: "200 OK",
        body: "{}",
    }]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(&["post", "delete", "mock", "555"], &config_for(&url, &dir));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Post 555 deleted"));

    let seen = requests.lock().expect("request log");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "DELETE");
    assert_eq!(seen[0].path, "/posts/555.json");
}

#[test]
fn post_move_sends_the_exact_source_topic_post_id_and_destination() {
    let (url, requests) = start_mock(vec![
        MockResponse {
            status: "200 OK",
            body: r#"{"id":555,"topic_id":10,"post_number":3}"#,
        },
        MockResponse {
            status: "200 OK",
            body: r#"{"url":"/t/some-topic/20"}"#,
        },
    ]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["post", "move", "mock", "555", "--to-topic", "20"],
        &config_for(&url, &dir),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Moved post 555 → topic 20"));

    let seen = requests.lock().expect("request log");
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/posts/555.json?include_raw=1");
    assert_eq!(seen[1].method, "POST");
    assert_eq!(
        seen[1].path, "/t/10/move-posts.json",
        "move-posts must target the post's actual CURRENT topic (10), not the destination"
    );
    let form = decode_form_multi(&seen[1].body);
    assert!(form.contains(&("destination_topic_id".to_string(), "20".to_string())));
    assert!(form.contains(&("post_ids[]".to_string(), "555".to_string())));
}

#[test]
fn post_move_refuses_when_already_in_the_destination_topic() {
    let (url, requests) = start_mock(vec![MockResponse {
        status: "200 OK",
        body: r#"{"id":555,"topic_id":20,"post_number":3}"#,
    }]);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc(
        &["post", "move", "mock", "555", "--to-topic", "20"],
        &config_for(&url, &dir),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already in topic"));

    let seen = requests.lock().expect("request log");
    assert_eq!(
        seen.len(),
        1,
        "a no-op move must refuse before any move-posts request"
    );
}

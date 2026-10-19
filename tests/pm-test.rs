// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// One captured request: method, path, and decoded form body (empty for
/// GET). Shared across every `pm` request in a test run.
struct CapturedRequest {
    method: String,
    path: String,
    body: String,
}

fn start_mock(response_body: &'static str) -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_requests = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle_request(stream, response_body, &thread_requests);
        }
    });
    (format!("http://{addr}"), requests)
}

fn handle_request(
    mut stream: TcpStream,
    response_body: &str,
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

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    stream
        .write_all(response.as_bytes())
        .expect("mock response");
}

/// Decode one `application/x-www-form-urlencoded` body into a lookup map.
/// Avoids a new dependency for this one test's assertions.
fn decode_form(body: &str) -> std::collections::HashMap<String, String> {
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

fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{url}\"\napikey = \"key\"\napi_username = \"system\"\n"
        ),
    )
}

/// `pm send` must encode the exact recipients/title/body Discourse's
/// `archetype=private_message` create-post endpoint expects, not just
/// avoid mutating under `--dry-run` (already covered separately).
#[test]
fn pm_send_posts_the_exact_recipients_title_and_body() {
    let (url, requests) = start_mock(r#"{"id":1,"topic_id":4242}"#);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc_with_stdin(
        &[
            "pm",
            "send",
            "mock",
            "alice,bob",
            "--title",
            "Heads up",
            "-",
        ],
        &config_for(&url, &dir),
        "Please see the attached report.\n",
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("topic id 4242"));

    let seen = requests.lock().expect("request log");
    let create = seen
        .iter()
        .find(|r| r.method == "POST" && r.path == "/posts.json")
        .expect("expected a POST /posts.json request");
    let form = decode_form(&create.body);
    assert_eq!(form.get("title").map(String::as_str), Some("Heads up"));
    assert_eq!(
        form.get("raw").map(String::as_str),
        Some("Please see the attached report.\n")
    );
    assert_eq!(
        form.get("archetype").map(String::as_str),
        Some("private_message")
    );
    assert_eq!(
        form.get("target_recipients").map(String::as_str),
        Some("alice,bob")
    );
}

#[test]
fn pm_send_rejects_an_empty_title() {
    let (url, requests) = start_mock(r#"{"id":1,"topic_id":1}"#);
    let dir = TempDir::new().expect("tempdir");
    let output = run_dsc_with_stdin(
        &["pm", "send", "mock", "alice", "--title", "  ", "-"],
        &config_for(&url, &dir),
        "body",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("title is empty"));
    assert!(
        requests.lock().expect("request log").is_empty(),
        "an invalid title must not reach the server"
    );
}

fn run_dsc_with_stdin(
    args: &[&str],
    config_path: &std::path::Path,
    stdin: &str,
) -> std::process::Output {
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .arg("-c")
        .arg(config_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dsc");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("dsc output")
}

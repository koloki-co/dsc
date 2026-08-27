// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! `--render` integration coverage for `topic push`, `topic reply`,
//! `topic new`, and `category push` (R29 Phase 2). `topic reply`/`topic new`
//! render before their dry-run early return, so those are exercised offline;
//! `topic push`/`category push` always fetch remote state first, so those
//! run against a small local mock Discourse.

mod common;
use common::*;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

// ─── mock Discourse (trimmed to what topic/category push need) ────────────

const POST: &str = r#"{"id":1,"topic_id":7,"post_number":1,"raw":"old body","cooked":"<p>old body</p>","username":"tester","created_at":"2026-01-01T00:00:00.000Z","category_id":4}"#;
const CATEGORY: &str = r#"{"id":4,"name":"Test","slug":"test","color":"0088CC","text_color":"FFFFFF","position":1,"description":"d","read_restricted":false,"permission":1,"topic_template":"","allowed_tags":[],"allowed_tag_groups":[]}"#;

fn get_body(path: &str) -> String {
    let p = path.split('?').next().unwrap_or(path);
    if p == "/t/7.json" {
        return format!(
            r#"{{"id":7,"title":"Test topic","slug":"test-topic","category_id":4,"posts_count":1,"created_at":"2026-01-01T00:00:00.000Z","post_stream":{{"posts":[{POST}],"stream":[1]}}}}"#
        );
    }
    if p == "/c/4.json" {
        return format!(
            r#"{{"category":{CATEGORY},"topic_list":{{"topics":[{{"id":7,"title":"Test topic","slug":"test-topic","posts_count":1,"category_id":4}}],"more_topics_url":null}}}}"#
        );
    }
    "{}".to_string()
}

fn handle(mut stream: TcpStream, log: &Arc<Mutex<Vec<String>>>) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();

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
    let mut request_body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut request_body);
    }
    let request_body = String::from_utf8_lossy(&request_body);

    log.lock()
        .expect("mock log poisoned")
        .push(format!("{method} {path}\n{request_body}"));

    let body = if method == "GET" {
        get_body(&path)
    } else {
        r#"{"success":"OK","id":1,"topic_id":7}"#.to_string()
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn start_mock() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let log = Arc::new(Mutex::new(Vec::new()));
    let thread_log = Arc::clone(&log);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &thread_log);
        }
    });
    (format!("http://{addr}"), log)
}

fn find_request<'a>(log: &'a [String], prefix: &str) -> &'a str {
    log.iter()
        .find(|entry| entry.starts_with(prefix))
        .unwrap_or_else(|| panic!("no request starting with {prefix:?} in {log:?}"))
}

// ─── topic push / category push: rendering happens before the request ─────

#[test]
fn topic_push_render_substitutes_template_variables_before_sending() {
    vprintln("topic_push_render: --render fills placeholders before the PUT");
    let (baseurl, log) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"mock-key\"\napi_username = \"tester\"\n\n[discourse.template]\norganisation = \"Rendered\"\n"
        ),
    );
    let file_path = dir.path().join("push.md");
    fs::write(&file_path, "Contact {{ organisation }} for help.\n").expect("write file");

    let output = run_dsc(
        &[
            "topic",
            "push",
            "mock",
            "7",
            file_path.to_str().unwrap(),
            "--render",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic push --render failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let requests = log.lock().expect("mock log poisoned");
    let request = find_request(&requests, "PUT /posts/1.json\n");
    assert!(
        request.contains("Rendered"),
        "expected substituted variable in request: {request}"
    );
    assert!(
        !request.contains("organisation"),
        "unrendered placeholder leaked into request: {request}"
    );
}

#[test]
fn topic_push_without_render_sends_placeholders_verbatim() {
    vprintln("topic_push_no_render: default behaviour is unchanged");
    let (baseurl, log) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"mock-key\"\napi_username = \"tester\"\n\n[discourse.template]\norganisation = \"Rendered\"\n"
        ),
    );
    let file_path = dir.path().join("push.md");
    fs::write(&file_path, "Contact {{ organisation }} for help.\n").expect("write file");

    let output = run_dsc(
        &["topic", "push", "mock", "7", file_path.to_str().unwrap()],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic push failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let requests = log.lock().expect("mock log poisoned");
    let request = find_request(&requests, "PUT /posts/1.json\n");
    assert!(
        request.contains("organisation"),
        "expected the literal placeholder without --render: {request}"
    );
}

#[test]
fn category_push_render_substitutes_template_variables_per_file() {
    vprintln("category_push_render: --render fills placeholders in every pushed file");
    let (baseurl, log) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"mock-key\"\napi_username = \"tester\"\n\n[discourse.template]\norganisation = \"Rendered\"\n"
        ),
    );
    let catdir = dir.path().join("catdir");
    fs::create_dir_all(&catdir).expect("catdir");
    fs::write(
        catdir.join("t.md"),
        "---\ntopic_id: 7\n---\nNotice: {{ organisation }} update.\n",
    )
    .expect("write category file");

    let output = run_dsc(
        &[
            "category",
            "push",
            "mock",
            "4",
            catdir.to_str().unwrap(),
            "--render",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "category push --render failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let requests = log.lock().expect("mock log poisoned");
    let request = find_request(&requests, "PUT /posts/1.json\n");
    assert!(
        request.contains("Rendered"),
        "expected substituted variable in request: {request}"
    );
    assert!(
        !request.contains("organisation"),
        "unrendered placeholder leaked into request: {request}"
    );
}

// ─── topic new / topic reply: rendering happens before the dry-run return ─

#[test]
fn topic_reply_render_dry_run_previews_rendered_byte_count() {
    vprintln("topic_reply_render: --render -n previews the rendered body, no network");
    let dir = TempDir::new().expect("tempdir");
    let rendered = "Hi Acme, thanks for reporting this!";
    let file_path = dir.path().join("reply.md");
    fs::write(
        &file_path,
        "Hi {{ organisation }}, thanks for reporting this!",
    )
    .expect("write file");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"offline\"\nbaseurl = \"https://example.invalid\"\napikey = \"unused\"\napi_username = \"system\"\n\n[discourse.template]\norganisation = \"Acme\"\n",
    );

    let output = run_dsc(
        &[
            "-n",
            "topic",
            "reply",
            "offline",
            "123",
            file_path.to_str().unwrap(),
            "--render",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic reply --render -n failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!(
            "would reply to topic 123 with {} bytes",
            rendered.len()
        )),
        "expected a rendered byte count, got: {stdout}"
    );
}

#[test]
fn topic_new_render_dry_run_previews_rendered_byte_count() {
    vprintln("topic_new_render: --render -n previews the rendered body, no network");
    let dir = TempDir::new().expect("tempdir");
    let rendered = "Welcome to Acme's forum!";
    let file_path = dir.path().join("new.md");
    fs::write(&file_path, "Welcome to {{ organisation }}'s forum!").expect("write file");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"offline\"\nbaseurl = \"https://example.invalid\"\napikey = \"unused\"\napi_username = \"system\"\n\n[discourse.template]\norganisation = \"Acme\"\n",
    );

    let output = run_dsc(
        &[
            "-n",
            "topic",
            "new",
            "offline",
            "4",
            "--title",
            "Welcome",
            file_path.to_str().unwrap(),
            "--render",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic new --render -n failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("({} bytes of body)", rendered.len())),
        "expected a rendered byte count, got: {stdout}"
    );
}

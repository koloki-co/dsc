// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! R88 Phase 1: safe read-only LLM inspection against the documented
//! `/admin/plugins/discourse-ai/ai-llms.json` response shape.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

fn start_mock(body: &'static str, status: &'static str) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let address = listener.local_addr().expect("mock address");
    let requests = Arc::new(AtomicUsize::new(0));
    let thread_requests = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, body, status, &thread_requests);
        }
    });
    (format!("http://{address}"), requests)
}

fn handle(mut stream: TcpStream, body: &str, status: &str, requests: &AtomicUsize) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    while {
        let mut line = String::new();
        reader.read_line(&mut line).is_ok() && line != "\r\n" && line != "\n"
    } {}

    if request_line.starts_with("GET /admin/plugins/discourse-ai/ai-llms.json ") {
        requests.fetch_add(1, Ordering::Relaxed);
    }
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    )
}

const LLMS: &str = r#"{
  "ai_llms": [
    {
      "id": 9,
      "display_name": "Custom model",
      "name": "model-canary",
      "provider": "open_ai",
      "tokenizer": "DiscourseAi::Tokenizer::OpenAiTokenizer",
      "url": "https://user:url-canary@example.test/v1?token=query-canary",
      "api_key": "api-key-canary",
      "provider_params": {"access_key_id": "provider-secret-canary"},
      "max_prompt_tokens": 8192
    },
    {
      "id": -200,
      "display_name": "Seeded model",
      "name": "seeded-model",
      "provider": "CDCK",
      "url": "********",
      "api_key": "********",
      "seeded": true
    }
  ],
  "meta": {"ai_secrets": [{"name": "secret-canary"}]}
}"#;

#[test]
fn list_and_show_redact_server_secrets_and_accept_negative_seeded_ids() {
    let (url, requests) = start_mock(LLMS, "200 OK");
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["ai", "llm", "list", "alpha", "--format", "json"], &config);
    assert!(
        output.status.success(),
        "AI LLM list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rendered = String::from_utf8_lossy(&output.stdout);
    for secret in [
        "api-key-canary",
        "provider-secret-canary",
        "url-canary",
        "query-canary",
        "secret-canary",
    ] {
        assert!(
            !rendered.contains(secret),
            "secret leaked in list output: {secret}"
        );
    }
    let listed: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).expect("list JSON");
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().any(|llm| llm["id"] == -200));
    assert!(
        listed
            .iter()
            .any(|llm| llm["id"] == 9 && llm["api_key_configured"] == true)
    );

    let output = run_dsc(
        &["ai", "llm", "show", "alpha", "-200", "--format", "json"],
        &config,
    );
    assert!(
        output.status.success(),
        "AI LLM show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let shown = String::from_utf8_lossy(&output.stdout);
    assert!(shown.contains("<masked>"));
    assert!(!shown.contains("api-key-canary"));
    assert_eq!(requests.load(Ordering::Relaxed), 2);
}

#[test]
fn unavailable_ai_api_is_named_without_echoing_its_body() {
    let (url, _) = start_mock("{\"error\":\"response-secret-canary\"}", "404 Not Found");
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);

    let output = run_dsc(&["ai", "llm", "list", "alpha"], &config);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Discourse AI may be disabled"));
    assert!(!stderr.contains("response-secret-canary"));
}

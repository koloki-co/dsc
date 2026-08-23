// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Coverage for `dsc search all` - the merged fan-out search across every
//! configured forum, as distinct from the pre-existing single-forum
//! `dsc search <discourse>`.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use tempfile::TempDir;

/// A mock Discourse that answers every `/search.json` request with a fixed
/// topics payload (or, if `status` is not 200, an error status).
fn start_mock(status: u16, topics_json: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, status, &topics_json);
        }
    });
    format!("http://{addr}")
}

fn handle(mut stream: TcpStream, status: u16, topics_json: &str) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
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
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }

    let (status_line, body) = if status == 200 {
        (
            "HTTP/1.1 200 OK",
            format!(r#"{{"topics":[{topics_json}],"more_full_page_results":false}}"#),
        )
    } else {
        (
            "HTTP/1.1 503 Service Unavailable",
            r#"{"error":"unavailable"}"#.to_string(),
        )
    };
    let response = format!(
        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn hit(id: u64, title: &str) -> String {
    format!(r#"{{"id":{id},"title":"{title}","slug":"{title}-slug","posts_count":1}}"#)
}

#[test]
fn search_all_merges_and_tags_results_by_forum() {
    let alpha_url = start_mock(200, hit(1, "Alpha topic"));
    let beta_url = start_mock(200, hit(2, "Beta topic"));

    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n\n[[discourse]]\nname = \"beta\"\nbaseurl = \"{beta_url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );

    let output = run_dsc(&["search", "all", "topic", "--format", "json"], &config);
    assert!(
        output.status.success(),
        "search all failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("search all JSON");
    assert_eq!(rows.len(), 2, "expected one hit per forum: {rows:?}");
    assert_eq!(rows[0]["forum"], "alpha");
    assert_eq!(rows[0]["id"], 1);
    assert_eq!(rows[1]["forum"], "beta");
    assert_eq!(rows[1]["id"], 2);

    let text_output = run_dsc(&["search", "all", "topic"], &config);
    assert!(text_output.status.success());
    let text = String::from_utf8_lossy(&text_output.stdout);
    assert!(text.contains("alpha") && text.contains("Alpha topic"));
    assert!(text.contains("beta") && text.contains("Beta topic"));
}

#[test]
fn search_all_reports_per_forum_failures_without_losing_other_results() {
    let alpha_url = start_mock(200, hit(1, "Alpha topic"));
    let down_url = start_mock(503, String::new());

    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n\n[[discourse]]\nname = \"down\"\nbaseurl = \"{down_url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );

    let output = run_dsc(&["search", "all", "topic", "--format", "json"], &config);
    assert!(
        !output.status.success(),
        "search all should fail overall when one forum errors"
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("search all JSON");
    assert_eq!(rows.len(), 1, "the healthy forum's results should survive");
    assert_eq!(rows[0]["forum"], "alpha");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("down"),
        "stderr should name the failing forum: {stderr}"
    );
}

#[test]
fn search_single_forum_still_works() {
    let alpha_url = start_mock(200, hit(1, "Alpha topic"));
    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );

    let output = run_dsc(&["search", "alpha", "topic", "--format", "json"], &config);
    assert!(output.status.success());
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).expect("search JSON");
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].get("forum").is_none(),
        "single-forum search should not carry a forum tag"
    );
}

#[test]
fn search_all_respects_tags_filter() {
    let alpha_url = start_mock(200, hit(1, "Alpha topic"));
    let beta_url = start_mock(200, hit(2, "Beta topic"));

    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\ntags = [\"production\"]\n\n[[discourse]]\nname = \"beta\"\nbaseurl = \"{beta_url}\"\napikey = \"k\"\napi_username = \"tester\"\ntags = [\"staging\"]\n"
        ),
    );

    let output = run_dsc(
        &[
            "search",
            "all",
            "topic",
            "--tags",
            "production",
            "--format",
            "json",
        ],
        &config,
    );
    assert!(
        output.status.success(),
        "search all --tags failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("search all JSON");
    assert_eq!(rows.len(), 1, "only the production-tagged forum: {rows:?}");
    assert_eq!(rows[0]["forum"], "alpha");
}

#[test]
fn search_all_rejects_an_empty_tags_filter() {
    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"alpha\"\nbaseurl = \"https://alpha.example\"\napikey = \"k\"\napi_username = \"tester\"\n",
    );
    let output = run_dsc(&["search", "all", "topic", "--tags", ",;"], &config);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--tags must include at least one non-empty tag"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn search_single_forum_rejects_tags() {
    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"alpha\"\nbaseurl = \"https://alpha.example\"\napikey = \"k\"\napi_username = \"tester\"\n",
    );
    let output = run_dsc(
        &["search", "alpha", "topic", "--tags", "production"],
        &config,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("only usable together with `all`"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

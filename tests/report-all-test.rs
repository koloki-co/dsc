// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Coverage for `dsc report all <name>` - the merged fan-out report across
//! every configured forum, as distinct from the pre-existing single-forum
//! `dsc report <discourse> <name>`.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use tempfile::TempDir;

/// A mock Discourse that answers any `/admin/reports/*.json` request with a
/// fixed payload, or a 503 (simulating an unreachable forum).
fn start_mock(status: u16, report_json: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, status, &report_json);
        }
    });
    format!("http://{addr}")
}

fn handle(mut stream: TcpStream, status: u16, report_json: &str) {
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
        ("HTTP/1.1 200 OK", report_json.to_string())
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

fn signups_report(total_y: u64) -> String {
    format!(
        r#"{{"report":{{"type":"signups","data":[{{"x":"2026-09-01","y":{total_y}}}],"higher_is_better":true}}}}"#
    )
}

#[test]
fn report_all_merges_and_tags_results_by_forum() {
    let alpha_url = start_mock(200, signups_report(3));
    let beta_url = start_mock(200, signups_report(5));

    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n\n[[discourse]]\nname = \"beta\"\nbaseurl = \"{beta_url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );

    let output = run_dsc(&["report", "all", "signups", "--format", "json"], &config);
    assert!(
        output.status.success(),
        "report all failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("report all JSON");
    assert_eq!(rows.len(), 2, "expected one report per forum: {rows:?}");
    assert_eq!(rows[0]["forum"], "alpha");
    assert_eq!(rows[0]["total"], 3.0);
    assert_eq!(rows[1]["forum"], "beta");
    assert_eq!(rows[1]["total"], 5.0);

    let text_output = run_dsc(&["report", "all", "signups"], &config);
    assert!(text_output.status.success());
    let text = String::from_utf8_lossy(&text_output.stdout);
    assert!(text.contains("== alpha =="));
    assert!(text.contains("== beta =="));
}

#[test]
fn report_all_reports_per_forum_failures_without_losing_other_results() {
    let alpha_url = start_mock(200, signups_report(3));
    let down_url = start_mock(503, String::new());

    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n\n[[discourse]]\nname = \"down\"\nbaseurl = \"{down_url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );

    let output = run_dsc(&["report", "all", "signups", "--format", "json"], &config);
    assert!(
        !output.status.success(),
        "report all should fail overall when one forum errors"
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("report all JSON");
    assert_eq!(rows.len(), 1, "the healthy forum's report should survive");
    assert_eq!(rows[0]["forum"], "alpha");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("down"),
        "stderr should name the failing forum: {stderr}"
    );
}

#[test]
fn report_single_forum_still_works() {
    let alpha_url = start_mock(200, signups_report(3));
    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );

    let output = run_dsc(&["report", "alpha", "signups", "--format", "json"], &config);
    assert!(output.status.success());
    let view: serde_json::Value = serde_json::from_slice(&output.stdout).expect("report JSON");
    assert!(
        view.get("forum").is_none(),
        "single-forum report should not carry a forum tag"
    );
}

#[test]
fn report_all_respects_tags_filter() {
    let alpha_url = start_mock(200, signups_report(3));
    let beta_url = start_mock(200, signups_report(5));

    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\ntags = [\"production\"]\n\n[[discourse]]\nname = \"beta\"\nbaseurl = \"{beta_url}\"\napikey = \"k\"\napi_username = \"tester\"\ntags = [\"staging\"]\n"
        ),
    );

    let output = run_dsc(
        &[
            "report",
            "all",
            "signups",
            "--tags",
            "production",
            "--format",
            "json",
        ],
        &config,
    );
    assert!(
        output.status.success(),
        "report all --tags failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("report all JSON");
    assert_eq!(rows.len(), 1, "only the production-tagged forum: {rows:?}");
    assert_eq!(rows[0]["forum"], "alpha");
}

#[test]
fn report_all_rejects_an_empty_tags_filter() {
    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"alpha\"\nbaseurl = \"https://alpha.example\"\napikey = \"k\"\napi_username = \"tester\"\n",
    );
    let output = run_dsc(&["report", "all", "signups", "--tags", ",;"], &config);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--tags must include at least one non-empty tag"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn report_single_forum_rejects_tags() {
    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"alpha\"\nbaseurl = \"https://alpha.example\"\napikey = \"k\"\napi_username = \"tester\"\n",
    );
    let output = run_dsc(
        &["report", "alpha", "signups", "--tags", "production"],
        &config,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("only usable together with `all`"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

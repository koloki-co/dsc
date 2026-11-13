// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! R85: reporting-only reconciliation must stay on the Discourse API path.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn start_mock(requests: Arc<Mutex<Vec<String>>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &requests);
        }
    });
    format!("http://{addr}")
}

fn handle(mut stream: TcpStream, requests: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("header line");
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().expect("content length");
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).expect("request body");
    requests
        .lock()
        .expect("requests lock")
        .push(request_line.trim().to_string());

    let response_body = if request_line.starts_with("GET /t/42.json ") {
        r#"{"id":42,"title":"Changelog","closed":false,"archived":false,"post_stream":{"posts":[],"stream":[]}}"#
    } else if request_line.starts_with("POST /posts.json ") {
        r#"{"id":99,"topic_id":42}"#
    } else {
        r#"{"errors":["unexpected request"]}"#
    };
    let status = if response_body.contains("unexpected request") {
        404
    } else {
        200
    };
    write!(
        stream,
        "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
        response_body.len()
    )
    .expect("write response");
}

#[test]
fn report_reconciles_only_a_pending_failure_without_ssh() {
    let dir = TempDir::new().expect("tempdir");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let baseurl = start_mock(Arc::clone(&requests));
    let config = dir.path().join("dsc.toml");
    std::fs::write(
        &config,
        format!("[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"key\"\napi_username = \"system\"\nchangelog_topic_id = 42\n"),
    )
    .expect("config");
    let log = dir.path().join("update.log");
    std::fs::write(
        &log,
        "2026-11-13T00:00:00Z\tmock\treporting-failed\tbefore\tafter\tHTTP 422\n",
    )
    .expect("seed log");
    let payload = dir.path().join("report.md");
    std::fs::write(&payload, "- [x] Reconciled\n").expect("payload");

    let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .args([
            "update",
            "report",
            "mock",
            payload.to_str().expect("payload path"),
            "--yes",
        ])
        .env("DSC_CONFIG", &config)
        .env("DSC_UPDATE_LOG", &log)
        .output()
        .expect("run dsc");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        *requests.lock().expect("requests lock"),
        vec!["GET /t/42.json HTTP/1.1", "POST /posts.json HTTP/1.1"]
    );
    assert!(
        std::fs::read_to_string(log)
            .expect("log")
            .contains("\treported\t")
    );
}

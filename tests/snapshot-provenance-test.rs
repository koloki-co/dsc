// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Functional coverage for the R71 snapshot provenance header on
//! `tag pull`, `category def pull` and `theme palette pull`, against a mocked Discourse. See
//! `spec/commands/snapshot-provenance.md` for the contract: pulls write a
//! credential-free `provenance` object, and push treats it as informational
//! only, so a snapshot copied from another site still applies.

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// A read-only mock Discourse serving one tag (tag groups refused, as for a
/// non-admin key) and one category. Every request line is recorded so tests
/// can prove push dry-runs stayed read-only.
fn start_mock() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_requests = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &thread_requests);
        }
    });
    (format!("http://{addr}"), requests)
}

fn handle(mut stream: TcpStream, requests: &Mutex<Vec<String>>) {
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
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    requests.lock().unwrap().push(format!("{method} {path}"));

    let category = r##"{"id":4,"name":"General","slug":"general","color":"0088CC","text_color":"FFFFFF","position":1,"custom_fields":{}}"##;
    let (status_line, body) = if method != "GET" {
        ("HTTP/1.1 500 Internal Server Error", "{}".to_string())
    } else if path.starts_with("/tags.json") {
        (
            "HTTP/1.1 200 OK",
            r#"{"tags":[{"id":1,"text":"alpha","count":0,"pm_count":0,"description":"Alpha tag"}]}"#
                .to_string(),
        )
    } else if path.starts_with("/tag_groups.json") {
        ("HTTP/1.1 403 Forbidden", "{}".to_string())
    } else if path.starts_with("/categories.json") {
        (
            "HTTP/1.1 200 OK",
            format!(r#"{{"category_list":{{"categories":[{category}]}}}}"#),
        )
    } else if path.starts_with("/c/4/show.json") {
        ("HTTP/1.1 200 OK", format!(r#"{{"category":{category}}}"#))
    } else if path.starts_with("/admin/color_schemes.json") {
        (
            "HTTP/1.1 200 OK",
            r#"[{"id":19,"name":"Brand","colors":[{"name":"primary","hex":"222222"}],"base_scheme_id":null}]"#
                .to_string(),
        )
    } else {
        ("HTTP/1.1 404 Not Found", "{}".to_string())
    };
    let response = format!(
        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Configure the mock with credentials embedded in the base URL, so the
/// tests also prove `provenance.source` never carries them.
fn config_for(url: &str, dir: &TempDir) -> std::path::PathBuf {
    let with_userinfo = url.replacen("http://", "http://admin:secret@", 1);
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{with_userinfo}/\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    )
}

fn assert_provenance(snapshot: &serde_yaml::Value, url: &str, content: &str) {
    assert_eq!(snapshot["provenance"]["source"], url);
    assert_eq!(
        snapshot["provenance"]["exporter_version"],
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        !content.contains("secret") && !content.contains("admin:"),
        "snapshot leaked URL credentials: {content}"
    );
}

/// Point a pulled snapshot's provenance at another site, as when a reviewed
/// staging snapshot is copied to production.
fn retarget_provenance(path: &std::path::Path) {
    let mut snapshot: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    snapshot["provenance"]["source"] = "https://staging.example.com".into();
    std::fs::write(path, serde_yaml::to_string(&snapshot).unwrap()).unwrap();
}

fn assert_only_reads(requests: &Mutex<Vec<String>>) {
    let requests = requests.lock().unwrap();
    assert!(
        requests.iter().all(|r| r.starts_with("GET ")),
        "dry-run sent a non-GET request: {requests:?}"
    );
}

#[test]
fn tag_pull_writes_provenance_and_push_ignores_it() {
    let (url, requests) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let path = dir.path().join("tags.yaml");

    let output = run_dsc(&["tag", "pull", "alpha", path.to_str().unwrap()], &config);
    assert!(
        output.status.success(),
        "tag pull failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let content = std::fs::read_to_string(&path).unwrap();
    let snapshot: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    assert_eq!(snapshot["version"], 1);
    assert_eq!(snapshot["complete"], false);
    assert_eq!(snapshot["tags"][0]["name"], "alpha");
    assert_provenance(&snapshot, &url, &content);

    retarget_provenance(&path);
    let output = run_dsc(
        &["--dry-run", "tag", "push", "alpha", path.to_str().unwrap()],
        &config,
    );
    assert!(
        output.status.success(),
        "tag push --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains('~') && !stdout.contains('+') && !stdout.contains("- delete"),
        "provenance must not plan a change, got: {stdout}"
    );
    assert_only_reads(&requests);
}

#[test]
fn category_def_pull_writes_provenance_and_push_ignores_it() {
    let (url, requests) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let path = dir.path().join("categories.yaml");

    let output = run_dsc(
        &["category", "def", "pull", "alpha", path.to_str().unwrap()],
        &config,
    );
    assert!(
        output.status.success(),
        "category def pull failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let content = std::fs::read_to_string(&path).unwrap();
    let snapshot: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    assert_eq!(snapshot["version"], 2);
    assert_eq!(snapshot["categories"][0]["name"], "General");
    assert_provenance(&snapshot, &url, &content);

    retarget_provenance(&path);
    let output = run_dsc(
        &[
            "--dry-run",
            "category",
            "def",
            "push",
            "alpha",
            path.to_str().unwrap(),
        ],
        &config,
    );
    assert!(
        output.status.success(),
        "category def push --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains('~') && !stdout.contains("+ create"),
        "provenance must not plan a change, got: {stdout}"
    );
    assert_only_reads(&requests);
}

#[test]
fn palette_pull_writes_provenance_and_push_ignores_it() {
    let (url, requests) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let config = config_for(&url, &dir);
    let path = dir.path().join("palette.yaml");

    let output = run_dsc(
        &[
            "theme",
            "palette",
            "pull",
            "alpha",
            "19",
            path.to_str().unwrap(),
        ],
        &config,
    );
    assert!(
        output.status.success(),
        "theme palette pull failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let content = std::fs::read_to_string(&path).unwrap();
    let snapshot: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    assert_eq!(snapshot["id"], 19);
    assert_eq!(snapshot["colors"]["primary"], "222222");
    assert_provenance(&snapshot, &url, &content);

    retarget_provenance(&path);
    let output = run_dsc(
        &[
            "--dry-run",
            "theme",
            "palette",
            "push",
            "alpha",
            path.to_str().unwrap(),
        ],
        &config,
    );
    assert!(
        output.status.success(),
        "theme palette push --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("= unchanged: palette 19") && !stdout.contains("PUT "),
        "provenance must not plan a change, got: {stdout}"
    );
    assert_only_reads(&requests);
}

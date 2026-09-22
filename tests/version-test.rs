// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use tempfile::TempDir;

fn start_version_mock(version: &str, commit: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let version = version.to_string();
    let commit = commit.to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle_version_request(stream, &version, &commit);
        }
    });
    format!("http://{addr}")
}

fn handle_version_request(mut stream: TcpStream, version: &str, commit: &str) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line == "\n" {
            break;
        }
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let (content_type, body) = if path.starts_with("/about.json") {
        (
            "application/json",
            format!(r#"{{"about":{{"version":"{version}"}}}}"#),
        )
    } else {
        (
            "text/html",
            format!(
                r#"<html><head><meta name="generator" content="Discourse {version} - https://github.com/discourse/discourse version {commit}"></head></html>"#
            ),
        )
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("write response");
}

#[test]
fn bare_dsc_prints_help_and_exits_successfully_without_config() {
    let dir = TempDir::new().expect("tempdir");
    let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .current_dir(dir.path())
        .env("DSC_CONFIG", dir.path().join("missing.toml"))
        .output()
        .expect("run dsc");

    assert!(output.status.success(), "bare dsc should exit successfully");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"), "missing usage summary: {stdout}");
    assert!(
        stdout.contains("Commands:"),
        "missing command list: {stdout}"
    );
}

#[test]
fn own_version_does_not_resolve_config() {
    let dir = TempDir::new().expect("tempdir");
    let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .args([
            "--config",
            dir.path().join("missing.toml").to_str().unwrap(),
            "version",
            "--format",
            "json",
        ])
        .output()
        .expect("run dsc version");

    assert!(
        output.status.success(),
        "dsc version should not load config: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse version JSON");
    assert_eq!(value["name"], "dsc");
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn version_all_requires_config() {
    let dir = TempDir::new().expect("tempdir");
    let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .args([
            "--config",
            dir.path().join("missing.toml").to_str().unwrap(),
            "version",
            "--all",
        ])
        .output()
        .expect("run dsc version --all");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("config file not found"),
        "fleet version should resolve config: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn version_all_reports_every_forum_in_config_order() {
    let alpha_url = start_version_mock("2026.9.0-latest", "aaaaaaaaaa");
    let beta_url = start_version_mock("2026.8.1", "bbbbbbbbbb");
    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"beta\"\nbaseurl = \"{beta_url}\"\napikey = \"k\"\napi_username = \"tester\"\ntags = [\"staging\"]\n\n[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\ntags = [\"production\"]\n"
        ),
    );

    let output = run_dsc(&["version", "--all", "--format", "json"], &config);
    assert!(
        output.status.success(),
        "version --all failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("version --all JSON");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["discourse"], "beta");
    assert_eq!(rows[0]["version"], "2026.8.1");
    assert_eq!(rows[0]["commit"], "bbbbbbbbbb");
    assert_eq!(rows[1]["discourse"], "alpha");
    assert_eq!(rows[1]["commit"], "aaaaaaaaaa");

    let tagged = run_dsc(
        &["version", "--tags", "PRODUCTION", "--format", "json"],
        &config,
    );
    assert!(tagged.status.success());
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&tagged.stdout).expect("tagged version JSON");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["discourse"], "alpha");
}

#[test]
fn version_all_keeps_error_rows_and_fails_after_rendering() {
    let alpha_url = start_version_mock("2026.9.0-latest", "aaaaaaaaaa");
    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n\n[[discourse]]\nname = \"missing-credentials\"\nbaseurl = \"https://example.invalid\"\n"
        ),
    );

    let output = run_dsc(&["version", "--all", "--format", "json"], &config);
    assert!(!output.status.success());
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("partial version JSON");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["discourse"], "alpha");
    assert_eq!(rows[1]["discourse"], "missing-credentials");
    assert!(
        rows[1]["error"]
            .as_str()
            .unwrap()
            .contains("missing apikey")
    );
    assert!(rows[1].get("version").is_none());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("missing-credentials"), "{stderr}");
    assert!(stderr.contains("failed on 1 of 2 forum(s)"), "{stderr}");
}

#[test]
fn version_selectors_are_mutually_exclusive() {
    let dir = TempDir::new().expect("tempdir");
    let config = write_temp_config(&dir, "");
    for args in [
        vec!["version", "alpha", "--all"],
        vec!["version", "alpha", "--tags", "production"],
        vec!["version", "--all", "--tags", "production"],
    ] {
        let output = run_dsc(&args, &config);
        assert!(
            !output.status.success(),
            "selectors should conflict: {args:?}"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("cannot be used with"),
            "unexpected clap error for {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn version_forum_reports_discourse_version_and_commit() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_version_forum: dsc version <forum> reads /about.json");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["version", &test.name], &config_path);
    assert!(
        output.status.success(),
        "version <forum> failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&test.name) && stdout.contains("Discourse"),
        "expected '<forum>: Discourse <version> (<commit>)', got: {stdout}"
    );
}

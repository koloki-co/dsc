// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! R73: check a dry-run plan against what a real run actually does.
//!
//! `dry-run-mutation-test.rs` already proves `--dry-run` issues zero
//! mutating requests. It does not prove the plan `--dry-run` *prints* is an
//! accurate description of what a real run would do - those are two
//! independent claims. A command could, for instance, print "would change
//! `x`" while the real apply path reads a different field, skips `x`, or
//! sends a different value, and the existing zero-mutation test would not
//! catch it.
//!
//! This file runs the same command twice - once under `--dry-run`, once for
//! real - against the same frozen mock state (the GET response never
//! changes, regardless of how many PUTs land), then compares the dry-run
//! plan's stated targets and values against the PUT requests the real run
//! actually made. Starting with `dsc setting push`, per the roadmap's R73
//! phasing; broaden to other declarative-push families once this harness
//! shape is proven.

mod common;
use common::*;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// Frozen server-side settings, as `GET /admin/site_settings.json` always
/// returns them - a real run's PUTs must not be able to change what the next
/// request sees, or "same frozen state for both phases" would not hold.
const SERVER_SETTINGS_JSON: &str = r#"{"site_settings":[
    {"setting":"setting_a","value":"old_a","default":"old_a","type":"string"},
    {"setting":"setting_b","value":"same_b","default":"same_b","type":"string"},
    {"setting":"setting_c","value":"old_c","default":"old_c","type":"string"}
]}"#;

/// The local file pushed in both phases. `setting_a`/`setting_c` differ from
/// the server (planned changes), `setting_b` matches it (planned no-op),
/// `setting_d` does not exist on the server (planned skip).
const SETTINGS_FILE: &str = r#"version: 1
complete: false
settings:
  - name: setting_a
    value: new_a
  - name: setting_b
    value: same_b
  - name: setting_c
    value: new_c
  - name: setting_d
    value: whatever
"#;

/// Frozen server-side theme, as `GET /admin/themes/42.json` always returns it.
/// `header_links` holds spaced JSON so the test also exercises the
/// list-setting normalisation (file side is compact JSON, so it must plan as
/// unchanged, not as a spurious change).
const SERVER_THEME_JSON: &str = r#"{"theme":{"id":42,"name":"mock theme","settings":[
    {"setting":"setting_a","value":"old_a","default":"old_a","type":"string"},
    {"setting":"setting_b","value":"same_b","default":"same_b","type":"string"},
    {"setting":"setting_c","value":"old_c","default":"old_c","type":"string"},
    {"setting":"header_links","value":"[{\"id\": 1}]","default":"[]","type":"string"}
]}}"#;

/// Theme settings file: `setting_a`/`setting_c` change, `setting_b` and
/// `header_links` match the server, `setting_d` is unknown (skipped).
const THEME_SETTINGS_FILE: &str = r#"version: 1
theme_id: 42
settings:
  - setting: setting_a
    value: new_a
  - setting: setting_b
    value: same_b
  - setting: setting_c
    value: new_c
  - setting: header_links
    value:
      - id: 1
  - setting: setting_d
    value: whatever
"#;

#[derive(Default)]
struct Recorder {
    /// (setting name, value sent) for every PUT received, in arrival order.
    puts: Vec<(String, String)>,
}

fn start_mock(recorder: Arc<Mutex<Recorder>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &recorder);
        }
    });
    format!("http://{addr}")
}

fn handle(mut stream: TcpStream, recorder: &Mutex<Recorder>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
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
    let body = String::from_utf8_lossy(&body).to_string();

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    let (status, out) = if method == "GET" && path == "/admin/site_settings.json" {
        ("HTTP/1.1 200 OK", SERVER_SETTINGS_JSON.to_string())
    } else if method == "GET" && path == "/admin/themes/42.json" {
        ("HTTP/1.1 200 OK", SERVER_THEME_JSON.to_string())
    } else if method == "PUT" && path == "/admin/themes/42/setting.json" {
        // Form-encoded `name=<setting>&value=<value>`.
        let field = |key: &str| {
            body.split('&')
                .filter_map(|kv| kv.split_once('='))
                .find(|(k, _)| *k == key)
                .map(|(_, v)| urlencoding_decode(v))
                .unwrap_or_default()
        };
        recorder
            .lock()
            .expect("recorder lock")
            .puts
            .push((field("name"), field("value")));
        ("HTTP/1.1 200 OK", r#"{"success":"OK"}"#.to_string())
    } else if method == "PUT" {
        if let Some(name) = path
            .strip_prefix("/admin/site_settings/")
            .and_then(|p| p.strip_suffix(".json"))
        {
            // Form-encoded `name=value`, matching `site_setting_form`.
            let value = body
                .split_once('=')
                .map(|(_, v)| urlencoding_decode(v))
                .unwrap_or_default();
            recorder
                .lock()
                .expect("recorder lock")
                .puts
                .push((name.to_string(), value));
        }
        ("HTTP/1.1 200 OK", r#"{"success":"OK"}"#.to_string())
    } else {
        ("HTTP/1.1 404 Not Found", "{}".to_string())
    };
    let response = format!(
        "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
        out.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

/// Minimal `application/x-www-form-urlencoded` value decoder - just enough
/// for the plain alphanumeric values this test sends (`new_a`, `new_c`).
fn urlencoding_decode(s: &str) -> String {
    s.replace('+', " ")
}

fn mock_config(dir: &TempDir, url: &str) -> PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    )
}

fn write_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).expect("write temp file");
    path
}

/// Run a declarative-push command twice against the same frozen mock state -
/// once under `--dry-run`, once for real - and assert the dry-run plan's
/// parsed `(name, value)` pairs exactly match the real run's PUT requests.
/// This is R73's core check, factored out once a second declarative-push
/// family (`theme setting push`, alongside `setting push`) needed the same
/// shape; each family keeps its own frozen fixtures and plan-line parser,
/// since those differ, but the two-phase run/compare logic does not.
fn assert_dry_run_plan_matches_real_run(
    config_path: &Path,
    recorder: &Mutex<Recorder>,
    dry_run_args: &[&str],
    real_args: &[&str],
    parse_plan: impl Fn(&str) -> BTreeMap<String, String>,
    expected_plan: &BTreeMap<String, String>,
) {
    // Phase 1: dry-run. Capture the plan; the mock's GET response is frozen
    // (constant regardless of PUT traffic), so phase 2 sees identical state.
    let dry_run_output = run_dsc(dry_run_args, config_path);
    assert!(
        dry_run_output.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run_output.stderr)
    );
    assert!(
        recorder.lock().expect("recorder lock").puts.is_empty(),
        "dry-run must issue zero PUT requests"
    );
    let planned = parse_plan(&String::from_utf8_lossy(&dry_run_output.stdout));
    assert_eq!(&planned, expected_plan, "unexpected dry-run plan");

    // Phase 2: the real run, against the identical frozen GET response.
    let real_output = run_dsc(real_args, config_path);
    assert!(
        real_output.status.success(),
        "{}",
        String::from_utf8_lossy(&real_output.stderr)
    );
    let actual: BTreeMap<String, String> = recorder
        .lock()
        .expect("recorder lock")
        .puts
        .iter()
        .cloned()
        .collect();

    // The core R73 assertion: the plan's stated targets and values are
    // exactly the requests the real run made against the same state - not a
    // subset, not a superset, and not merely "the same names with some other
    // value".
    assert_eq!(
        planned, actual,
        "the dry-run plan's stated changes must exactly match the real run's \
         PUT requests against the same frozen server state"
    );
}

/// Parse `dsc setting push`'s plan output for its `~ name: "from" → "to"`
/// change lines, returning `name -> to` for each. This is the "stated
/// targets and values" half of R73's comparison - deliberately reading the
/// same text a human reviewing `--dry-run` output would read, not an
/// internal data structure, so the test fails if the printed plan and the
/// wire requests ever disagree for any reason, including a future change
/// that splits their current shared code path.
fn parse_planned_changes(stdout: &str) -> BTreeMap<String, String> {
    let mut changes = BTreeMap::new();
    for line in stdout.lines() {
        let Some(rest) = line.trim_start().strip_prefix("~ ") else {
            continue;
        };
        let Some((name, arrow_part)) = rest.split_once(": ") else {
            continue;
        };
        let Some((_from, to)) = arrow_part.split_once(" \u{2192} ") else {
            continue;
        };
        let to = to.trim_matches('"').to_string();
        changes.insert(name.to_string(), to);
    }
    changes
}

#[test]
fn setting_push_dry_run_plan_matches_the_real_run_against_the_same_frozen_state() {
    let recorder = Arc::new(Mutex::new(Recorder::default()));
    let url = start_mock(Arc::clone(&recorder));
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &url);
    let settings_path = write_file(&dir, "settings.yaml", SETTINGS_FILE);
    let settings_path = settings_path.to_str().expect("utf-8 temp path");

    assert_dry_run_plan_matches_real_run(
        &config_path,
        &recorder,
        &["--dry-run", "setting", "push", "mock", settings_path],
        &["setting", "push", "mock", settings_path],
        parse_planned_changes,
        // setting_b is unchanged and setting_d is unknown on the server, so
        // only setting_a/setting_c should be planned changes.
        &BTreeMap::from([
            ("setting_a".to_string(), "new_a".to_string()),
            ("setting_c".to_string(), "new_c".to_string()),
        ]),
    );
}

/// Parse `dsc theme setting push`'s `  name: from -> to` plan lines into
/// `name -> to`. Same principle as `parse_planned_changes`: read the printed
/// text a reviewer would read, not an internal structure.
fn parse_theme_planned_changes(stdout: &str) -> BTreeMap<String, String> {
    let mut changes = BTreeMap::new();
    for line in stdout.lines() {
        if !line.starts_with("  ") {
            continue;
        }
        let Some((name, change)) = line.trim().split_once(": ") else {
            continue;
        };
        let Some((_from, to)) = change.split_once(" -> ") else {
            continue;
        };
        changes.insert(name.to_string(), to.to_string());
    }
    changes
}

#[test]
fn theme_setting_push_dry_run_plan_matches_the_real_run_against_the_same_frozen_state() {
    let recorder = Arc::new(Mutex::new(Recorder::default()));
    let url = start_mock(Arc::clone(&recorder));
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &url);
    let file = write_file(&dir, "theme-settings.yaml", THEME_SETTINGS_FILE);
    let file = file.to_str().expect("utf-8 temp path");

    assert_dry_run_plan_matches_real_run(
        &config_path,
        &recorder,
        &["--dry-run", "theme", "setting", "push", "mock", "42", file],
        &["theme", "setting", "push", "mock", "42", file],
        parse_theme_planned_changes,
        // setting_b and header_links match the server (one via a plain
        // value, the other via JSON-list normalisation of spaced vs
        // compact JSON) and setting_d is unknown, so only setting_a/
        // setting_c should be planned changes.
        &BTreeMap::from([
            ("setting_a".to_string(), "new_a".to_string()),
            ("setting_c".to_string(), "new_c".to_string()),
        ]),
    );
}

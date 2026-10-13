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

/// Frozen server-side palette list, as `GET /admin/color_schemes.json` always
/// returns it. Palette 19 is what the pushed file was pulled from.
const SERVER_PALETTES_JSON: &str = r#"[{"id":19,"name":"Palette","colors":[
    {"name":"primary","hex":"000000"},
    {"name":"secondary","hex":"ffffff"},
    {"name":"tertiary","hex":"0088cc"}
]}]"#;

/// Palette file: renamed, `primary`/`tertiary` edited against the pulled
/// baseline, `secondary` untouched (planned no-op).
const PALETTE_FILE: &str = r#"{"id":19,"name":"Renamed palette",
"colors":{"primary":"111111","secondary":"ffffff","tertiary":"abcdef"},
"pulled":{"name":"Palette","colors":{"primary":"000000","secondary":"ffffff","tertiary":"0088cc"}}}
"#;

/// Frozen server-side tags, as `GET /tags.json` always returns them. Both
/// file tags already exist on the server, so this fixture stays clear of
/// "groupless_missing" tags entirely: `tag_push`'s real-run apply path
/// `bail!`s at the end if any file tag is in no group and absent from the
/// server, even after successfully applying every achievable change, which
/// would make the real run's exit status fail for reasons unrelated to
/// what this test is checking.
const SERVER_TAGS_JSON: &str = r#"{"tags":[
    {"id":1,"text":"tag_a","description":"old description"},
    {"id":2,"text":"tag_b","description":"same description"}
]}"#;

/// No tag groups on the server; the file declares none either, so the
/// group-reconciliation half of `tag push` plans no changes and this test
/// covers only the description-update path.
const SERVER_TAG_GROUPS_JSON: &str = r#"{"tag_groups":[]}"#;

/// Taxonomy file: `tag_a`'s description differs from the server (planned
/// change), `tag_b`'s matches (planned no-op).
const TAG_FILE: &str = r#"version: 1
complete: true
tags:
  - name: tag_a
    description: new description
  - name: tag_b
    description: same description
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
    } else if method == "GET" && path == "/admin/color_schemes.json" {
        ("HTTP/1.1 200 OK", SERVER_PALETTES_JSON.to_string())
    } else if method == "GET" && path == "/tags.json" {
        ("HTTP/1.1 200 OK", SERVER_TAGS_JSON.to_string())
    } else if method == "GET" && path == "/tag_groups.json" {
        ("HTTP/1.1 200 OK", SERVER_TAG_GROUPS_JSON.to_string())
    } else if method == "PUT" && path.starts_with("/tag/") && path.ends_with(".json") {
        // JSON `{"tag":{"description":"..."}}`; recorded as `name -> desc`.
        let name = path
            .strip_prefix("/tag/")
            .and_then(|p| p.strip_suffix(".json"))
            .unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        if let Some(desc) = payload["tag"]["description"].as_str() {
            recorder
                .lock()
                .expect("recorder lock")
                .puts
                .push((name.to_string(), desc.to_string()));
        }
        ("HTTP/1.1 200 OK", r#"{"success":"OK"}"#.to_string())
    } else if method == "PUT" && path == "/admin/color_schemes/19.json" {
        // JSON `{"color_scheme":{"name":?,"colors":[{"name","hex"}]}}`;
        // recorded as `name -> new name` plus `color -> hex`.
        let payload: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let scheme = &payload["color_scheme"];
        let mut recorder = recorder.lock().expect("recorder lock");
        if let Some(name) = scheme["name"].as_str() {
            recorder.puts.push(("name".to_string(), name.to_string()));
        }
        for color in scheme["colors"].as_array().into_iter().flatten() {
            recorder.puts.push((
                color["name"].as_str().unwrap_or_default().to_string(),
                color["hex"].as_str().unwrap_or_default().to_string(),
            ));
        }
        ("HTTP/1.1 200 OK", r#"{"success":"OK"}"#.to_string())
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

/// Parse `dsc palette push`'s `  ~ name: "from" -> "to"` plan lines (palette
/// rename and colour edits alike) into `name -> to`.
fn parse_palette_planned_changes(stdout: &str) -> BTreeMap<String, String> {
    let mut changes = BTreeMap::new();
    for line in stdout.lines() {
        let Some(rest) = line.trim_start().strip_prefix("~ ") else {
            continue;
        };
        let Some((name, change)) = rest.split_once(": ") else {
            continue;
        };
        let Some((_from, to)) = change.split_once(" -> ") else {
            continue;
        };
        changes.insert(name.to_string(), to.trim_matches('"').to_string());
    }
    changes
}

#[test]
fn palette_push_dry_run_plan_matches_the_real_run_against_the_same_frozen_state() {
    let recorder = Arc::new(Mutex::new(Recorder::default()));
    let url = start_mock(Arc::clone(&recorder));
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &url);
    // A real run rewrites the file with a refreshed snapshot, so each phase
    // gets its own identical copy.
    let dry_file = write_file(&dir, "palette-dry.json", PALETTE_FILE);
    let real_file = write_file(&dir, "palette-real.json", PALETTE_FILE);
    let dry_file = dry_file.to_str().expect("utf-8 temp path");
    let real_file = real_file.to_str().expect("utf-8 temp path");

    assert_dry_run_plan_matches_real_run(
        &config_path,
        &recorder,
        &["--dry-run", "palette", "push", "mock", dry_file],
        &["palette", "push", "mock", real_file],
        parse_palette_planned_changes,
        // secondary is unchanged, so only the rename and the two edited
        // colours should be planned.
        &BTreeMap::from([
            ("name".to_string(), "Renamed palette".to_string()),
            ("primary".to_string(), "111111".to_string()),
            ("tertiary".to_string(), "abcdef".to_string()),
        ]),
    );
}

/// Parse `dsc tag push`'s `  ~ set description: name ("desc")` plan lines
/// into `name -> desc`. Scoped to the description-update path only - tag
/// group create/update/delete and tag creation-via-group have their own
/// differently-shaped plan lines, deliberately not covered here (see the
/// roadmap's R73 entry for what's left).
fn parse_tag_planned_changes(stdout: &str) -> BTreeMap<String, String> {
    let mut changes = BTreeMap::new();
    for line in stdout.lines() {
        let Some(rest) = line.trim_start().strip_prefix("~ set description: ") else {
            continue;
        };
        let Some((name, desc_part)) = rest.split_once(" (") else {
            continue;
        };
        let desc = desc_part
            .trim_end_matches(')')
            .trim_matches('"')
            .to_string();
        changes.insert(name.to_string(), desc);
    }
    changes
}

#[test]
fn tag_push_dry_run_plan_matches_the_real_run_against_the_same_frozen_state() {
    let recorder = Arc::new(Mutex::new(Recorder::default()));
    let url = start_mock(Arc::clone(&recorder));
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &url);
    let file = write_file(&dir, "tags.yaml", TAG_FILE);
    let file = file.to_str().expect("utf-8 temp path");

    assert_dry_run_plan_matches_real_run(
        &config_path,
        &recorder,
        &["--dry-run", "tag", "push", "mock", file],
        &["tag", "push", "mock", file],
        parse_tag_planned_changes,
        // tag_b already matches the server, so only tag_a's description
        // should be planned.
        &BTreeMap::from([("tag_a".to_string(), "new description".to_string())]),
    );
}

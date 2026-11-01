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
use std::collections::{BTreeMap, BTreeSet};
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

/// Frozen category 7: two existing topics. `alpha.md` routes to 101 by
/// filename/slug, `beta.md` to 102; `gamma.md` matches neither (a create).
const SERVER_CATEGORY_JSON: &str = r#"{"category":{"name":"Docs","slug":"docs"},
"topic_list":{"topics":[
    {"id":101,"title":"Alpha","slug":"alpha"},
    {"id":102,"title":"Beta","slug":"beta"}
]}}"#;

const SERVER_TOPIC_101_JSON: &str = r#"{"id":101,"category_id":7,
"post_stream":{"posts":[{"id":1001,"raw":"old alpha body"}]}}"#;

const SERVER_TOPIC_102_JSON: &str = r#"{"id":102,"category_id":7,
"post_stream":{"posts":[{"id":1002,"raw":"same beta body"}]}}"#;

/// Frozen post 3003, as `GET /posts/3003.json?include_raw=1` always returns
/// it: currently in topic 101, so `post move` must plan and send the move
/// from that topic.
const SERVER_POST_3003_JSON: &str = r#"{"id":3003,"topic_id":101,"post_number":4,
"raw":"a reply"}"#;

#[derive(Default)]
struct Recorder {
    /// Overrides the `GET /tag_groups.json` body when set; the default is
    /// `SERVER_TAG_GROUPS_JSON` (no groups). Frozen for the whole test.
    tag_groups_json: Option<String>,
    /// Overrides the `GET /tags.json` body when set; the default is
    /// `SERVER_TAGS_JSON`. Frozen for the whole test.
    tags_json: Option<String>,
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
        let body = recorder
            .lock()
            .expect("recorder lock")
            .tags_json
            .clone()
            .unwrap_or_else(|| SERVER_TAGS_JSON.to_string());
        ("HTTP/1.1 200 OK", body)
    } else if method == "GET" && path == "/tag_groups.json" {
        let body = recorder
            .lock()
            .expect("recorder lock")
            .tag_groups_json
            .clone()
            .unwrap_or_else(|| SERVER_TAG_GROUPS_JSON.to_string());
        ("HTTP/1.1 200 OK", body)
    } else if (method == "POST" && path == "/tag_groups.json")
        || (method == "PUT" && path.starts_with("/tag_groups/") && path.ends_with(".json"))
    {
        // JSON `{"tag_group":{"name":..,"tag_names":[..]}}`; recorded as
        // `group name -> tag names joined with ", "`.
        let payload: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let group = &payload["tag_group"];
        let tags: Vec<&str> = group["tag_names"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str())
            .collect();
        recorder.lock().expect("recorder lock").puts.push((
            group["name"].as_str().unwrap_or_default().to_string(),
            tags.join(", "),
        ));
        (
            "HTTP/1.1 200 OK",
            r#"{"success":"OK","tag_group":{"id":77}}"#.to_string(),
        )
    } else if method == "DELETE" && path.starts_with("/tag/") && path.ends_with(".json") {
        // Recorded as `delete tag <name> -> ""`, matching the plan parser.
        let name = path
            .strip_prefix("/tag/")
            .and_then(|p| p.strip_suffix(".json"))
            .unwrap_or_default();
        recorder
            .lock()
            .expect("recorder lock")
            .puts
            .push((format!("delete tag {name}"), String::new()));
        ("HTTP/1.1 200 OK", r#"{"success":"OK"}"#.to_string())
    } else if method == "DELETE" && path.starts_with("/tag_groups/") && path.ends_with(".json") {
        // The real run deletes by ID; map it back to the frozen fixture's
        // group name so it is comparable with the plan's name-based line.
        let id = path
            .strip_prefix("/tag_groups/")
            .and_then(|p| p.strip_suffix(".json"))
            .unwrap_or_default();
        let name = match id {
            "6" => "stale",
            other => other,
        };
        recorder
            .lock()
            .expect("recorder lock")
            .puts
            .push((format!("delete group {name}"), String::new()));
        ("HTTP/1.1 200 OK", r#"{"success":"OK"}"#.to_string())
    } else if method == "GET" && path == "/c/7.json" {
        ("HTTP/1.1 200 OK", SERVER_CATEGORY_JSON.to_string())
    } else if method == "GET" && path.starts_with("/t/101.json") {
        ("HTTP/1.1 200 OK", SERVER_TOPIC_101_JSON.to_string())
    } else if method == "GET" && path.starts_with("/t/102.json") {
        ("HTTP/1.1 200 OK", SERVER_TOPIC_102_JSON.to_string())
    } else if method == "PUT" && path == "/posts/1001.json" {
        // Form `post[raw]=...`; recorded as `topic 101 -> raw byte length`
        // (post 1001 is the OP of topic 101 in the frozen fixture).
        let raw = form_field(&body, "post[raw]");
        recorder
            .lock()
            .expect("recorder lock")
            .puts
            .push(("topic 101".to_string(), raw.len().to_string()));
        ("HTTP/1.1 200 OK", r#"{"success":"OK"}"#.to_string())
    } else if method == "PUT" && path == "/posts/2002.json" {
        // `dsc post push`: form `post[raw]=...`; recorded as
        // `post 2002 -> raw byte length`.
        let raw = form_field(&body, "post[raw]");
        recorder
            .lock()
            .expect("recorder lock")
            .puts
            .push(("post 2002".to_string(), raw.len().to_string()));
        ("HTTP/1.1 200 OK", r#"{"success":"OK"}"#.to_string())
    } else if method == "GET" && path.starts_with("/posts/3003.json") {
        // `dsc post move` looks up the post's current topic first.
        ("HTTP/1.1 200 OK", SERVER_POST_3003_JSON.to_string())
    } else if method == "POST" && path.starts_with("/t/") && path.ends_with("/move-posts.json") {
        // Form `destination_topic_id=D&post_ids[]=P...`; recorded as
        // `move post P from topic S -> topic D` per moved post, with S taken
        // from the request path the real run actually chose.
        let source = path
            .strip_prefix("/t/")
            .and_then(|p| p.strip_suffix("/move-posts.json"))
            .unwrap_or_default()
            .to_string();
        let dest = form_field(&body, "destination_topic_id");
        let mut recorder = recorder.lock().expect("recorder lock");
        for kv in body.split('&') {
            if let Some((k, v)) = kv.split_once('=')
                && urlencoding_decode(k)
                    .replace("%5B", "[")
                    .replace("%5D", "]")
                    == "post_ids[]"
            {
                recorder.puts.push((
                    format!("move post {v} from topic {source}"),
                    format!("topic {dest}"),
                ));
            }
        }
        (
            "HTTP/1.1 200 OK",
            format!(r#"{{"success":"OK","url":"/t/moved/{dest}"}}"#),
        )
    } else if method == "POST" && path == "/posts.json" {
        // Form `title=...&raw=...&category=7`; recorded as
        // `new "<title>" -> raw byte length`.
        let title = form_field(&body, "title");
        let raw = form_field(&body, "raw");
        recorder
            .lock()
            .expect("recorder lock")
            .puts
            .push((format!("new \"{title}\""), raw.len().to_string()));
        (
            "HTTP/1.1 200 OK",
            r#"{"topic_id":900,"id":9000}"#.to_string(),
        )
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

/// Full `application/x-www-form-urlencoded` field lookup (percent-decoding
/// multi-byte UTF-8 and newlines), for bodies where length matters.
fn form_field(body: &str, key: &str) -> String {
    let encoded = body
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| {
            urlencoding_decode(k)
                .replace("%5B", "[")
                .replace("%5D", "]")
                == key
        })
        .map(|(_, v)| v)
        .unwrap_or_default();
    let bytes = encoded.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
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
/// group create/update/delete have their own
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

/// Frozen server tag groups: `colours` exists with a stale tag set (planned
/// update), `sizes` is absent (planned create), `shapes` already matches the
/// file (planned no-op).
const SERVER_TAG_GROUPS_RECONCILE_JSON: &str = r#"{"tag_groups":[
    {"id":5,"name":"colours","tag_names":["red","blue"]},
    {"id":6,"name":"shapes","tag_names":["circle"]}
]}"#;

/// Taxonomy file for the group-reconciliation test. Every file tag lives in a
/// group, so nothing is `groupless_missing` and the real run exits cleanly.
const TAG_GROUP_FILE: &str = r#"version: 1
complete: true
tag_groups:
  - name: colours
    tags: [red, green]
  - name: sizes
    tags: [small, large]
  - name: shapes
    tags: [circle]
"#;

/// Parse `dsc tag push`'s `+ create group:` / `~ update group:` plan lines
/// into `group -> "tag, tag"`, matching the recorded group payloads.
fn parse_tag_group_planned_changes(stdout: &str) -> BTreeMap<String, String> {
    let mut changes = BTreeMap::new();
    for line in stdout.lines() {
        let line = line.trim_start();
        let Some(rest) = line
            .strip_prefix("+ create group: ")
            .or_else(|| line.strip_prefix("~ update group: "))
        else {
            continue;
        };
        let Some((name, tags_part)) = rest.split_once(" (tags: [") else {
            continue;
        };
        let tags = tags_part.trim_end_matches(')').trim_end_matches(']');
        changes.insert(name.to_string(), tags.to_string());
    }
    changes
}

#[test]
fn tag_push_group_plan_matches_the_real_run_against_the_same_frozen_state() {
    let recorder = Arc::new(Mutex::new(Recorder {
        tag_groups_json: Some(SERVER_TAG_GROUPS_RECONCILE_JSON.to_string()),
        ..Recorder::default()
    }));
    let url = start_mock(Arc::clone(&recorder));
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &url);
    let file = write_file(&dir, "tag-groups.yaml", TAG_GROUP_FILE);
    let file = file.to_str().expect("utf-8 temp path");

    assert_dry_run_plan_matches_real_run(
        &config_path,
        &recorder,
        &["--dry-run", "tag", "push", "mock", file],
        &["tag", "push", "mock", file],
        parse_tag_group_planned_changes,
        // `shapes` matches the server, so only the update and the create.
        &BTreeMap::from([
            ("colours".to_string(), "red, green".to_string()),
            ("sizes".to_string(), "small, large".to_string()),
        ]),
    );
}

/// Server tags for the creation-via-group test: `red`, `blue` and `circle`
/// already exist, so only the file tags absent from here (`green`, `small`,
/// `large`) can be planned as created.
const SERVER_TAGS_VIA_GROUP_JSON: &str = r#"{"tags":[
    {"id":1,"text":"red"},
    {"id":2,"text":"blue"},
    {"id":3,"text":"circle"}
]}"#;

#[test]
fn tag_push_creation_via_group_plan_matches_the_real_run_against_the_same_frozen_state() {
    let recorder = Arc::new(Mutex::new(Recorder {
        tags_json: Some(SERVER_TAGS_VIA_GROUP_JSON.to_string()),
        tag_groups_json: Some(SERVER_TAG_GROUPS_RECONCILE_JSON.to_string()),
        ..Recorder::default()
    }));
    let url = start_mock(Arc::clone(&recorder));
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &url);
    let file = write_file(&dir, "tag-groups.yaml", TAG_GROUP_FILE);
    let file = file.to_str().expect("utf-8 temp path");

    let dry_run_output = run_dsc(&["--dry-run", "tag", "push", "mock", file], &config_path);
    assert!(
        dry_run_output.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run_output.stderr)
    );
    assert!(
        recorder.lock().expect("recorder lock").puts.is_empty(),
        "dry-run must issue zero requests"
    );
    let planned: BTreeSet<String> = String::from_utf8_lossy(&dry_run_output.stdout)
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("+ create tag: "))
        .filter_map(|rest| rest.split_once(" (via its tag group)"))
        .map(|(name, _)| name.to_string())
        .collect();
    assert_eq!(
        planned,
        ["green", "large", "small"].map(String::from).into(),
        "unexpected dry-run tag-creation plan"
    );

    let real_output = run_dsc(&["tag", "push", "mock", file], &config_path);
    assert!(
        real_output.status.success(),
        "{}",
        String::from_utf8_lossy(&real_output.stderr)
    );
    // The only way Discourse materialises a tag is a group payload naming
    // it, so the tags the real run creates are those in a POSTed/PUT group
    // that the frozen server did not already have.
    let existing: BTreeSet<&str> = ["red", "blue", "circle"].into();
    let created: BTreeSet<String> = recorder
        .lock()
        .expect("recorder lock")
        .puts
        .iter()
        .flat_map(|(_group, tags)| tags.split(", "))
        .filter(|t| !t.is_empty() && !existing.contains(t))
        .map(String::from)
        .collect();
    assert_eq!(
        planned, created,
        "the plan's `+ create tag` lines must exactly match the new tags \
         carried by the real run's tag group requests"
    );
}

/// Frozen server state for the `--prune` test: `tag_old` is in no desired
/// group and not in the file (planned tag delete); group `stale` (id 6) is
/// absent from the file (planned group delete); group `keep` matches.
const SERVER_TAGS_PRUNE_JSON: &str = r#"{"tags":[
    {"id":1,"text":"tag_a"},
    {"id":2,"text":"tag_old"}
]}"#;

const SERVER_TAG_GROUPS_PRUNE_JSON: &str = r#"{"tag_groups":[
    {"id":5,"name":"keep","tag_names":["tag_a"]},
    {"id":6,"name":"stale","tag_names":["tag_old"]}
]}"#;

const TAG_PRUNE_FILE: &str = r#"version: 1
complete: true
tag_groups:
  - name: keep
    tags: [tag_a]
"#;

/// Parse `dsc tag push --prune`'s `- delete tag:` / `- delete group:` plan
/// lines into `delete tag <name>` / `delete group <name>` keys with empty
/// values, matching what the mock records for the DELETE requests.
fn parse_tag_prune_planned_changes(stdout: &str) -> BTreeMap<String, String> {
    let mut changes = BTreeMap::new();
    for line in stdout.lines() {
        let line = line.trim_start();
        if let Some(name) = line.strip_prefix("- delete tag: ") {
            changes.insert(format!("delete tag {name}"), String::new());
        } else if let Some(name) = line.strip_prefix("- delete group: ") {
            changes.insert(format!("delete group {name}"), String::new());
        }
    }
    changes
}

#[test]
fn tag_push_prune_plan_matches_the_real_run_against_the_same_frozen_state() {
    let recorder = Arc::new(Mutex::new(Recorder {
        tags_json: Some(SERVER_TAGS_PRUNE_JSON.to_string()),
        tag_groups_json: Some(SERVER_TAG_GROUPS_PRUNE_JSON.to_string()),
        ..Recorder::default()
    }));
    let url = start_mock(Arc::clone(&recorder));
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &url);
    let file = write_file(&dir, "tag-prune.yaml", TAG_PRUNE_FILE);
    let file = file.to_str().expect("utf-8 temp path");

    assert_dry_run_plan_matches_real_run(
        &config_path,
        &recorder,
        &["--dry-run", "tag", "push", "mock", file, "--prune"],
        &["tag", "push", "mock", file, "--prune", "--yes"],
        parse_tag_prune_planned_changes,
        &BTreeMap::from([
            ("delete tag tag_old".to_string(), String::new()),
            ("delete group stale".to_string(), String::new()),
        ]),
    );
}

/// Parse `dsc category push`'s plan lines into `target -> byte length`:
/// `~ file → topic N (B bytes)` becomes `topic N -> B`, and
/// `+ file → new topic "T" (B bytes)` becomes `new "T" -> B`. `=` lines are
/// planned no-ops and are intentionally not collected.
fn parse_category_planned_changes(stdout: &str) -> BTreeMap<String, String> {
    let mut changes = BTreeMap::new();
    for line in stdout.lines() {
        let line = line.trim_start();
        let Some((_file, rest)) = line
            .strip_prefix("~ ")
            .or_else(|| line.strip_prefix("+ "))
            .and_then(|r| r.split_once(" \u{2192} "))
        else {
            continue;
        };
        let Some((target, bytes)) = rest.rsplit_once(" (") else {
            continue;
        };
        let bytes = bytes.trim_end_matches(" bytes)");
        let target = match target.strip_prefix("new topic ") {
            Some(title) => format!("new {title}"),
            None => target.to_string(),
        };
        changes.insert(target, bytes.to_string());
    }
    changes
}

#[test]
fn category_push_dry_run_plan_matches_the_real_run_against_the_same_frozen_state() {
    let recorder = Arc::new(Mutex::new(Recorder::default()));
    let url = start_mock(Arc::clone(&recorder));
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &url);
    let docs = dir.path().join("docs");
    std::fs::create_dir(&docs).expect("create docs dir");
    // alpha differs from the remote OP (update), beta matches it (no-op),
    // gamma has no remote topic (create, titled from its `# ` heading).
    std::fs::write(docs.join("alpha.md"), "new alpha body, caf\u{e9}\n").expect("write alpha");
    std::fs::write(docs.join("beta.md"), "same beta body\n").expect("write beta");
    std::fs::write(docs.join("gamma.md"), "# Gamma\n\nbrand new\n").expect("write gamma");
    let docs = docs.to_str().expect("utf-8 temp path");

    let alpha_len = "new alpha body, caf\u{e9}\n".len().to_string();
    let gamma_len = "# Gamma\n\nbrand new\n".len().to_string();

    assert_dry_run_plan_matches_real_run(
        &config_path,
        &recorder,
        &["--dry-run", "category", "push", "mock", "7", docs],
        &["category", "push", "mock", "7", docs],
        parse_category_planned_changes,
        &BTreeMap::from([
            ("topic 101".to_string(), alpha_len),
            ("new \"Gamma\"".to_string(), gamma_len),
        ]),
    );
}

/// Parse `dsc topic push`'s single plan line,
/// `[dry-run] mock: would replace OP of topic N (post id P) with B bytes from F`,
/// into `topic N -> B` - the same key the mock records for the real run's
/// `PUT /posts/1001.json` (post 1001 is topic 101's OP in the fixture).
fn parse_topic_planned_changes(stdout: &str) -> BTreeMap<String, String> {
    let mut changes = BTreeMap::new();
    for line in stdout.lines() {
        let Some((_, rest)) = line.split_once("would replace OP of topic ") else {
            continue;
        };
        let Some((topic, rest)) = rest.split_once(" (post id ") else {
            continue;
        };
        let Some((_, rest)) = rest.split_once(") with ") else {
            continue;
        };
        let Some((bytes, _file)) = rest.split_once(" bytes from ") else {
            continue;
        };
        changes.insert(format!("topic {topic}"), bytes.to_string());
    }
    changes
}

#[test]
fn topic_push_dry_run_plan_matches_the_real_run_against_the_same_frozen_state() {
    let recorder = Arc::new(Mutex::new(Recorder::default()));
    let url = start_mock(Arc::clone(&recorder));
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &url);
    // Local-only front matter must be stripped before both the planned byte
    // count and the real PUT; the multi-byte body checks bytes, not chars.
    let body = "new alpha body, caf\u{e9}\n";
    let file = write_file(
        &dir,
        "alpha.md",
        &format!("---\ntitle: Alpha\ntopic_id: 101\n---\n{body}"),
    );
    let file = file.to_str().expect("utf-8 temp path");

    assert_dry_run_plan_matches_real_run(
        &config_path,
        &recorder,
        &[
            "--dry-run",
            "topic",
            "push",
            "mock",
            "101",
            file,
            "--no-bump",
        ],
        &["topic", "push", "mock", "101", file, "--no-bump"],
        parse_topic_planned_changes,
        &BTreeMap::from([("topic 101".to_string(), body.len().to_string())]),
    );
}

/// Parse `dsc post push`'s single plan line,
/// `[dry-run] mock: would replace post P with B bytes`, into `post P -> B` -
/// the same key the mock records for the real run's `PUT /posts/P.json`.
fn parse_post_push_planned_changes(stdout: &str) -> BTreeMap<String, String> {
    let mut changes = BTreeMap::new();
    for line in stdout.lines() {
        let Some((_, rest)) = line.split_once("would replace post ") else {
            continue;
        };
        let Some((post, rest)) = rest.split_once(" with ") else {
            continue;
        };
        let Some(bytes) = rest.strip_suffix(" bytes") else {
            continue;
        };
        changes.insert(format!("post {post}"), bytes.to_string());
    }
    changes
}

#[test]
fn post_push_dry_run_plan_matches_the_real_run_against_the_same_frozen_state() {
    let recorder = Arc::new(Mutex::new(Recorder::default()));
    let url = start_mock(Arc::clone(&recorder));
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &url);
    // Multi-byte body so the planned count must be bytes, not chars.
    let body = "edited reply, na\u{ef}ve r\u{e9}sum\u{e9}\n";
    let file = write_file(&dir, "reply.md", body);
    let file = file.to_str().expect("utf-8 temp path");

    assert_dry_run_plan_matches_real_run(
        &config_path,
        &recorder,
        &["--dry-run", "post", "push", "mock", "2002", file],
        &["post", "push", "mock", "2002", file],
        parse_post_push_planned_changes,
        &BTreeMap::from([("post 2002".to_string(), body.len().to_string())]),
    );
}

/// Parse `dsc post move`'s single plan line,
/// `[dry-run] mock: would move post P from topic S to topic D`, into
/// `move post P from topic S -> topic D` - the same key/value the mock
/// records from the real run's `POST /t/S/move-posts.json` form body.
fn parse_post_move_planned_changes(stdout: &str) -> BTreeMap<String, String> {
    let mut changes = BTreeMap::new();
    for line in stdout.lines() {
        let Some((_, rest)) = line.split_once("would move post ") else {
            continue;
        };
        let Some((from, dest)) = rest.rsplit_once(" to topic ") else {
            continue;
        };
        changes.insert(format!("move post {from}"), format!("topic {dest}"));
    }
    changes
}

#[test]
fn post_move_dry_run_plan_matches_the_real_run_against_the_same_frozen_state() {
    let recorder = Arc::new(Mutex::new(Recorder::default()));
    let url = start_mock(Arc::clone(&recorder));
    let dir = TempDir::new().expect("tempdir");
    let config_path = mock_config(&dir, &url);

    assert_dry_run_plan_matches_real_run(
        &config_path,
        &recorder,
        &[
            "--dry-run",
            "post",
            "move",
            "mock",
            "3003",
            "--to-topic",
            "555",
        ],
        &["post", "move", "mock", "3003", "--to-topic", "555"],
        parse_post_move_planned_changes,
        &BTreeMap::from([(
            "move post 3003 from topic 101".to_string(),
            "topic 555".to_string(),
        )]),
    );
}

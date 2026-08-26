// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Request-budget tests: assert exact or maximum HTTP request counts for
//! specific commands against the mock Discourse, so a regression that
//! re-introduces an N+1 read or an unnecessary extra request fails CI
//! rather than silently degrading. These tests do not assert wall-clock
//! time; they count requests.
//!
//! The mock server and its request log are shared with
//! `dry-run-mutation-test.rs`. Each test starts a fresh mock, runs one
//! command, locks the log, and counts GET requests matching a pattern.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

// Re-use the same mock shape as dry-run-mutation-test.rs. Duplicated
// rather than extracted into a shared module to keep the test file
// self-contained and avoid a non-trivial refactor of the test harness.

fn get_body(path: &str) -> String {
    let p = path.split('?').next().unwrap_or(path);
    let post = r#"{"id":1,"topic_id":7,"post_number":1,"raw":"hello","cooked":"<p>hello</p>","username":"tester","created_at":"2026-01-01T00:00:00.000Z","category_id":4}"#;
    let category = r#"{"id":4,"name":"Test","slug":"test","color":"0088CC","text_color":"FFFFFF","position":1,"description":"d","read_restricted":false,"permission":1,"topic_template":"","allowed_tags":[],"allowed_tag_groups":[]}"#;
    let theme = r#"{"id":1,"name":"Test theme","component":false,"enabled":true,"user_selectable":true,"default":false,"color_scheme_id":19,"theme_fields":[{"target":"common","name":"scss","value":"body{}","type_id":1}],"settings":[{"setting":"k","value":"old","type":"string","default":"old"}],"child_themes":[{"id":2,"name":"Child"}],"remote_theme":null}"#;
    let group = r#"{"id":41,"name":"testgroup","full_name":"Test Group","user_count":1}"#;

    if p == "/site.json" {
        return r#"{"site":{"title":"Mock"}}"#.to_string();
    }
    if p == "/about.json" {
        return r#"{"about":{"version":"3.0.0","installed_version":"3.0.0"}}"#.to_string();
    }
    if p == "/t/888.json" {
        return r#"{"id":888,"title":"Deleted topic","slug":"deleted-topic","category_id":4,"posts_count":1,"deleted_at":"2026-06-25T00:00:00.000Z","post_stream":{"posts":[],"stream":[]}}"#.to_string();
    }
    if p.starts_with("/t/") && p.ends_with("/posts.json") {
        return format!(r#"{{"post_stream":{{"posts":[{post}]}}}}"#);
    }
    if p.starts_with("/t/") {
        return format!(
            r#"{{"id":7,"title":"Test topic","slug":"test-topic","category_id":4,"posts_count":1,"tags":["alpha"],"created_at":"2026-01-01T00:00:00.000Z","post_stream":{{"posts":[{post}],"stream":[1]}}}}"#
        );
    }
    if p == "/posts/999.json" {
        return r#"{"id":999,"topic_id":888,"post_number":3,"raw":"hello","cooked":"<p>hello</p>","username":"tester","created_at":"2026-01-01T00:00:00.000Z","category_id":4,"deleted_at":"2026-06-30T14:31:08Z"}"#.to_string();
    }
    if p.starts_with("/posts/") {
        return post.to_string();
    }
    if p.starts_with("/categories.json") {
        return format!(r#"{{"category_list":{{"categories":[{category}]}}}}"#);
    }
    if p.starts_with("/c/") {
        return format!(
            r#"{{"category":{category},"topic_list":{{"topics":[{{"id":7,"title":"Test topic","slug":"test-topic","posts_count":1,"category_id":4}}],"more_topics_url":null}}}}"#
        );
    }
    if p == "/admin/themes.json" {
        return format!(r#"{{"themes":[{theme}]}}"#);
    }
    if p.starts_with("/admin/themes/") {
        return format!(r#"{{"theme":{theme}}}"#);
    }
    if p == "/admin/site_settings.json" {
        return r#"{"site_settings":[{"setting":"title","value":"Mock","default":"Discourse","type":"string","category":"required","description":"t"}]}"#.to_string();
    }
    if p == "/tags.json" {
        return r#"{"tags":[{"id":1,"text":"alpha","count":1,"pm_count":0,"description":"d"}]}"#
            .to_string();
    }
    if p.starts_with("/tag/") {
        return r#"{"tag":{"id":1,"text":"alpha"},"topic_list":{"topics":[],"more_topics_url":null}}"#.to_string();
    }
    if p == "/tag_groups.json" {
        return r#"{"tag_groups":[{"id":1,"name":"G","tag_names":["alpha"],"one_per_topic":false,"parent_tag_name":null,"permissions":{}}]}"#.to_string();
    }
    if p.starts_with("/groups/") {
        return format!(r#"{{"group":{group}}}"#);
    }
    if p.contains("groups") {
        return format!(r#"{{"groups":[{group}]}}"#);
    }
    if p.starts_with("/admin/users/list/") {
        return r#"[{"id":2,"username":"tester","email":"t@example.com"}]"#.to_string();
    }
    if p.starts_with("/admin/users/") || p.starts_with("/u/") {
        return r#"{"user":{"id":2,"username":"Tester","email":"t@example.com"}}"#.to_string();
    }
    if p == "/admin/customize/colors.json" {
        return r#"[{"id":19,"name":"Palette","colors":[],"base_scheme_id":null}]"#.to_string();
    }
    if p == "/admin/color_schemes.json" {
        return r#"[{"id":19,"name":"Palette","colors":[{"name":"primary","hex":"000000"}],"base_scheme_id":null}]"#.to_string();
    }
    if p == "/admin/backups.json" {
        return "[]".to_string();
    }
    if p == "/user_actions.json" {
        return r#"{"user_actions":[],"total_rows":0}"#.to_string();
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
        r#"{"success":"OK","id":999,"topic_id":999}"#.to_string()
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

fn run_dsc(args: &[&str], config: &Path) -> (String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .args(args)
        .env("DSC_CONFIG", config)
        .output()
        .expect("running dsc");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (combined, output.status.success())
}

fn make_config(baseurl: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let config = dir.path().join("dsc.toml");
    std::fs::write(
        &config,
        format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"mock-key\"\napi_username = \"tester\"\n"
        ),
    )
    .expect("write config");
    (dir, config)
}

/// Count GET requests whose path starts with the given prefix.
fn count_gets(log: &Arc<Mutex<Vec<String>>>, prefix: &str) -> usize {
    log.lock()
        .expect("mock log poisoned")
        .iter()
        .filter(|entry| entry.starts_with("GET ") && entry.contains(prefix))
        .count()
}

// ─── P2: tag pull should not N+1 detail-read each tag ─────────────────────

#[test]
fn tag_pull_uses_one_list_request_not_n_plus_1() {
    let (baseurl, log) = start_mock();
    let (_dir, config) = make_config(&baseurl);
    let out = TempDir::new().expect("tempdir");
    let out_file = out.path().join("tags.yaml");
    let (output, ok) = run_dsc(
        &["tag", "pull", "mock", out_file.to_str().unwrap()],
        &config,
    );
    assert!(ok, "tag pull failed: {output}");

    // /tags.json returns one tag. The old code also called /tag/{name}.json
    // per tag for the description, but TagInfo already carries it. Assert
    // that the list endpoint is hit and no per-tag detail read occurs.
    let tag_list_gets = count_gets(&log, "/tags.json");
    assert_eq!(
        tag_list_gets, 1,
        "expected exactly one /tags.json request, got {tag_list_gets}"
    );

    let tag_detail_gets = count_gets(&log, "/tag/");
    // The mock returns one tag; a per-tag detail read would be one GET
    // to /tag/alpha.json. Assert zero - the description is in the list.
    assert_eq!(
        tag_detail_gets, 0,
        "tag pull should not issue per-tag detail requests (N+1), but got {tag_detail_gets}"
    );
}

// ─── P17: setting pull should not fetch the homepage for version stamp ────

#[test]
fn setting_pull_does_not_request_homepage_for_version() {
    let (baseurl, log) = start_mock();
    let (_dir, config) = make_config(&baseurl);
    let dir = TempDir::new().expect("tempdir");
    let out = dir.path().join("settings.yaml");
    let (output, ok) = run_dsc(&["setting", "pull", "mock", out.to_str().unwrap()], &config);
    assert!(ok, "setting pull failed: {output}");

    // The version stamp used to fetch both /about.json and / (the homepage
    // HTML) to extract the commit. The fix makes /about.json sufficient.
    // Assert that GET / (the homepage) is never requested.
    let homepage_gets = log
        .lock()
        .expect("mock log poisoned")
        .iter()
        .filter(|entry| {
            let line = entry.lines().next().unwrap_or("");
            line == "GET /" || line == "GET / HTTP/1.1"
        })
        .count();
    assert_eq!(
        homepage_gets, 0,
        "setting pull should not fetch the homepage (/) for a version stamp, but got {homepage_gets}"
    );

    // /about.json should be requested at most once.
    let about_gets = count_gets(&log, "/about.json");
    assert!(
        about_gets <= 1,
        "setting pull should request /about.json at most once, but got {about_gets}"
    );
}

// ─── P19: category copy should not fetch the catalogue twice ──────────────

#[test]
fn category_copy_fetches_categories_once() {
    let (baseurl, log) = start_mock();
    let (_dir, config) = make_config(&baseurl);
    let (output, ok) = run_dsc(
        &["category", "copy", "mock", "4", "--target", "mock"],
        &config,
    );
    // category copy may or may not succeed against the mock (it tries to
    // create a category), but we only care about the request count.
    let _ = (output, ok);

    // The old code called fetch_categories twice: once to resolve the ID
    // and once to retrieve the category. Assert at most one /categories.json.
    let cat_gets = count_gets(&log, "/categories.json");
    assert!(
        cat_gets <= 1,
        "category copy should fetch /categories.json at most once, but got {cat_gets}"
    );
}

// ─── P18: live topic delete should not GET before DELETE ─────────────────

#[test]
fn topic_delete_does_not_prefetch_topic_detail() {
    let (baseurl, log) = start_mock();
    let (_dir, config) = make_config(&baseurl);
    // Live delete (not dry-run) of topic 7.
    let (output, ok) = run_dsc(&["topic", "delete", "mock", "7", "--force"], &config);
    // May succeed or fail against the mock; we count requests either way.
    let _ = (output, ok);

    // The old code fetched topic detail before every DELETE to print the
    // title. The fix skips the GET in live mode. Assert no GET to /t/7.json
    // (the topic detail endpoint) - only the DELETE should appear.
    let topic_gets = log
        .lock()
        .expect("mock log poisoned")
        .iter()
        .filter(|entry| entry.starts_with("GET ") && entry.contains("/t/7.json"))
        .count();
    assert_eq!(
        topic_gets, 0,
        "live topic delete should not prefetch topic detail (GET /t/7.json), but got {topic_gets}"
    );
}

// ─── P31: tag DELETE should go through the retry path ───────────────────

#[test]
fn tag_push_delete_uses_retrying_send_path() {
    // This is harder to test without a rate-limit mock, but we can at least
    // verify that a DELETE request appears in the log when a tag is pruned.
    // The old code bypassed send_retrying for DELETE; the fix routes through
    // delete_builder + send_retrying. Both paths produce a DELETE in the log,
    // so this test guards against accidental removal of the DELETE entirely.
    let (baseurl, log) = start_mock();
    let (_dir, config) = make_config(&baseurl);
    let dir = TempDir::new().expect("tempdir");
    let tags_file = dir.path().join("tags.yaml");
    // Empty taxonomy: no tags, no groups, but marked complete so --prune
    // is allowed. Push with --prune should DELETE the one tag the mock returns.
    std::fs::write(
        &tags_file,
        "version: 1\ncomplete: true\ntags: []\ntag_groups: []\n",
    )
    .expect("write tags file");
    let (output, _ok) = run_dsc(
        &[
            "tag",
            "push",
            "mock",
            tags_file.to_str().unwrap(),
            "--prune",
            "--yes",
        ],
        &config,
    );
    let _ = output;

    let delete_gets = log
        .lock()
        .expect("mock log poisoned")
        .iter()
        .filter(|entry| entry.starts_with("DELETE "))
        .count();
    // The mock returns one tag, so a push of an empty list should DELETE it.
    assert!(
        delete_gets >= 1,
        "tag push with an empty list should DELETE the existing tag, but got {delete_gets} DELETEs"
    );
}

// ─── P12: setting audit request count is one per forum ──────────────────

#[test]
fn setting_audit_makes_one_settings_request_per_forum() {
    let (baseurl, log) = start_mock();
    let (_dir, config) = make_config(&baseurl);
    let (output, ok) = run_dsc(&["setting", "audit", "title"], &config);
    assert!(ok, "setting audit failed: {output}");

    // Each forum fetches the whole settings catalogue once. With one forum,
    // that's exactly one GET to /admin/site_settings.json.
    let settings_gets = count_gets(&log, "/admin/site_settings.json");
    assert_eq!(
        settings_gets, 1,
        "setting audit should request /admin/site_settings.json exactly once for one forum, but got {settings_gets}"
    );
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Behavioural guarantee for the global `--dry-run` contract.
//!
//! The pre-existing dry-run tests check two things: that the *refusing*
//! commands short-circuit before configuration is resolved, and that
//! `dry_run_refusal_reason` classifies each command as refuse/allow. Neither
//! asserts what a non-refusing command actually *does*. That gap is not
//! hypothetical: `dsc explorer run` was added to the "allowed" list while it
//! still issued `POST .../run.json` under `--dry-run`, and every test passed.
//!
//! This test closes it end to end. Each mutating command runs against a local
//! mock Discourse that records every request method, and the run fails if any
//! command issues a POST, PUT, or DELETE. `dry_run_coverage` then walks the
//! real clap tree so a newly added command cannot silently escape triage.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

// ─── mock Discourse ───────────────────────────────────────────────────────────

fn webhook_list_body(path: &str) -> String {
    let offset = path
        .split_once('?')
        .and_then(|(_, query)| {
            query.split('&').find_map(|pair| {
                pair.split_once("offset=")
                    .and_then(|(_, value)| value.parse::<u64>().ok())
            })
        })
        .unwrap_or(0);
    let ids: Vec<u64> = match offset {
        0 => (1..=50).collect(),
        50 => vec![51],
        _ => Vec::new(),
    };
    let web_hooks = ids
        .into_iter()
        .map(|id| {
            format!(
                r#"{{"id":{id},"payload_url":"https://user:url-canary@example.test/hooks/{id}","content_type":1,"active":true,"wildcard_web_hook":true,"secret":"secret-canary","verify_certificate":true,"last_delivery_status":3,"category_ids":[],"group_ids":[],"tags":[],"web_hook_event_types":[]}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"web_hooks":[{web_hooks}],"extras":{{"default_event_types":[{{"id":201,"name":"post_created","group":"post"}},{{"id":202,"name":"post_edited","group":"post"}},{{"id":203,"name":"post_destroyed","group":"post"}},{{"id":204,"name":"topic_created","group":"topic"}}]}},"total_rows_web_hooks":51,"load_more_web_hooks":"/admin/api/web_hooks.json?limit=50&offset=50"}}"#
    )
}

/// Canned GET bodies, shaped just well enough that each command reaches its
/// dry-run decision point instead of erroring on a malformed response.
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
    if p == "/admin/api/web_hooks.json" {
        return webhook_list_body(path);
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
        return r#"{"user":{"id":2,"username":"tester","email":"t@example.com"}}"#.to_string();
    }
    if p == "/admin/customize/colors.json" {
        return r#"[{"id":19,"name":"Palette","colors":[],"base_scheme_id":null}]"#.to_string();
    }
    if p == "/admin/backups.json" {
        return "[]".to_string();
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

    // Drain headers, then any body, so the client always sees a clean exchange.
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
    } else if method == "POST" && path == "/admin/api/web_hooks.json" {
        r#"{"web_hook":{"id":999,"payload_url":"https://user:server-url-canary@example.test/hook","content_type":1,"active":true,"wildcard_web_hook":true,"secret":"server-secret-canary","verify_certificate":true,"last_delivery_status":3,"category_ids":[],"group_ids":[],"tags":[],"web_hook_event_types":[]}}"#.to_string()
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

/// Start a mock Discourse on an ephemeral port. Returns its base URL and the
/// shared request log. The listener thread is detached; it dies with the test.
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

fn mutating(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    log.lock()
        .expect("mock log poisoned")
        .iter()
        .filter(|entry| {
            entry.starts_with("POST ") || entry.starts_with("PUT ") || entry.starts_with("DELETE ")
        })
        .cloned()
        .collect()
}

fn run_dsc(args: &[&str], config: &Path) -> (String, bool) {
    run_dsc_with_input(args, config, None)
}

fn run_dsc_with_input(args: &[&str], config: &Path, input: Option<&str>) -> (String, bool) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dsc"));
    command
        .args(args)
        .env("DSC_CONFIG", config)
        .env_remove("DSC_CONFIG_HOME")
        // The SSH-templated commands render these into their dry-run plan.
        .env("DSC_SSH_PLUGIN_INSTALL_CMD", "echo plugin install {url}")
        .env("DSC_SSH_PLUGIN_REMOVE_CMD", "echo plugin remove {name}")
        .env("DSC_SSH_THEME_REMOVE_CMD", "echo theme remove {name}");
    let out = if let Some(input) = input {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn dsc");
        child
            .stdin
            .take()
            .expect("dsc stdin")
            .write_all(input.as_bytes())
            .expect("write dsc stdin");
        child.wait_with_output().expect("wait for dsc")
    } else {
        command.output().expect("run dsc")
    };
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (combined, out.status.success())
}

// ─── the commands under test ──────────────────────────────────────────────────

/// `(leaf command, argv after `-n`, must print the `[dry-run]` marker)`.
///
/// `false` in the third slot means the command legitimately stops earlier on a
/// validation path (so no plan is printed); the no-mutation assertion still
/// applies, which is the property this test exists to protect.
type Case = (&'static str, &'static [&'static str], bool);

const CASES: &[Case] = &[
    (
        "topic reply",
        &["topic", "reply", "mock", "7", "BODY"],
        true,
    ),
    (
        "topic new",
        &[
            "topic",
            "new",
            "mock",
            "4",
            "--title",
            "A new topic title",
            "BODY",
        ],
        true,
    ),
    ("topic push", &["topic", "push", "mock", "7", "BODY"], true),
    ("topic sync", &["topic", "sync", "mock", "7", "BODY"], true),
    (
        "topic title",
        &["topic", "title", "mock", "7", "Renamed topic title"],
        true,
    ),
    ("topic delete", &["topic", "delete", "mock", "7"], true),
    ("topic restore", &["topic", "restore", "mock", "7"], true),
    ("topic tag", &["topic", "tag", "mock", "7", "beta"], true),
    (
        "topic untag",
        &["topic", "untag", "mock", "7", "alpha"],
        true,
    ),
    ("post push", &["post", "push", "mock", "1", "BODY"], true),
    ("post delete", &["post", "delete", "mock", "1"], true),
    (
        "post move",
        &["post", "move", "mock", "1", "--to-topic", "8"],
        true,
    ),
    (
        "pm send",
        &[
            "pm",
            "send",
            "mock",
            "tester",
            "--title",
            "A private message",
            "BODY",
        ],
        true,
    ),
    (
        "setting set",
        &["setting", "set", "mock", "title", "New Title"],
        true,
    ),
    (
        "setting push",
        &["setting", "push", "mock", "SETTINGS"],
        true,
    ),
    ("tag push", &["tag", "push", "mock", "TAGS"], true),
    (
        "tag rename",
        &["tag", "rename", "mock", "alpha", "gamma"],
        true,
    ),
    (
        "category push",
        &["category", "push", "mock", "4", "CATDIR"],
        true,
    ),
    (
        "category rename",
        &["category", "rename", "mock", "4", "New Category Name"],
        true,
    ),
    ("theme push", &["theme", "push", "mock", "THEME"], true),
    ("theme delete", &["theme", "delete", "mock", "1"], true),
    ("theme enable", &["theme", "enable", "mock", "1"], true),
    ("theme disable", &["theme", "disable", "mock", "1"], true),
    // The mock theme already has child 2, so attach a different component.
    ("theme attach", &["theme", "attach", "mock", "1", "3"], true),
    ("theme detach", &["theme", "detach", "mock", "1", "2"], true),
    (
        "theme setting set",
        &["theme", "setting", "set", "mock", "1", "k", "v"],
        true,
    ),
    (
        "theme field push",
        &["theme", "field", "push", "mock", "1", "common/scss", "BODY"],
        true,
    ),
    (
        "theme asset set",
        &["theme", "asset", "set", "mock", "1", "logo", "BODY"],
        true,
    ),
    // Refuses first when the named asset is absent from the mock theme.
    (
        "theme asset unset",
        &["theme", "asset", "unset", "mock", "1", "logo"],
        false,
    ),
    ("user suspend", &["user", "suspend", "mock", "tester"], true),
    (
        "user unsuspend",
        &["user", "unsuspend", "mock", "tester"],
        true,
    ),
    ("user silence", &["user", "silence", "mock", "tester"], true),
    (
        "user unsilence",
        &["user", "unsilence", "mock", "tester"],
        true,
    ),
    (
        "user promote",
        &["user", "promote", "mock", "tester", "--role", "moderator"],
        true,
    ),
    (
        "user demote",
        &["user", "demote", "mock", "tester", "--role", "moderator"],
        true,
    ),
    (
        "user create",
        &["user", "create", "mock", "n@example.com", "newbie"],
        true,
    ),
    (
        "user password-reset",
        &["user", "password-reset", "mock", "tester"],
        true,
    ),
    (
        "user email-set",
        &["user", "email-set", "mock", "tester", "new@example.com"],
        true,
    ),
    (
        "user groups add",
        &["user", "groups", "add", "mock", "tester", "41"],
        true,
    ),
    (
        "user groups remove",
        &["user", "groups", "remove", "mock", "tester", "41"],
        true,
    ),
    ("group add", &["group", "add", "mock", "41", "EMAILS"], true),
    (
        "group copy",
        &["group", "copy", "mock", "41", "--target", "mock"],
        true,
    ),
    (
        "invite send",
        &["invite", "send", "mock", "n@example.com"],
        true,
    ),
    ("invite bulk", &["invite", "bulk", "mock", "EMAILS"], true),
    (
        "api-key create",
        &["api-key", "create", "mock", "audit key"],
        true,
    ),
    ("api-key revoke", &["api-key", "revoke", "mock", "1"], true),
    (
        "webhook create",
        &[
            "webhook",
            "create",
            "mock",
            "https://user:url-canary@example.test/hook",
        ],
        true,
    ),
    ("webhook delete", &["webhook", "delete", "mock", "1"], true),
    ("webhook ping", &["webhook", "ping", "mock", "1"], true),
    ("backup create", &["backup", "create", "mock"], true),
    (
        "backup restore",
        &["backup", "restore", "mock", "some-backup.tar.gz"],
        true,
    ),
    ("emoji push", &["emoji", "push", "mock", "BODY"], true),
    ("upload", &["upload", "mock", "BODY"], true),
    (
        "notification read",
        &["notification", "read", "mock", "--all"],
        true,
    ),
    ("explorer run", &["explorer", "run", "mock", "1"], true),
    (
        "explorer run --csv",
        &["explorer", "run", "mock", "1", "--csv", "CSVOUT"],
        true,
    ),
    ("topic tags", &["topic", "tags", "mock", "7", "beta"], true),
    (
        "category copy",
        &["category", "copy", "mock", "4", "--target", "mock"],
        true,
    ),
    (
        "category def push",
        &["category", "def", "push", "mock", "CATDEF"],
        true,
    ),
    ("backup push", &["backup", "push", "mock", "BODY"], true),
    (
        "theme setting push",
        &["theme", "setting", "push", "mock", "1", "THEMESET"],
        true,
    ),
    // Refuse-before-dispatch commands: the marker is the refusal, not a plan.
    (
        "theme duplicate",
        &["theme", "duplicate", "mock", "1"],
        true,
    ),
    ("theme update", &["theme", "update", "mock", "1"], true),
    (
        "palette push",
        &["palette", "push", "mock", "PALETTE"],
        true,
    ),
    (
        "theme palette push",
        &["theme", "palette", "push", "mock", "PALETTE"],
        true,
    ),
    ("update", &["update", "mock"], true),
    // SSH-driven mutations: a dry run must describe the remote command only.
    (
        "theme install",
        &["theme", "install", "mock", "https://example.invalid/t.git"],
        true,
    ),
    (
        "theme remove",
        &["theme", "remove", "mock", "Test theme"],
        true,
    ),
    (
        "plugin install",
        &["plugin", "install", "mock", "https://example.invalid/p.git"],
        true,
    ),
    (
        "plugin remove",
        &["plugin", "remove", "mock", "someplugin"],
        true,
    ),
    (
        "app env set",
        &["app", "env", "set", "mock", "DISCOURSE_KEY", "value"],
        false,
    ),
    (
        "app env unset",
        &["app", "env", "unset", "mock", "DISCOURSE_KEY"],
        false,
    ),
    ("backup setup-s3", &["backup", "setup-s3", "mock"], true),
];

#[test]
fn dry_run_never_issues_a_mutating_request() {
    let (baseurl, log) = start_mock();
    let dir = TempDir::new().expect("tempdir");

    let config = dir.path().join("dsc.toml");
    std::fs::write(
        &config,
        format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"mock-key\"\n\
             api_username = \"tester\"\nssh_host = \"mock.invalid\"\nchangelog_topic_id = 7\n"
        ),
    )
    .expect("write config");

    let body = dir.path().join("body.md");
    std::fs::write(&body, "some body text\n").expect("write body");
    let settings = dir.path().join("settings.yaml");
    std::fs::write(
        &settings,
        "version: 1\ncomplete: true\nsettings:\n  - name: title\n    value: Changed\n",
    )
    .expect("write settings");
    let tags = dir.path().join("tags.yaml");
    std::fs::write(
        &tags,
        "version: 1\ntags:\n  - name: alpha\n    description: changed\ntag_groups: []\n",
    )
    .expect("write tags");
    let theme = dir.path().join("theme.json");
    std::fs::write(&theme, "{\"name\":\"New theme\"}\n").expect("write theme");
    let emails = dir.path().join("emails.txt");
    std::fs::write(&emails, "n@example.com\n").expect("write emails");
    let catdir = dir.path().join("catdir");
    std::fs::create_dir_all(&catdir).expect("catdir");
    std::fs::write(catdir.join("t.md"), "---\ntopic_id: 7\n---\nnew body\n").expect("write cat");
    let catdef = dir.path().join("categories.yaml");
    std::fs::write(
        &catdef,
        "version: 1\ncategories:\n  - id: 4\n    name: Renamed Test\n    slug: test\n",
    )
    .expect("write catdef");
    let themeset = dir.path().join("theme-settings.yaml");
    std::fs::write(
        &themeset,
        "version: 1\ntheme_id: 1\nsettings:\n  - setting: k\n    value: v\n",
    )
    .expect("write themeset");
    let palette = dir.path().join("palette.json");
    std::fs::write(&palette, "{\"name\":\"Palette\",\"colors\":[]}\n").expect("write palette");
    let csvout = dir.path().join("result.csv");

    let substitute = |arg: &str| -> String {
        match arg {
            "BODY" => body.display().to_string(),
            "SETTINGS" => settings.display().to_string(),
            "TAGS" => tags.display().to_string(),
            "THEME" => theme.display().to_string(),
            "EMAILS" => emails.display().to_string(),
            "CATDIR" => catdir.display().to_string(),
            "CATDEF" => catdef.display().to_string(),
            "THEMESET" => themeset.display().to_string(),
            "PALETTE" => palette.display().to_string(),
            "CSVOUT" => csvout.display().to_string(),
            other => other.to_string(),
        }
    };

    let mut failures: Vec<String> = Vec::new();
    for (label, argv, expects_plan) in CASES {
        let before = mutating(&log).len();
        let mut args: Vec<String> = vec!["-n".to_string()];
        args.extend(argv.iter().map(|a| substitute(a)));
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let (output, _ok) = run_dsc(&borrowed, &config);
        let after = mutating(&log);

        if after.len() != before {
            failures.push(format!(
                "{label}: issued {} mutating request(s) under --dry-run: {:?}",
                after.len() - before,
                &after[before..]
            ));
            continue;
        }
        if *expects_plan && !output.contains("[dry-run]") {
            failures.push(format!(
                "{label}: printed no [dry-run] plan or refusal, so it may have stopped \
                 before its dry-run decision point. Output: {}",
                output.lines().take(2).collect::<Vec<_>>().join(" | ")
            ));
        }
        if *label == "webhook create"
            && (output.contains("secret-canary") || output.contains("url-canary"))
        {
            failures.push(format!(
                "{label}: leaked a secret or URL credential in dry-run output: {output}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "--dry-run must never mutate the server:\n  {}",
        failures.join("\n  ")
    );

    // A CSV/export destination must not be written during a dry run either.
    assert!(
        !csvout.exists(),
        "explorer run --csv wrote {} during a dry run",
        csvout.display()
    );
}

// ─── coverage: every command must be triaged ──────────────────────────────────

/// Leaf commands that never mutate the forum: they only read, print, or write
/// local files. They still accept `--dry-run` (some refuse under it), but they
/// cannot violate the property this file protects, so they need no case above.
///
/// Keep this list honest. Listing a forum-mutating command here instead of in
/// `CASES` silently reopens the hole that let `explorer run` POST under a dry
/// run: `--dry-run` would go untested for it.
const NO_SERVER_MUTATION_LEAVES: &[&str] = &[
    "add",
    "analytics",
    "api-key list",
    "app env audit",
    "app env get",
    "app env list",
    "backup health",
    "backup list",
    "backup pull",
    "category def pull",
    "category diff",
    "category get",
    "category list",
    "category pull",
    "category set",
    "category show",
    "completions install",
    "config check",
    "doctor",
    "emoji list",
    "emoji pull",
    "explorer list",
    "explorer show",
    "group info",
    "group list",
    "group members",
    "harden",
    "import",
    "list tidy",
    "log staff",
    "man",
    "notification list",
    "open",
    "palette list",
    "palette pull",
    "plugin list",
    "pm list",
    "post info",
    "post pull",
    "sar",
    "search",
    "setting audit",
    "setting diff",
    "setting get",
    "setting list",
    "setting pull",
    "tag list",
    "tag pull",
    "theme asset list",
    "theme field list",
    "theme field pull",
    "theme list",
    "theme palette list",
    "theme palette pull",
    "theme pull",
    "theme setting get",
    "theme setting list",
    "theme setting pull",
    "theme show",
    "topic list",
    "topic pull",
    "update log",
    "user activity",
    "user groups list",
    "user info",
    "user list",
    "version",
    "webhook list",
];

#[test]
fn webhook_output_never_emits_secrets_or_url_credentials() {
    let (baseurl, _log) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let config = dir.path().join("dsc.toml");
    std::fs::write(
        &config,
        format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"mock-key\"\napi_username = \"tester\"\n"
        ),
    )
    .expect("write config");

    for args in [
        &["webhook", "list", "mock"] as &[&str],
        &["webhook", "list", "mock", "--format", "json"],
        &["webhook", "list", "mock", "--format", "yaml"],
    ] {
        let (output, ok) = run_dsc(args, &config);
        assert!(ok, "webhook list failed: {output}");
        assert!(!output.contains("secret-canary"), "secret leaked: {output}");
        assert!(
            !output.contains("url-canary"),
            "URL credential leaked: {output}"
        );
    }

    let (json, ok) = run_dsc(&["webhook", "list", "mock", "--format", "json"], &config);
    assert!(ok, "webhook JSON list failed: {json}");
    let rows: Vec<serde_json::Value> = serde_json::from_str(&json).expect("webhook JSON");
    assert_eq!(rows.len(), 51, "webhook list should follow offset pages");
}

#[test]
fn post_info_never_emits_raw_or_author() {
    let (baseurl, _log) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let config = dir.path().join("dsc.toml");
    std::fs::write(
        &config,
        format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"mock-key\"\napi_username = \"tester\"\n"
        ),
    )
    .expect("write config");

    for args in [
        &["post", "info", "mock", "1"] as &[&str],
        &["post", "info", "mock", "1", "--format", "json"],
        &["post", "info", "mock", "1", "--format", "yaml"],
    ] {
        let (output, ok) = run_dsc(args, &config);
        assert!(ok, "post info failed: {output}");
        assert!(!output.contains("hello"), "raw body leaked: {output}");
        assert!(!output.contains("tester"), "author leaked: {output}");
        assert!(!output.contains("<p>"), "cooked HTML leaked: {output}");
    }

    let (json, ok) = run_dsc(&["post", "info", "mock", "1", "--format", "json"], &config);
    assert!(ok, "post info JSON failed: {json}");
    let value: serde_json::Value = serde_json::from_str(&json).expect("post info JSON");
    assert_eq!(value["id"], 1);
    assert_eq!(value["topic"]["id"], 7);
    assert_eq!(value["topic"]["title"], "Test topic");
    assert_eq!(value["topic"]["slug"], "test-topic");
    assert_eq!(value["topic"]["category_id"], 4);
    assert_eq!(value["post_number"], 1);
    assert_eq!(value["url"], format!("{baseurl}/t/test-topic/7/1"));

    // A soft-deleted post in a soft-deleted topic still resolves, and the
    // deletion state on both post and topic is surfaced.
    let (deleted_json, ok) = run_dsc(
        &["post", "info", "mock", "999", "--format", "json"],
        &config,
    );
    assert!(ok, "post info (deleted) failed: {deleted_json}");
    assert!(
        !deleted_json.contains("hello") && !deleted_json.contains("tester"),
        "raw/author leaked for deleted post: {deleted_json}"
    );
    let deleted: serde_json::Value = serde_json::from_str(&deleted_json).expect("post info JSON");
    assert_eq!(deleted["id"], 999);
    assert_eq!(deleted["topic"]["id"], 888);
    assert_eq!(deleted["topic"]["title"], "Deleted topic");
    assert_eq!(deleted["deleted_at"], "2026-06-30T14:31:08Z");
    assert_eq!(deleted["topic"]["deleted_at"], "2026-06-25T00:00:00.000Z");
}

#[test]
fn webhook_secret_stdin_is_redacted_in_dry_run() {
    let (baseurl, log) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let config = dir.path().join("dsc.toml");
    std::fs::write(
        &config,
        format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"mock-key\"\napi_username = \"tester\"\n"
        ),
    )
    .expect("write config");

    let (output, ok) = run_dsc_with_input(
        &[
            "-n",
            "webhook",
            "create",
            "mock",
            "https://user:url-canary@example.test/hook",
            "--secret-stdin",
        ],
        &config,
        Some("secret-canary\n"),
    );
    assert!(ok, "webhook dry run failed: {output}");
    assert!(output.contains("event_types:201,202,203,204"));
    assert!(output.contains("secret:provided"));
    assert!(!output.contains("secret-canary"));
    assert!(!output.contains("url-canary"));
    assert!(
        mutating(&log).is_empty(),
        "dry run mutated: {:?}",
        mutating(&log)
    );
}

#[test]
fn webhook_mutations_use_verified_routes_and_payload() {
    let (baseurl, log) = start_mock();
    let dir = TempDir::new().expect("tempdir");
    let config = dir.path().join("dsc.toml");
    std::fs::write(
        &config,
        format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"{baseurl}\"\napikey = \"mock-key\"\napi_username = \"tester\"\n"
        ),
    )
    .expect("write config");

    let (create_output, create_ok) = run_dsc(
        &[
            "webhook",
            "create",
            "mock",
            "https://hooks.example.test/discourse",
            "--format",
            "json",
        ],
        &config,
    );
    assert!(create_ok, "webhook create failed: {create_output}");
    assert!(!create_output.contains("server-secret-canary"));
    assert!(!create_output.contains("server-url-canary"));

    let (delete_output, delete_ok) = run_dsc(
        &["webhook", "delete", "mock", "999", "--format", "json"],
        &config,
    );
    assert!(delete_ok, "webhook delete failed: {delete_output}");
    let (ping_output, ping_ok) = run_dsc(
        &["webhook", "ping", "mock", "999", "--format", "json"],
        &config,
    );
    assert!(ping_ok, "webhook ping failed: {ping_output}");

    let requests = log.lock().expect("mock log poisoned");
    let create = requests
        .iter()
        .find(|request| request.starts_with("POST /admin/api/web_hooks.json\n"))
        .expect("webhook create request");
    assert!(
        create.contains("web_hook%5Bpayload_url%5D=https%3A%2F%2Fhooks.example.test%2Fdiscourse")
    );
    assert!(create.contains("web_hook%5Bwildcard_web_hook%5D=true"));
    for event_type_id in [201, 202, 203, 204] {
        assert!(create.contains(&format!(
            "web_hook%5Bweb_hook_event_type_ids%5D%5B%5D={event_type_id}"
        )));
    }
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("DELETE /admin/api/web_hooks/999.json\n"))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("POST /admin/api/web_hooks/999/ping.json\n"))
    );
}

fn leaf_commands(cmd: &clap::Command, prefix: &str, out: &mut BTreeSet<String>) {
    let subs: Vec<_> = cmd
        .get_subcommands()
        .filter(|s| s.get_name() != "help")
        .collect();
    if subs.is_empty() {
        if !prefix.is_empty() {
            out.insert(prefix.to_string());
        }
        return;
    }
    for sub in subs {
        let next = if prefix.is_empty() {
            sub.get_name().to_string()
        } else {
            format!("{prefix} {}", sub.get_name())
        };
        leaf_commands(sub, &next, out);
    }
}

#[test]
fn every_command_is_triaged_for_dry_run() {
    use clap::CommandFactory;

    let mut leaves = BTreeSet::new();
    leaf_commands(&dsc::cli::Cli::command(), "", &mut leaves);

    let covered: BTreeSet<String> = CASES
        .iter()
        .map(|(label, _, _)| label.trim_end_matches(" --csv").to_string())
        .collect();
    let no_mutation: BTreeSet<String> = NO_SERVER_MUTATION_LEAVES
        .iter()
        .map(|s| s.to_string())
        .collect();

    let untriaged: Vec<&String> = leaves
        .iter()
        .filter(|leaf| !covered.contains(*leaf) && !no_mutation.contains(*leaf))
        .collect();

    assert!(
        untriaged.is_empty(),
        "these commands are not triaged for --dry-run. If a command changes server \
         or local state, add it to CASES in this file so its dry-run behaviour is \
         actually exercised; if it is genuinely read-only, add it to \
         NO_SERVER_MUTATION_LEAVES:\n  {untriaged:?}"
    );

    // Guard the guard: a stale entry means the surface moved under us.
    let stale: Vec<&String> = no_mutation
        .iter()
        .filter(|r| !leaves.contains(*r))
        .collect();
    assert!(
        stale.is_empty(),
        "NO_SERVER_MUTATION_LEAVES lists commands that no longer exist: {stale:?}"
    );
}

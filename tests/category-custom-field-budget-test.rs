// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! R91/R92 request-budget tests: single-category `show`, `get`, `set` and `diff`
//! resolve the requested category from one `/categories.json` listing, then
//! read `/c/:id/show.json` only for that category (and only when custom
//! fields are needed). A regression to "enrich every category first" fails
//! here because the mock forum carries several categories.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// Three categories: `Docs` (4), `Archive` (5) and a second `Archive` (6)
/// nested under Docs, so the name `Archive` is ambiguous but slugs are not.
const CATEGORIES: &[(u64, &str, &str, Option<u64>)] = &[
    (4, "Docs", "docs", None),
    (5, "Archive", "archive", None),
    (6, "Archive", "docs-archive", Some(4)),
];

fn category_json(id: u64, name: &str, slug: &str, parent: Option<u64>, full: bool) -> String {
    let parent = parent.map_or("null".to_string(), |p| p.to_string());
    // The list endpoint preloads no custom fields; the show endpoint carries
    // the complete map, so enrichment is observable in the output.
    let custom_fields = if full {
        format!(r#","custom_fields":{{"docs_index_id":"{}"}}"#, id * 100)
    } else {
        String::new()
    };
    format!(
        r#"{{"id":{id},"name":"{name}","slug":"{slug}","parent_category_id":{parent},"color":"0088CC","text_color":"FFFFFF","position":{id},"description":"d","read_restricted":false,"permission":1,"topic_template":"","allowed_tags":[],"allowed_tag_groups":[]{custom_fields}}}"#
    )
}

fn get_body(path: &str) -> (&'static str, String) {
    let p = path.split('?').next().unwrap_or(path);
    if p == "/categories.json" {
        let list: Vec<String> = CATEGORIES
            .iter()
            .map(|(id, name, slug, parent)| category_json(*id, name, slug, *parent, false))
            .collect();
        return (
            "200 OK",
            format!(
                r#"{{"category_list":{{"categories":[{}]}}}}"#,
                list.join(",")
            ),
        );
    }
    if let Some(rest) = p.strip_prefix("/c/")
        && let Some(id) = rest.strip_suffix("/show.json")
        && let Some((id, name, slug, parent)) = CATEGORIES
            .iter()
            .find(|(candidate, ..)| candidate.to_string() == id)
    {
        return (
            "200 OK",
            format!(
                r#"{{"category":{}}}"#,
                category_json(*id, name, slug, *parent, true)
            ),
        );
    }
    ("404 Not Found", r#"{"errors":["not found"]}"#.to_string())
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
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line == "\n" {
            break;
        }
    }
    log.lock()
        .expect("mock log poisoned")
        .push(format!("{method} {path}"));
    let (status, body) = if method == "GET" {
        get_body(&path)
    } else {
        ("200 OK", r#"{"success":"OK"}"#.to_string())
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

struct Mock {
    _dir: TempDir,
    config: PathBuf,
    log: Arc<Mutex<Vec<String>>>,
}

fn start_mock() -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    let log = Arc::new(Mutex::new(Vec::new()));
    let thread_log = Arc::clone(&log);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &thread_log);
        }
    });
    let dir = TempDir::new().expect("tempdir");
    let config = dir.path().join("dsc.toml");
    std::fs::write(
        &config,
        format!(
            "[[discourse]]\nname = \"mock\"\nbaseurl = \"http://{addr}\"\napikey = \"mock-key\"\napi_username = \"tester\"\n"
        ),
    )
    .expect("write config");
    Mock {
        _dir: dir,
        config,
        log,
    }
}

fn run_dsc(args: &[&str], config: &Path) -> (String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .args(args)
        .env("DSC_CONFIG", config)
        .output()
        .expect("running dsc");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        output.status.success(),
    )
}

impl Mock {
    fn run(&self, args: &[&str]) -> (String, bool) {
        run_dsc(args, &self.config)
    }

    fn requests(&self) -> Vec<String> {
        self.log.lock().expect("mock log poisoned").clone()
    }

    fn list_reads(&self) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.starts_with("GET /categories.json"))
            .count()
    }

    /// Per-category show reads, in request order.
    fn show_reads(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter(|r| r.starts_with("GET /c/"))
            .collect()
    }

    fn writes(&self) -> usize {
        self.requests()
            .iter()
            .filter(|r| !r.starts_with("GET "))
            .count()
    }
}

#[test]
fn show_by_id_reads_only_the_selected_category() {
    let mock = start_mock();
    let (output, ok) = mock.run(&["category", "show", "mock", "5", "--format", "json"]);
    assert!(ok, "category show failed: {output}");
    assert!(
        output.contains("\"docs_index_id\": \"500\""),
        "custom fields should come from /c/5/show.json: {output}"
    );
    assert_eq!(mock.list_reads(), 1);
    assert_eq!(mock.show_reads(), vec!["GET /c/5/show.json".to_string()]);
}

#[test]
fn show_by_slug_and_by_unique_name_read_only_the_selected_category() {
    for selector in ["docs-archive", "Docs"] {
        let mock = start_mock();
        let (output, ok) = mock.run(&["category", "show", "mock", selector]);
        assert!(ok, "category show {selector} failed: {output}");
        let expected = if selector == "Docs" { 4 } else { 6 };
        assert_eq!(mock.list_reads(), 1, "{selector}");
        assert_eq!(
            mock.show_reads(),
            vec![format!("GET /c/{expected}/show.json")],
            "{selector}"
        );
    }
}

#[test]
fn show_child_keeps_parent_slug_from_the_listing() {
    let mock = start_mock();
    let (output, ok) = mock.run(&["category", "show", "mock", "6", "--format", "json"]);
    assert!(ok, "category show failed: {output}");
    assert!(
        output.contains("\"parent\": \"docs\""),
        "parent slug should still resolve from the full listing: {output}"
    );
    assert_eq!(mock.show_reads().len(), 1);
}

#[test]
fn unknown_and_ambiguous_targets_fail_before_any_show_read() {
    let mock = start_mock();
    let (output, ok) = mock.run(&["category", "show", "mock", "missing"]);
    assert!(!ok, "unknown category should fail: {output}");
    assert!(output.contains("not found"), "{output}");

    let (output, ok) = mock.run(&["category", "show", "mock", "999"]);
    assert!(!ok, "unknown category id should fail: {output}");

    let (output, ok) = mock.run(&["category", "show", "mock", "Archive"]);
    assert!(!ok, "ambiguous category name should fail: {output}");
    assert!(output.contains("ambiguous"), "{output}");

    assert!(
        mock.show_reads().is_empty(),
        "no per-category read should precede a resolution error: {:?}",
        mock.show_reads()
    );
}

#[test]
fn get_reads_custom_fields_only_when_requested() {
    let mock = start_mock();
    let (output, ok) = mock.run(&["category", "get", "mock", "docs", "description"]);
    assert!(ok, "category get description failed: {output}");
    assert!(mock.show_reads().is_empty(), "{:?}", mock.show_reads());

    let (output, ok) = mock.run(&["category", "get", "mock", "docs", "custom_fields"]);
    assert!(ok, "category get custom_fields failed: {output}");
    assert!(output.contains("400"), "{output}");
    assert_eq!(mock.show_reads(), vec!["GET /c/4/show.json".to_string()]);
}

#[test]
fn set_custom_fields_dry_run_reads_only_the_selected_category() {
    let mock = start_mock();
    let (output, ok) = mock.run(&[
        "category",
        "set",
        "--dry-run",
        "mock",
        "archive",
        "custom_fields",
        r#"{"docs_index_id":"501"}"#,
    ]);
    assert!(ok, "category set --dry-run failed: {output}");
    assert!(output.contains("PUT /categories/5.json"), "{output}");
    assert_eq!(mock.show_reads(), vec!["GET /c/5/show.json".to_string()]);
    assert_eq!(mock.writes(), 0, "dry run must not write");
}

#[test]
fn set_non_custom_field_dry_run_skips_show_reads() {
    let mock = start_mock();
    let (output, ok) = mock.run(&[
        "category",
        "set",
        "--dry-run",
        "mock",
        "archive",
        "description",
        "new",
    ]);
    assert!(ok, "category set --dry-run failed: {output}");
    assert!(mock.show_reads().is_empty(), "{:?}", mock.show_reads());
    assert_eq!(mock.writes(), 0, "dry run must not write");
}

#[test]
fn diff_reads_one_show_per_side() {
    let mock = start_mock();
    let (output, ok) = mock.run(&["category", "diff", "mock", "4", "mock", "5"]);
    assert!(ok, "category diff failed: {output}");
    assert_eq!(mock.list_reads(), 2, "one listing per side");
    assert_eq!(
        mock.show_reads(),
        vec![
            "GET /c/4/show.json".to_string(),
            "GET /c/5/show.json".to_string()
        ]
    );
}

#[test]
fn def_push_reads_custom_fields_only_for_matched_entries_that_declare_them() {
    let mock = start_mock();
    let file_dir = TempDir::new().expect("tempdir");
    let file = file_dir.path().join("categories.yaml");
    std::fs::write(
        &file,
        r#"version: 2
categories:
  - name: Docs
    id: 4
    custom_fields:
      docs_index_id: "400"
  - name: Archive
    id: 5
  - name: Archive
    id: 6
    custom_fields:
      docs_index_id: "600"
  - name: New category
    custom_fields:
      docs_index_id: "new"
"#,
    )
    .expect("write category definitions");

    let (output, ok) = mock.run(&[
        "--dry-run",
        "category",
        "def",
        "push",
        "mock",
        file.to_str().expect("UTF-8 path"),
    ]);
    assert!(ok, "category def push --dry-run failed: {output}");
    assert_eq!(mock.list_reads(), 1);
    assert_eq!(
        mock.show_reads(),
        vec![
            "GET /c/4/show.json".to_string(),
            "GET /c/6/show.json".to_string(),
        ]
    );
    assert_eq!(mock.writes(), 0, "dry run must not write");
}

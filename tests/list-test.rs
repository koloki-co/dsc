// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn list() {
    vprintln("e2e_list: listing discourses");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"local\"\nbaseurl = \"https://example.com\"\n",
    );
    let output = run_dsc(&["list", "-f", "json"], &config_path);
    assert!(output.status.success(), "list failed");
}

#[test]
fn list_filters_by_tags() {
    vprintln("e2e_list_tags: filtering by tags");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "one"
baseurl = "https://one.example"
tags = ["alpha", "beta"]

[[discourse]]
name = "two"
baseurl = "https://two.example"
tags = ["gamma"]

[[discourse]]
name = "three"
baseurl = "https://three.example"
"#,
    );
    let output = run_dsc(
        &["list", "--tags", "alpha;gamma", "-f", "json"],
        &config_path,
    );
    assert!(output.status.success(), "list with tags failed");
    let raw = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&raw).expect("parse json");
    let names: Vec<String> = value
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|item| {
            item.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    assert!(
        names.contains(&"one".to_string()),
        "missing 'one' in {names:?}"
    );
    assert!(
        names.contains(&"two".to_string()),
        "missing 'two' in {names:?}"
    );
    assert!(
        !names.contains(&"three".to_string()),
        "unexpected 'three' in {names:?}"
    );
}

#[test]
fn list_urls_format_and_tags_are_pipe_friendly() {
    vprintln("e2e_list_urls: list base urls line-by-line");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "one"
baseurl = "https://one.example"
tags = ["alpha", "beta"]

[[discourse]]
name = "two"
baseurl = "https://two.example"
tags = ["gamma"]

[[discourse]]
name = "three"
baseurl = "https://three.example"
"#,
    );
    let output = run_dsc(&["list", "--tags", "gamma", "-f", "urls"], &config_path);
    assert!(output.status.success(), "list urls with tags failed");
    let raw = String::from_utf8_lossy(&output.stdout);
    assert_eq!(raw.trim(), "https://two.example");
}

#[test]
fn list_open_uses_tag_filter() {
    vprintln("e2e_list_open: open only matching tag urls");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "one"
baseurl = "https://one.example"
tags = ["alpha"]

[[discourse]]
name = "two"
baseurl = "https://two.example"
tags = ["gamma"]
"#,
    );
    let output = run_dsc_env(
        &["list", "--tags", "gamma", "--open", "-f", "urls"],
        &config_path,
        &[("DSC_BROWSER_OPENER", "true")],
    );
    assert!(output.status.success(), "list --open with tags failed");
    let raw = String::from_utf8_lossy(&output.stdout);
    assert_eq!(raw.trim(), "https://two.example");
}

#[test]
fn list_tidy_sorts_inserts_placeholders_and_reports_missing() {
    vprintln("e2e_list_tidy: sorting config and inserting placeholders");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "b"
baseurl = ""

[[discourse]]
name = "a"
baseurl = "https://a.example"
apikey = "abc"
api_username = "user"
tags = ["t1"]
changelog_topic_id = 123
ssh_host = "a-host"
"#,
    );

    let output = run_dsc(&["list", "tidy"], &config_path);
    assert!(output.status.success(), "list tidy failed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "b: missing baseurl, apikey, api_username, tags, ssh_host, changelog_topic_id"
        ),
        "unexpected missing-fields report: {stdout}"
    );
    assert!(
        !stdout.contains("a: missing"),
        "did not expect discourse 'a' to be reported missing: {stdout}"
    );

    let raw = fs::read_to_string(&config_path).expect("read rewritten config");

    // Sorted by name: 'a' entry should appear before 'b'.
    let pos_a = raw.find("name = \"a\"").expect("missing a entry");
    let pos_b = raw.find("name = \"b\"").expect("missing b entry");
    assert!(pos_a < pos_b, "config not sorted by name:\n{raw}");

    // Placeholder keys inserted for the discourse with missing fields.
    assert!(
        raw.contains("apikey = \"\""),
        "missing apikey placeholder:\n{raw}"
    );
    assert!(
        raw.contains("api_username = \"\""),
        "missing api_username placeholder:\n{raw}"
    );
    assert!(
        raw.contains("tags = []"),
        "missing tags placeholder:\n{raw}"
    );
    assert!(
        raw.contains("changelog_topic_id = 0"),
        "missing changelog_topic_id placeholder:\n{raw}"
    );
    assert!(
        raw.contains("ssh_host = \"\""),
        "missing ssh_host placeholder:\n{raw}"
    );

    // Existing non-empty fields remain as-is.
    assert!(
        raw.contains("tags = [\"t1\"]"),
        "tags overwritten unexpectedly:\n{raw}"
    );
    assert!(
        raw.contains("changelog_topic_id = 123"),
        "changelog_topic_id overwritten unexpectedly:\n{raw}"
    );
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::process::Command;
use tempfile::TempDir;

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

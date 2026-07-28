// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::fs;
use std::path::Path;

const VALID_CONFIG: &str = r#"
version = 1

[[discourse]]
name = "demo"
baseurl = "https://demo.example.com"
apikey = "secret"
api_username = "system"
disposable = true
test_topic_id = 1
test_category_id = 2
test_color_scheme_id = 3
test_group_id = 4
test_theme_id = 5
"#;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn live_configuration_is_valid() {
    if std::env::var("DSC_LIVE_TEST_PHASE").as_deref() != Ok("preflight") {
        return;
    }
    let discourse = test_discourse().expect("live-test config has a forum");
    validate_live_forum(&discourse).expect("validate live-test forum settings");
    let stale = cleanup_live_resources(&discourse).expect("clean stale live-test resources");
    if !stale.is_empty() {
        eprintln!(
            "[live] removed stale resources from an interrupted run: {}",
            stale.join(", ")
        );
    }
    eprintln!(
        "[live] validated disposable forum {} ({}) for run {}",
        discourse.name,
        discourse.baseurl,
        live_test_run_id()
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn live_cleanup_is_complete() {
    if std::env::var("DSC_LIVE_TEST_PHASE").as_deref() != Ok("postflight") {
        return;
    }
    let discourse = test_discourse().expect("live-test config has a forum");
    let leaked = cleanup_live_resources(&discourse).expect("clean leaked live-test resources");
    assert!(
        leaked.is_empty(),
        "live tests leaked resources (removed during postflight): {}",
        leaked.join(", ")
    );
}

#[test]
fn every_test_using_the_live_config_is_explicitly_ignored() {
    let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    for entry in fs::read_dir(&tests_dir).expect("read tests directory") {
        let path = entry.expect("test entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("rs")
            || path.file_name().and_then(|value| value.to_str()) == Some("live-harness-test.rs")
        {
            continue;
        }
        let raw = fs::read_to_string(&path).expect("read integration test");
        for block in raw.split("#[test]").skip(1) {
            let body = block.split("#[test]").next().unwrap_or(block);
            if body.contains("test_discourse()") {
                assert!(
                    body.trim_start().starts_with("#[ignore"),
                    "{} has a live test without #[ignore]",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn live_config_requires_version_and_disposable_marker() {
    assert!(validate_test_config_toml(VALID_CONFIG).is_ok());

    let missing_version = VALID_CONFIG.replacen("version = 1", "", 1);
    assert_eq!(
        validate_test_config_toml(&missing_version).unwrap_err(),
        "live-test config must set version = 1"
    );

    let not_disposable = VALID_CONFIG.replacen("disposable = true", "disposable = false", 1);
    assert!(
        validate_test_config_toml(&not_disposable)
            .unwrap_err()
            .contains("must set disposable = true")
    );
}

#[test]
fn live_config_requires_every_core_fixture() {
    for field in [
        "test_topic_id",
        "test_category_id",
        "test_color_scheme_id",
        "test_group_id",
        "test_theme_id",
    ] {
        let line = VALID_CONFIG
            .lines()
            .find(|line| line.starts_with(field))
            .expect("fixture line");
        let incomplete = VALID_CONFIG.replacen(line, "", 1);
        assert!(
            validate_test_config_toml(&incomplete)
                .unwrap_err()
                .contains(field),
            "missing {field} was accepted"
        );
    }
}

#[cfg(unix)]
#[test]
fn live_runner_never_steals_an_ownerless_lock() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let dir = tempfile::TempDir::new().expect("tempdir");
    let config_path = dir.path().join("test-dsc.toml");
    fs::write(&config_path, "placeholder").expect("write config");
    let config_path = fs::canonicalize(config_path).expect("canonical config path");
    let runtime_dir = dir.path().join("runtime");
    fs::create_dir(&runtime_dir).expect("create runtime directory");

    let mut cksum = Command::new("cksum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn cksum");
    cksum
        .stdin
        .as_mut()
        .expect("cksum stdin")
        .write_all(config_path.to_string_lossy().as_bytes())
        .expect("write cksum input");
    let cksum_output = cksum.wait_with_output().expect("read cksum output");
    assert!(cksum_output.status.success());
    let lock_id = String::from_utf8(cksum_output.stdout)
        .expect("cksum output is UTF-8")
        .split_whitespace()
        .next()
        .expect("cksum value")
        .to_string();
    let uid_output = Command::new("id").arg("-u").output().expect("read user ID");
    assert!(uid_output.status.success());
    let uid = String::from_utf8(uid_output.stdout)
        .expect("user ID is UTF-8")
        .trim()
        .to_string();
    let state_root = runtime_dir.join(format!("dsc-live-tests-{uid}"));
    fs::create_dir(&state_root).expect("create live-test state directory");
    let lock_dir = state_root.join(format!("{lock_id}.lock"));
    fs::create_dir(&lock_dir).expect("create ownerless lock");

    let output = Command::new("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("s/test-live"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("DSC_LIVE_TESTS", "1")
        .env("TEST_DSC_CONFIG", &config_path)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .output()
        .expect("run live-test script");

    assert_eq!(output.status.code(), Some(2));
    assert!(lock_dir.is_dir(), "runner removed an unowned lock");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to steal the lock"),
        "unexpected error: {stderr}"
    );
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use tempfile::TempDir;

fn make_config(dir: &TempDir, test: &TestDiscourse) -> std::path::PathBuf {
    write_temp_config(
        dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    )
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn setting_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_setting_list: listing site settings");
    let dir = TempDir::new().expect("tempdir");
    let config_path = make_config(&dir, &test);
    let output = run_dsc(&["setting", "list", &test.name], &config_path);
    assert!(
        output.status.success(),
        "setting list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The output should contain at least one "key = value" line.
    assert!(
        stdout.contains(" = "),
        "expected key = value lines in output, got: {}",
        stdout
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn setting_list_json() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_setting_list_json: listing site settings as JSON");
    let dir = TempDir::new().expect("tempdir");
    let config_path = make_config(&dir, &test);
    let output = run_dsc(
        &["setting", "list", &test.name, "--format", "json"],
        &config_path,
    );
    assert!(
        output.status.success(),
        "setting list --format json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("setting list --format json did not produce valid JSON");
    assert!(parsed.is_array(), "expected JSON array, got: {}", stdout);
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn setting_get() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_setting_get: fetching a known site setting");
    let dir = TempDir::new().expect("tempdir");
    let config_path = make_config(&dir, &test);
    // title is a standard Discourse site setting that always exists.
    let output = run_dsc(&["setting", "get", &test.name, "title"], &config_path);
    assert!(
        output.status.success(),
        "setting get title failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "expected non-empty value for 'title' setting"
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn setting_get_json() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_setting_get_json: --format json emits a structured object");
    let dir = TempDir::new().expect("tempdir");
    let config_path = make_config(&dir, &test);
    let output = run_dsc(
        &["setting", "get", &test.name, "title", "--format", "json"],
        &config_path,
    );
    assert!(
        output.status.success(),
        "setting get --format json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The object shape must be present regardless of the live value.
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("setting get --format json should emit JSON");
    assert_eq!(
        parsed.get("setting").and_then(|v| v.as_str()),
        Some("title")
    );
    assert!(parsed.get("value").is_some(), "expected a value key");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn setting_get_missing() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_setting_get_missing: fetching a non-existent setting should fail");
    let dir = TempDir::new().expect("tempdir");
    let config_path = make_config(&dir, &test);
    let output = run_dsc(
        &[
            "setting",
            "get",
            &test.name,
            "this_setting_does_not_exist_xyzzy",
        ],
        &config_path,
    );
    assert!(
        !output.status.success(),
        "expected failure for missing setting, but it succeeded"
    );
}

#[test]
fn setting_set_dry_run() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"offline\"\nbaseurl = \"https://example.invalid\"\napikey = \"unused\"\napi_username = \"system\"\n",
    );
    let set_output = run_dsc(
        &[
            "-n",
            "setting",
            "set",
            "offline",
            "short_site_description",
            "dsc-live-dry-run-marker",
        ],
        &config_path,
    );
    assert!(
        set_output.status.success(),
        "setting set --dry-run failed: {}",
        String::from_utf8_lossy(&set_output.stderr)
    );
    let stdout = String::from_utf8_lossy(&set_output.stdout);
    assert!(
        stdout.contains("[dry-run]") && stdout.contains("would set"),
        "expected dry-run setting plan, got: {}",
        stdout
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn setting_audit_json() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_setting_audit_json: audit a setting across configured forums");
    let dir = TempDir::new().expect("tempdir");
    let config_path = make_config(&dir, &test);
    // No discourse positional: audit runs across every configured forum (here, one).
    let output = run_dsc(
        &["setting", "audit", "title", "--format", "json"],
        &config_path,
    );
    assert!(
        output.status.success(),
        "setting audit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("setting audit --format json should emit JSON");
    let rows = parsed
        .as_array()
        .expect("audit output should be a JSON array");
    assert_eq!(
        rows.len(),
        1,
        "expected one row for the single configured forum"
    );
    assert_eq!(
        rows[0].get("discourse").and_then(|v| v.as_str()),
        Some(test.name.as_str())
    );
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use tempfile::TempDir;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn theme_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_theme_list: listing themes");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["theme", "list", &test.name], &config_path);
    assert!(output.status.success(), "theme list failed");
}

#[test]
fn theme_install_dry_run() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"offline\"\nbaseurl = \"https://forum.example.invalid\"\napikey = \"fake-api-key\"\napi_username = \"fake-admin\"\n",
    );

    let output = run_dsc(
        &[
            "-n",
            "theme",
            "install",
            "offline",
            "https://github.com/discourse/discourse-brand-header",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "theme install --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]") && stdout.contains("would import theme"),
        "expected dry-run theme install plan, got: {stdout}"
    );
    assert!(
        !stdout.contains(": installed") && !stdout.contains("theme import completed"),
        "dry-run must not report a completed mutation, got: {stdout}"
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn theme_pull_push_dry_run() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(theme_id) = test.test_theme_id else {
        return;
    };
    vprintln("e2e_theme_pull_push_dry_run: pull theme then preview push");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );

    // Pull the theme to a file
    let json_path = dir.path().join("pulled-theme.json");
    let output = run_dsc(
        &[
            "theme",
            "pull",
            &test.name,
            &theme_id.to_string(),
            json_path.to_str().unwrap(),
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "theme pull failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(json_path.exists(), "pulled theme file not created");

    let raw = std::fs::read_to_string(&json_path).expect("read pulled theme");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse pulled theme");
    assert!(
        parsed.get("name").is_some(),
        "pulled theme JSON missing 'name'"
    );

    // Preview pushing back to the same theme ID without updating it.
    let output = run_dsc(
        &[
            "-n",
            "theme",
            "push",
            &test.name,
            json_path.to_str().unwrap(),
            &theme_id.to_string(),
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "theme push --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]") && stdout.contains("would update theme"),
        "expected dry-run theme push plan, got: {stdout}"
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn theme_show() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(theme_id) = test.test_theme_id else {
        return;
    };
    vprintln("e2e_theme_show: show a theme's detail (json)");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(
        &[
            "theme",
            "show",
            &test.name,
            &theme_id.to_string(),
            "--format",
            "json",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "theme show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("theme show should emit JSON");
    assert_eq!(
        parsed.get("id").and_then(|v| v.as_u64()),
        Some(theme_id),
        "theme show id should match requested theme"
    );
    assert!(
        parsed.get("settings_count").is_some(),
        "missing settings_count"
    );
    assert!(parsed.get("fields").is_some(), "missing fields inventory");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn theme_setting_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(theme_id) = test.test_theme_id else {
        return;
    };
    vprintln("e2e_theme_setting_list: list a theme's settings");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    // JSON format so the output is parseable regardless of how many settings
    // the theme has (a theme with no settings is still valid).
    let output = run_dsc(
        &[
            "theme",
            "setting",
            "list",
            &test.name,
            &theme_id.to_string(),
            "--format",
            "json",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "theme setting list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("theme setting list should emit JSON array");
    assert!(parsed.is_array(), "expected a JSON array of settings");
}

#[test]
fn theme_setting_set_dry_run() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"offline\"\nbaseurl = \"https://forum.example.invalid\"\napikey = \"fake-api-key\"\napi_username = \"fake-admin\"\n",
    );
    let output = run_dsc(
        &[
            "-n",
            "theme",
            "setting",
            "set",
            "offline",
            "1234",
            "example_setting",
            "probe-value",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "theme setting set --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]"),
        "expected dry-run notice, got: {stdout}"
    );
}

#[test]
fn theme_enable_disable_dry_run() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"offline\"\nbaseurl = \"https://forum.example.invalid\"\napikey = \"fake-api-key\"\napi_username = \"fake-admin\"\n",
    );
    for verb in ["enable", "disable"] {
        let output = run_dsc(&["-n", "theme", verb, "offline", "1234"], &config_path);
        assert!(
            output.status.success(),
            "theme {verb} --dry-run failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("[dry-run]") && stdout.contains(verb),
            "expected dry-run {verb} notice, got: {stdout}"
        );
    }
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn theme_detach_unattached_is_noop_dry_run() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(theme_id) = test.test_theme_id else {
        return;
    };
    vprintln("e2e_theme_detach_unattached: dry-run detaching an unattached id");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(
        &[
            "-n",
            "theme",
            "detach",
            &test.name,
            &theme_id.to_string(),
            "999999999",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "theme detach no-op failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("not attached"),
        "expected 'not attached' no-op message, got: {stdout}"
    );
}

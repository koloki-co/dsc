// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use tempfile::TempDir;

#[test]
fn notification_list_returns_a_json_array_without_mutating_the_forum() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_notification_list: fetch one liked notification as JSON");
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
            "notification",
            "list",
            &test.name,
            "--type",
            "liked",
            "--limit",
            "1",
            "--format",
            "json",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "notification list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("notification list --format json must emit JSON");
    let entries = parsed.as_array().expect("expected JSON array");
    assert!(
        entries.iter().all(|entry| entry
            .get("notification_type")
            .and_then(|value| value.as_u64())
            == Some(5)),
        "--type liked returned a non-liked notification: {stdout}"
    );
}

#[test]
fn notification_read_dry_run_does_not_mutate_the_forum() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_notification_read: dry-run mark-all-read never sends the request");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );

    let output = run_dsc(
        &["--dry-run", "notification", "read", &test.name, "--all"],
        &config_path,
    );
    assert!(
        output.status.success(),
        "notification read --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]") && stdout.contains("mark all unread notifications read"),
        "expected dry-run preview, got: {stdout}"
    );
}

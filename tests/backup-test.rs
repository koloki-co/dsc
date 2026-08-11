// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use tempfile::TempDir;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn backup_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    if test.backup_enabled != Some(true) {
        eprintln!("[live:skip] backup_list requires backup_enabled = true");
        return;
    }
    vprintln("e2e_backup_list: listing backups");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["backup", "list", &test.name], &config_path);
    assert!(output.status.success(), "backup list failed");
}

#[test]
fn backup_health_reports_unreachable_forum_in_structured_output() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "offline-health"
baseurl = "not-a-url"
apikey = "secret"
api_username = "system"
"#,
    );
    let output = run_dsc(
        &["backup", "health", "offline-health", "--format", "json"],
        &config_path,
    );
    assert!(
        !output.status.success(),
        "unreachable forum must be unhealthy"
    );
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).expect("health JSON");
    assert_eq!(rows[0]["discourse"], "offline-health");
    assert_eq!(rows[0]["status"], "unknown");
}

#[test]
fn setup_s3_dry_run_use_iam_profile_skips_user_and_keys() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "forum"
baseurl = "https://forum.example"
apikey = "secret"
api_username = "system"
"#,
    );
    let output = run_dsc(
        &[
            "backup",
            "setup-s3",
            "forum",
            "--dry-run",
            "--use-iam-profile",
        ],
        &config_path,
    );
    assert!(output.status.success(), "dry-run must not fail offline");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("s3_use_iam_profile   = true"));
    assert!(!stdout.contains("create-access-key"));
    assert!(!stdout.contains("create-user"));
    assert!(!stdout.contains("s3_access_key_id"));
}

#[test]
fn setup_s3_dry_run_default_still_plans_dedicated_user() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "forum"
baseurl = "https://forum.example"
apikey = "secret"
api_username = "system"
"#,
    );
    let output = run_dsc(&["backup", "setup-s3", "forum", "--dry-run"], &config_path);
    assert!(output.status.success(), "dry-run must not fail offline");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("aws iam create-access-key --user-name forum-discourse-backup-user"));
    assert!(stdout.contains("s3_access_key_id     = <minted at run time>"));
}

#[test]
fn backup_health_rejects_discourse_and_tags_together() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "forum"
baseurl = "https://forum.example"
apikey = "secret"
api_username = "system"
tags = ["production"]
"#,
    );
    let output = run_dsc(
        &["backup", "health", "forum", "--tags", "production"],
        &config_path,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("cannot pass <discourse> together with --tags")
    );
}

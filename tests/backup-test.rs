// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use tempfile::TempDir;

/// A mock Discourse that answers every request with a fixed status, ignoring
/// path and method - `backup create`'s only interest is the response status.
fn start_mock(status: u16) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, status);
        }
    });
    format!("http://{addr}")
}

fn handle(mut stream: TcpStream, status: u16) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
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
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }

    let (status_line, body) = if status == 200 {
        ("HTTP/1.1 200 OK", "{}".to_string())
    } else {
        ("HTTP/1.1 503 Service Unavailable", "{}".to_string())
    };
    let response = format!(
        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

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
    assert!(stdout.contains("s3_use_iam_profile   = false"));
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

#[test]
fn backup_create_all_fans_out_to_every_configured_forum() {
    let alpha_url = start_mock(200);
    let beta_url = start_mock(200);

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n\n[[discourse]]\nname = \"beta\"\nbaseurl = \"{beta_url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );

    let output = run_dsc(&["backup", "create", "--all"], &config_path);
    assert!(
        output.status.success(),
        "backup create --all failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("alpha: backup requested"));
    assert!(stdout.contains("beta: backup requested"));
}

#[test]
fn backup_create_all_reports_per_forum_failures_without_stopping_the_fleet() {
    let alpha_url = start_mock(200);
    let down_url = start_mock(503);

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"alpha\"\nbaseurl = \"{alpha_url}\"\napikey = \"k\"\napi_username = \"tester\"\n\n[[discourse]]\nname = \"down\"\nbaseurl = \"{down_url}\"\napikey = \"k\"\napi_username = \"tester\"\n"
        ),
    );

    let output = run_dsc(&["backup", "create", "--all"], &config_path);
    assert!(
        !output.status.success(),
        "backup create --all should fail overall when one forum errors"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("alpha: backup requested"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("down: backup failed"));
}

#[test]
fn backup_create_all_requires_configured_discourses() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(&dir, "");
    let output = run_dsc(&["backup", "create", "--all"], &config_path);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no discourses configured"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn setup_s3_all_dry_run_fans_out_to_every_configured_forum() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "alpha"
baseurl = "https://alpha.example"
apikey = "secret"
api_username = "system"

[[discourse]]
name = "beta"
baseurl = "https://beta.example"
apikey = "secret"
api_username = "system"
"#,
    );
    let output = run_dsc(&["backup", "setup-s3", "--all", "--dry-run"], &config_path);
    assert!(
        output.status.success(),
        "dry-run fan-out must not fail offline: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("S3 backup setup for alpha"));
    assert!(stdout.contains("S3 backup setup for beta"));
}

#[test]
fn setup_s3_all_dry_run_respects_tags_filter() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "alpha"
baseurl = "https://alpha.example"
apikey = "secret"
api_username = "system"
tags = ["production"]

[[discourse]]
name = "beta"
baseurl = "https://beta.example"
apikey = "secret"
api_username = "system"
tags = ["staging"]
"#,
    );
    let output = run_dsc(
        &["backup", "setup-s3", "--tags", "production", "--dry-run"],
        &config_path,
    );
    assert!(
        output.status.success(),
        "dry-run fan-out must not fail offline: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("S3 backup setup for alpha"));
    assert!(!stdout.contains("S3 backup setup for beta"));
}

#[test]
fn setup_s3_all_rejects_an_empty_tags_filter() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "alpha"
baseurl = "https://alpha.example"
apikey = "secret"
api_username = "system"
"#,
    );
    let output = run_dsc(
        &["backup", "setup-s3", "--tags", ",;", "--dry-run"],
        &config_path,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--tags must include at least one non-empty tag"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn setup_s3_all_continues_after_a_forum_failure() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "missing-credentials"
baseurl = "https://missing.example"

[[discourse]]
name = "configured"
baseurl = "https://configured.example"
apikey = "secret"
api_username = "system"
"#,
    );
    let output = run_dsc(&["backup", "setup-s3", "--all", "--dry-run"], &config_path);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("missing-credentials: setup-s3 failed"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("S3 backup setup for configured"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn setup_s3_all_requires_configured_discourses() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(&dir, "");
    let output = run_dsc(&["backup", "setup-s3", "--all", "--dry-run"], &config_path);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no discourses configured"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn setup_s3_rejects_bucket_override_with_all() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        r#"[[discourse]]
name = "alpha"
baseurl = "https://alpha.example"
apikey = "secret"
api_username = "system"
"#,
    );
    let output = run_dsc(
        &[
            "backup",
            "setup-s3",
            "--all",
            "--bucket",
            "shared-bucket",
            "--dry-run",
        ],
        &config_path,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot be used with"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn setup_s3_requires_discourse_or_all_or_tags() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(&dir, "");
    let output = run_dsc(&["backup", "setup-s3", "--dry-run"], &config_path);
    assert!(!output.status.success());
}

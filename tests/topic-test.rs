// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::fs::{self, FileTimes, OpenOptions};
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn topic_pull() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    vprintln("e2e_topic_pull: pull topic");

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
            "topic",
            "pull",
            &test.name,
            &topic_id.to_string(),
            dir.path().to_str().unwrap(),
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic pull failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn topic_push() {
    let Some(test) = test_discourse() else {
        return;
    };
    let disposable = create_disposable_topic(&test, "topic-push");
    vprintln("e2e_topic_push: write file, then push disposable topic");
    let dir = TempDir::new().expect("tempdir");
    let file_path = dir.path().join("push.md");
    let body = format!("# E2E Push\n\n{}", disposable.marker);
    fs::write(&file_path, &body).expect("write file");

    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(
        &[
            "topic",
            "push",
            &test.name,
            &disposable.id.to_string(),
            file_path.to_str().unwrap(),
            "--no-bump",
            "--skip-revision",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic push failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let remote = disposable
        .client
        .fetch_topic(disposable.id, true)
        .expect("topic");
    let remote_body = remote
        .post_stream
        .posts
        .first()
        .and_then(|post| post.raw.as_deref())
        .expect("topic OP has raw content");
    assert_eq!(remote_body, body, "marker body not applied after push");
    if std::env::var("DSC_LIVE_TEST_FORCE_FAILURE").as_deref() == Ok("topic_push") {
        panic!("forced failure after topic creation for cleanup verification");
    }
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn topic_sync() {
    let Some(test) = test_discourse() else {
        return;
    };
    let disposable = create_disposable_topic(&test, "topic-sync");
    vprintln("e2e_topic_sync: write file, then sync disposable topic");
    let dir = TempDir::new().expect("tempdir");
    let file_path = dir.path().join("sync.md");
    let body = format!("# E2E Sync\n\n{}", disposable.marker);
    fs::write(&file_path, &body).expect("write file");
    OpenOptions::new()
        .write(true)
        .open(&file_path)
        .expect("open sync file")
        .set_times(
            FileTimes::new()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(4_102_444_800)),
        )
        .expect("set deterministic newer mtime");

    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(
        &[
            "topic",
            "sync",
            &test.name,
            &disposable.id.to_string(),
            file_path.to_str().unwrap(),
            "--yes",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic sync failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let remote = disposable
        .client
        .fetch_topic(disposable.id, true)
        .expect("topic");
    let remote_body = remote
        .post_stream
        .posts
        .first()
        .and_then(|post| post.raw.as_deref())
        .expect("topic OP has raw content");
    assert_eq!(remote_body, body, "marker body not applied after sync");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn topic_title_roundtrip() {
    let Some(test) = test_discourse() else {
        return;
    };
    let disposable = create_disposable_topic(&test, "topic-title-roundtrip");
    vprintln("e2e_topic_title_roundtrip: rename disposable topic and verify");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let title = format!("DSC E2E Title {}", disposable.marker);
    let output = run_dsc(
        &[
            "topic",
            "title",
            &test.name,
            &disposable.id.to_string(),
            &title,
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic title failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let now = disposable
        .client
        .fetch_topic(disposable.id, false)
        .expect("re-fetch topic")
        .title
        .unwrap_or_default();
    assert_eq!(now, title, "marker title was not applied");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn topic_deleted_list_and_restore() {
    let Some(test) = test_discourse() else {
        return;
    };
    let disposable = create_disposable_topic(&test, "topic-deleted-list");
    vprintln("e2e_topic_deleted_list_and_restore: delete, list, and restore disposable topic");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );

    let deleted = run_dsc(
        &["topic", "delete", &test.name, &disposable.id.to_string()],
        &config_path,
    );
    assert!(
        deleted.status.success(),
        "topic delete failed: {}",
        String::from_utf8_lossy(&deleted.stderr)
    );

    let listed = run_dsc(
        &[
            "topic",
            "list",
            &test.name,
            "--deleted",
            &disposable.marker,
            "--format",
            "json",
        ],
        &config_path,
    );
    assert!(
        listed.status.success(),
        "deleted-topic list failed: {}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let topics: serde_json::Value =
        serde_json::from_slice(&listed.stdout).expect("deleted-topic list JSON");
    assert!(
        topics
            .as_array()
            .expect("deleted-topic list array")
            .iter()
            .any(|topic| topic["id"].as_u64() == Some(disposable.id)),
        "deleted topic was not listed"
    );

    let restored = run_dsc(
        &["topic", "restore", &test.name, &disposable.id.to_string()],
        &config_path,
    );
    assert!(
        restored.status.success(),
        "topic restore failed: {}",
        String::from_utf8_lossy(&restored.stderr)
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn topic_tags_dry_run() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    vprintln("e2e_topic_tags_dry_run: dry-run set tags must not write");
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
            "topic",
            "tags",
            &test.name,
            &topic_id.to_string(),
            "dsc-e2e-probe",
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic tags --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]") && stdout.contains("would set tags"),
        "expected dry-run tags notice, got: {stdout}"
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn topic_change_owner_dry_run() {
    let Some(test) = test_discourse() else {
        return;
    };
    let disposable = create_disposable_topic(&test, "topic-change-owner-dry-run");
    vprintln("e2e_topic_change_owner_dry_run: dry-run change-owner must not write");
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
            "topic",
            "change-owner",
            &test.name,
            &disposable.id.to_string(),
            &test.api_username,
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic change-owner --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]") && stdout.contains("would reassign"),
        "expected dry-run change-owner notice, got: {stdout}"
    );
}

#[test]
fn topic_reply_dry_run_previews_without_posting() {
    let topic_id = 12345;
    vprintln("topic_reply_dry_run: -n must return before network access (issue #20)");
    let dir = TempDir::new().expect("tempdir");
    let file_path = dir.path().join("reply.md");
    fs::write(&file_path, "A dry-run reply that must NOT be posted.").expect("write file");
    let config_path = write_temp_config(
        &dir,
        "[[discourse]]\nname = \"offline\"\nbaseurl = \"https://example.invalid\"\napikey = \"unused\"\napi_username = \"system\"\n",
    );

    let output = run_dsc(
        &[
            "-n",
            "topic",
            "reply",
            "offline",
            &topic_id.to_string(),
            file_path.to_str().unwrap(),
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic reply --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]") && stdout.contains("would reply"),
        "expected a marked dry-run preview, got: {stdout}"
    );
    assert!(
        !stdout.contains("Replied to topic"),
        "dry-run must not print a success line, got: {stdout}"
    );
}

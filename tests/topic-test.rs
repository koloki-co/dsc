// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use dsc::api::DiscourseClient;
use std::fs;
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn topic_pull() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    let marker = Uuid::new_v4().to_string();
    vprintln("e2e_topic_pull: post marker, then pull topic");
    post_and_verify(&test, topic_id, &marker);

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
    assert!(output.status.success(), "topic pull failed");
}

#[test]
fn topic_push() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    let marker = Uuid::new_v4().to_string();
    vprintln("e2e_topic_push: write file, then push topic");
    let dir = TempDir::new().expect("tempdir");
    let file_path = dir.path().join("push.md");
    fs::write(&file_path, format!("# E2E Push\n\n{}", marker)).expect("write file");

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
            &topic_id.to_string(),
            file_path.to_str().unwrap(),
        ],
        &config_path,
    );
    assert!(output.status.success(), "topic push failed");
    let config = to_config(&test);
    let client = DiscourseClient::new(&config).expect("client");
    let topic = client.fetch_topic(topic_id, true).expect("topic");
    let found = topic.post_stream.posts.iter().any(|post| {
        post.raw
            .as_ref()
            .map(|raw| raw.contains(&marker))
            .unwrap_or(false)
    });
    assert!(found, "marker not found after push");
}

#[test]
fn topic_sync() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    let marker = Uuid::new_v4().to_string();
    vprintln("e2e_topic_sync: write file, then sync");
    let dir = TempDir::new().expect("tempdir");
    let file_path = dir.path().join("sync.md");
    fs::write(&file_path, format!("# E2E Sync\n\n{}", marker)).expect("write file");

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
            &topic_id.to_string(),
            file_path.to_str().unwrap(),
            "--yes",
        ],
        &config_path,
    );
    assert!(output.status.success(), "topic sync failed");
    let config = to_config(&test);
    let client = DiscourseClient::new(&config).expect("client");
    let topic = client.fetch_topic(topic_id, true).expect("topic");
    let found = topic.post_stream.posts.iter().any(|post| {
        post.raw
            .as_ref()
            .map(|raw| raw.contains(&marker))
            .unwrap_or(false)
    });
    assert!(found, "marker not found after sync");
}

#[test]
fn topic_title_roundtrip() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    vprintln("e2e_topic_title_roundtrip: rename topic, verify, restore");
    let config = to_config(&test);
    let client = DiscourseClient::new(&config).expect("client");
    let original = client
        .fetch_topic(topic_id, false)
        .expect("fetch topic")
        .title
        .expect("topic has a title");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let marker = format!("DSC E2E Title {}", Uuid::new_v4());
    let output = run_dsc(
        &["topic", "title", &test.name, &topic_id.to_string(), &marker],
        &config_path,
    );
    assert!(
        output.status.success(),
        "topic title failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let now = client
        .fetch_topic(topic_id, false)
        .expect("re-fetch topic")
        .title
        .unwrap_or_default();
    assert_eq!(now, marker, "title was not applied");

    // Restore the original title so the test leaves no trace.
    let restore = run_dsc(
        &[
            "topic",
            "title",
            &test.name,
            &topic_id.to_string(),
            &original,
        ],
        &config_path,
    );
    assert!(
        restore.status.success(),
        "restoring original title failed: {}",
        String::from_utf8_lossy(&restore.stderr)
    );
}

#[test]
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
fn topic_reply_dry_run_previews_without_posting() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    vprintln("e2e_topic_reply_dry_run: -n must preview, not post (issue #20)");
    let dir = TempDir::new().expect("tempdir");
    let file_path = dir.path().join("reply.md");
    fs::write(&file_path, "A dry-run reply that must NOT be posted.").expect("write file");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );

    let config = to_config(&test);
    let client = DiscourseClient::new(&config).expect("client");
    let before = client.fetch_topic(topic_id, false).expect("topic");
    let count_before = before.post_stream.stream.len();

    let output = run_dsc(
        &[
            "-n",
            "topic",
            "reply",
            &test.name,
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

    // And it must not actually post.
    let after = client.fetch_topic(topic_id, false).expect("topic");
    assert_eq!(
        after.post_stream.stream.len(),
        count_before,
        "dry-run must not change the post count"
    );
}

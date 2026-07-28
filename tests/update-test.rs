// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use dsc::api::DiscourseClient;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn update() {
    let Some(test) = test_discourse() else {
        return;
    };
    if test.ssh_enabled != Some(true) {
        eprintln!("[live:skip] update requires ssh_enabled = true");
        return;
    }
    vprintln("e2e_update: update + changelog post");
    let Some(topic_id) = test.changelog_topic_id else {
        eprintln!("[live:skip] update requires changelog_topic_id");
        return;
    };
    let marker = live_test_marker("update");
    let client = DiscourseClient::new(&to_config(&test)).expect("client");
    let _cleanup = DeletePostsByMarkerOnDrop::new(&test, topic_id, &marker);
    let ssh_host_line = test
        .ssh_host
        .as_ref()
        .map(|host| format!("ssh_host = \"{}\"\n", host))
        .unwrap_or_default();
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n{}changelog_topic_id = {}\n",
            test.name, test.baseurl, test.apikey, test.api_username, ssh_host_line, topic_id
        ),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .arg("-c")
        .arg(&config_path)
        .arg("update")
        .arg(&test.name)
        .arg("-y")
        .env("DSC_TEST_MARKER", &marker)
        .env("DSC_SSH_UPDATE_CMD", "echo update-ok")
        .env("DSC_SSH_CLEANUP_CMD", "echo Total reclaimed space: 0B")
        .env("DSC_SSH_OS_UPDATE_CMD", "echo OS packages updated")
        .env("DSC_SSH_REBOOT_CMD", "echo Server rebooted")
        .env("DSC_SSH_OS_VERSION_CMD", "echo Ubuntu 22.04.3 LTS")
        .output()
        .expect("run update");
    if !output.status.success() {
        panic!(
            "update failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let post_id = extract_test_post_id(&output).expect("update test should report its post ID");
    let found = wait_for_post_marker(&client, post_id, &marker);
    assert!(found, "marker not found on changelog");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn update_all() {
    let Some(test) = test_discourse() else {
        return;
    };
    if test.ssh_enabled != Some(true) {
        eprintln!("[live:skip] update_all requires ssh_enabled = true");
        return;
    }
    vprintln("e2e_update_all: update all + changelog post");
    let Some(topic_id) = test.changelog_topic_id else {
        eprintln!("[live:skip] update_all requires changelog_topic_id");
        return;
    };
    let marker = live_test_marker("update-all");
    let client = DiscourseClient::new(&to_config(&test)).expect("client");
    let _cleanup = DeletePostsByMarkerOnDrop::new(&test, topic_id, &marker);
    let ssh_host_line = test
        .ssh_host
        .as_ref()
        .map(|host| format!("ssh_host = \"{}\"\n", host))
        .unwrap_or_default();
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n{}changelog_topic_id = {}\n",
            test.name, test.baseurl, test.apikey, test.api_username, ssh_host_line, topic_id
        ),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_dsc"))
        .arg("-c")
        .arg(&config_path)
        .arg("update")
        .arg("all")
        .arg("-y")
        .env("DSC_TEST_MARKER", &marker)
        .env("DSC_SSH_UPDATE_CMD", "echo update-ok")
        .env("DSC_SSH_CLEANUP_CMD", "echo Total reclaimed space: 0B")
        .env("DSC_SSH_OS_UPDATE_CMD", "echo OS packages updated")
        .env("DSC_SSH_REBOOT_CMD", "echo Server rebooted")
        .env("DSC_SSH_OS_VERSION_CMD", "echo Ubuntu 22.04.3 LTS")
        .output()
        .expect("run update all");
    if !output.status.success() {
        panic!(
            "update all failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let post_id = extract_test_post_id(&output).expect("update all test should report its post ID");
    let found = wait_for_post_marker(&client, post_id, &marker);
    assert!(found, "marker not found on changelog");
}

fn wait_for_post_marker(client: &DiscourseClient, post_id: u64, marker: &str) -> bool {
    let max_attempts = 10;
    for _ in 0..max_attempts {
        if let Ok(Some(raw)) = client.fetch_post_raw(post_id)
            && raw.contains(marker)
        {
            return true;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    false
}

fn extract_test_post_id(output: &std::process::Output) -> Option<u64> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(|line| {
        line.strip_prefix("DSC_TEST_POST_ID=")
            .and_then(|value| value.parse::<u64>().ok())
    })
}

#[test]
fn update_all_parallel_empty_config_succeeds() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(&dir, "");
    let output = run_dsc(&["update", "all", "--parallel"], &config_path);
    assert!(
        output.status.success(),
        "update all --parallel should succeed for empty config"
    );
}

#[test]
fn update_all_parallel_zero_is_rejected() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(&dir, "");
    let output = run_dsc(&["update", "all", "--parallel", "0"], &config_path);
    assert!(
        !output.status.success(),
        "update all --parallel 0 should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("width must be at least 1"),
        "unexpected stderr: {stderr}"
    );
}

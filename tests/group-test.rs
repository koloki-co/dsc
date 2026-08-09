// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use tempfile::TempDir;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn group_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_group_list: list groups");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["group", "list", &test.name], &config_path);
    assert!(output.status.success(), "group list failed");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn group_info() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(group_id) = test.test_group_id else {
        return;
    };
    vprintln("e2e_group_info: fetch group info");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(
        &["group", "info", &test.name, &group_id.to_string()],
        &config_path,
    );
    assert!(output.status.success(), "group info failed");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn group_info_with_defaults() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(group_id) = test.test_group_id else {
        return;
    };
    vprintln("e2e_group_info_with_defaults: fetch group info including notification defaults");

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
            "group",
            "info",
            &test.name,
            &group_id.to_string(),
            "--with-defaults",
        ],
        &config_path,
    );
    assert!(output.status.success(), "group info --with-defaults failed");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn group_members() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(group_id) = test.test_group_id else {
        return;
    };
    vprintln("e2e_group_members: fetch group members");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(
        &["group", "members", &test.name, &group_id.to_string()],
        &config_path,
    );
    assert!(output.status.success(), "group members failed");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn group_copy() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(group_id) = test.test_group_id else {
        return;
    };
    vprintln("e2e_group_copy: dry-run copy group on one forum");

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
            "group",
            "copy",
            &test.name,
            &group_id.to_string(),
            "--target",
            &test.name,
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "group copy --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]") && stdout.contains("would create group"),
        "expected dry-run group copy plan, got: {stdout}"
    );
}

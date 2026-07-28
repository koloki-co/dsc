// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use tempfile::TempDir;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn emoji_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_emoji_list: list custom emojis");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["emoji", "list", &test.name], &config_path);
    assert!(output.status.success(), "emoji list failed");
    assert!(!output.stdout.is_empty(), "emoji list produced no output");
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn emoji_list_inline() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_emoji_list_inline: list custom emojis inline");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["emoji", "list", "--inline", &test.name], &config_path);
    assert!(output.status.success(), "emoji list inline failed");
}

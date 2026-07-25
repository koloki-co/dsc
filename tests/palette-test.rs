// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use dsc::api::DiscourseClient;
use std::fs;
use tempfile::TempDir;

#[test]
fn palette_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_palette_list: listing palettes");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["palette", "list", &test.name], &config_path);
    assert!(output.status.success(), "palette list failed");
}

#[test]
fn palette_pull_push() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(palette_id) = test.test_color_scheme_id else {
        return;
    };
    vprintln("e2e_palette_pull_push: pull then push palette");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let palette_path = dir.path().join("palette.json");
    let output = run_dsc(
        &[
            "palette",
            "pull",
            &test.name,
            &palette_id.to_string(),
            palette_path.to_str().unwrap(),
        ],
        &config_path,
    );
    assert!(output.status.success(), "palette pull failed");
    let raw = fs::read_to_string(&palette_path).expect("read palette file");
    assert!(raw.contains("colors"), "palette file missing colors");

    // Push the same palette back to ensure command succeeds without changes.
    let output = run_dsc(
        &[
            "palette",
            "push",
            &test.name,
            palette_path.to_str().unwrap(),
            &palette_id.to_string(),
        ],
        &config_path,
    );
    assert!(output.status.success(), "palette push failed");

    // Verify the palette still exists by fetching via API.
    let client = DiscourseClient::new(&to_config(&test)).expect("client");
    let response = client
        .fetch_color_scheme(palette_id)
        .expect("fetch palette");
    let scheme = response.get("color_scheme").unwrap_or(&response);
    let id = scheme
        .get("id")
        .or_else(|| scheme.get("color_scheme_id"))
        .and_then(|v| v.as_u64())
        .unwrap_or_default();
    assert_eq!(id, palette_id, "palette id mismatch");
}

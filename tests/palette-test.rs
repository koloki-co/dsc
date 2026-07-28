// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::fs;
use tempfile::TempDir;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
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
    let output = run_dsc(
        &["palette", "list", &test.name, "--format", "json"],
        &config_path,
    );
    assert!(output.status.success(), "palette list failed");
    let palettes: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("palette list JSON");
    assert!(
        palettes
            .as_array()
            .expect("palette list array")
            .iter()
            .any(|palette| palette["id"].as_i64().is_some_and(|id| id < 0)),
        "palette list omitted built-in negative IDs"
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn palette_pull() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(palette_id) = test.test_color_scheme_id else {
        return;
    };
    vprintln("e2e_palette_pull: pull palette");
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
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn sar_creates_bundle() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_sar: build a SAR bundle for the API user");
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let out_dir = dir.path().join("sar-out");
    // Run the SAR on the API user themselves (guaranteed to exist).
    let output = run_dsc(
        &[
            "sar",
            &test.name,
            &test.api_username,
            "-o",
            out_dir.to_str().unwrap(),
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "sar failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Core bundle artefacts must exist.
    assert!(out_dir.join("README.md").exists(), "README.md missing");
    assert!(
        out_dir.join("manifest.json").exists(),
        "manifest.json missing"
    );
    assert!(
        out_dir.join("profile.json").exists(),
        "profile.json missing"
    );

    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(out_dir.join("manifest.json")).expect("read manifest"),
    )
    .expect("parse manifest");
    assert_eq!(
        manifest["subject"]["username"].as_str(),
        Some(test.api_username.as_str())
    );
    // Messages were not requested, so the section must be absent and unflagged.
    assert_eq!(manifest["messages_included"], serde_json::json!(false));
    assert!(
        !out_dir.join("messages").exists(),
        "messages/ should be absent without --messages"
    );
}

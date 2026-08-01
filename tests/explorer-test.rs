// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use tempfile::TempDir;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn explorer_lists_and_runs_builtin_query() {
    let Some(test) = test_discourse() else {
        return;
    };
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );

    let list = run_dsc(
        &["explorer", "list", &test.name, "--format", "json"],
        &config_path,
    );
    if !list.status.success() {
        let stderr = String::from_utf8_lossy(&list.stderr);
        if stderr.contains("Data Explorer may be disabled") {
            eprintln!("Data Explorer is disabled on {}; skipping run", test.name);
            return;
        }
    }
    assert!(
        list.status.success(),
        "explorer list failed: {}",
        String::from_utf8_lossy(&list.stderr)
    );
    let catalogue: serde_json::Value =
        serde_json::from_slice(&list.stdout).expect("explorer list JSON");
    assert!(
        catalogue["queries"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );

    let run = run_dsc(
        &[
            "explorer", "run", &test.name, "-1", "--limit", "1", "--format", "json",
        ],
        &config_path,
    );
    assert!(
        run.status.success(),
        "explorer run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&run.stdout).expect("explorer run JSON");
    assert_eq!(result["success"], true);
    assert!(result["columns"].is_array());
    assert!(result["rows"].is_array());
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use std::fs;
use tempfile::TempDir;
use uuid::Uuid;

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn category_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    vprintln("e2e_category_list: list categories");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["category", "list", &test.name], &config_path);
    assert!(
        output.status.success(),
        "category list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn category_copy() {
    let Some(source) = test_discourse() else {
        return;
    };
    let Some(category_id) = source.test_category_id else {
        return;
    };
    vprintln("e2e_category_copy: preview copying a category on the source forum");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            source.name, source.baseurl, source.apikey, source.api_username
        ),
    );
    let output = run_dsc(
        &[
            "-n",
            "category",
            "copy",
            &source.name,
            &category_id.to_string(),
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "category copy --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]") && stdout.contains("would create category"),
        "expected dry-run category creation plan, got: {stdout}"
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn category_pull() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(category_id) = test.test_category_id else {
        return;
    };
    vprintln("e2e_category_pull: pull category");

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
            "category",
            "pull",
            &test.name,
            &category_id.to_string(),
            dir.path().to_str().unwrap(),
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "category pull failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "live compatibility test; run through s/test-live"]
fn category_push() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(category_id) = test.test_category_id else {
        return;
    };
    vprintln("e2e_category_push: preview category push");

    let dir = TempDir::new().expect("tempdir");
    let file_path = dir.path().join("category-push.md");
    let title = format!("E2E Category Push {}", Uuid::new_v4());
    fs::write(
        &file_path,
        format!("# {title}\n\nDry-run category push body."),
    )
    .expect("write file");
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
            "category",
            "push",
            &test.name,
            &category_id.to_string(),
            dir.path().to_str().unwrap(),
        ],
        &config_path,
    );
    assert!(
        output.status.success(),
        "category push --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[dry-run]")
            && stdout.contains("Category push plan")
            && stdout.contains("1 create")
            && stdout.contains(&title),
        "expected dry-run category push plan, got: {stdout}"
    );
}

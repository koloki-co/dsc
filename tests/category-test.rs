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
fn category_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    let marker = Uuid::new_v4().to_string();
    vprintln("e2e_category_list: post marker, then list categories");
    post_and_verify(&test, topic_id, &marker);

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            test.name, test.baseurl, test.apikey, test.api_username
        ),
    );
    let output = run_dsc(&["category", "list", &test.name], &config_path);
    assert!(output.status.success(), "category list failed");
}

#[test]
fn category_copy() {
    let Some(source) = test_discourse() else {
        return;
    };
    let Some(category_id) = source.test_category_id else {
        return;
    };
    let Some(topic_id) = source.test_topic_id else {
        return;
    };
    let marker = Uuid::new_v4().to_string();
    vprintln("e2e_category_copy: post marker, then copy category");
    post_and_verify(&source, topic_id, &marker);

    let source_client = DiscourseClient::new(&to_config(&source)).expect("client");
    let source_categories = source_client.fetch_categories().expect("categories");
    let source_category = source_categories
        .iter()
        .find(|cat| cat.id == Some(category_id))
        .expect("source category");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            source.name, source.baseurl, source.apikey, source.api_username
        ),
    );
    let output = run_dsc(
        &["category", "copy", &source.name, &category_id.to_string()],
        &config_path,
    );
    assert!(output.status.success(), "category copy failed");
    let categories = source_client.fetch_categories().expect("categories");
    let expected_name = format!("Copy of {}", source_category.name);
    let found = categories.iter().any(|cat| cat.name == expected_name);
    assert!(found, "copied category not found");
}

#[test]
fn category_pull() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(category_id) = test.test_category_id else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    let marker = Uuid::new_v4().to_string();
    vprintln("e2e_category_pull: post marker, then pull category");
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
            "category",
            "pull",
            &test.name,
            &category_id.to_string(),
            dir.path().to_str().unwrap(),
        ],
        &config_path,
    );
    assert!(output.status.success(), "category pull failed");
}

#[test]
fn category_push() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(category_id) = test.test_category_id else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    let marker = Uuid::new_v4().to_string();
    vprintln("e2e_category_push: post marker, then push category");
    post_and_verify(&test, topic_id, &marker);

    let dir = TempDir::new().expect("tempdir");
    let file_path = dir.path().join("category-push.md");
    let title = format!("E2E Category Push {}", marker);
    fs::write(&file_path, format!("# {}\n\n{}", title, marker)).expect("write file");
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
            "push",
            &test.name,
            &category_id.to_string(),
            dir.path().to_str().unwrap(),
        ],
        &config_path,
    );
    assert!(output.status.success(), "category push failed");
    let config = to_config(&test);
    let client = DiscourseClient::new(&config).expect("client");
    let category = client.fetch_category(category_id).expect("category");
    let found = category
        .topic_list
        .topics
        .iter()
        .any(|topic| topic.title.contains(&marker));
    assert!(found, "new category topic not found");
}

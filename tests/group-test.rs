// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

mod common;
use common::*;
use dsc::api::{DiscourseClient, GroupDetail};
use dsc::utils::slugify;
use std::collections::BTreeMap;
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn group_list() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    let marker = Uuid::new_v4().to_string();
    vprintln("e2e_group_list: post marker, then list groups");
    post_and_verify(&test, topic_id, &marker);

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
fn group_info() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(group_id) = test.test_group_id else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    let marker = Uuid::new_v4().to_string();
    vprintln("e2e_group_info: post marker, then fetch group info");
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
        &["group", "info", &test.name, &group_id.to_string()],
        &config_path,
    );
    assert!(output.status.success(), "group info failed");
}

#[test]
fn group_members() {
    let Some(test) = test_discourse() else {
        return;
    };
    let Some(group_id) = test.test_group_id else {
        return;
    };
    let Some(topic_id) = test.test_topic_id else {
        return;
    };
    let marker = Uuid::new_v4().to_string();
    vprintln("e2e_group_members: post marker, then fetch group members");
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
        &["group", "members", &test.name, &group_id.to_string()],
        &config_path,
    );
    assert!(output.status.success(), "group members failed");
}

#[test]
fn group_copy() {
    let Some((source, target)) = test_discourse_pair() else {
        return;
    };
    let Some(group_id) = source.test_group_id else {
        return;
    };
    let Some(topic_id) = source.test_topic_id else {
        return;
    };
    let marker = Uuid::new_v4().to_string();
    vprintln("e2e_group_copy: post marker, then copy group");
    post_and_verify(&source, topic_id, &marker);

    let source_client = DiscourseClient::new(&to_config(&source)).expect("client");
    let source_groups = source_client.fetch_groups().expect("groups");
    let source_group = source_groups
        .iter()
        .find(|group| group.id == group_id)
        .expect("source group");

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_temp_config(
        &dir,
        &format!(
            "[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n\n[[discourse]]\nname = \"{}\"\nbaseurl = \"{}\"\napikey = \"{}\"\napi_username = \"{}\"\n",
            source.name,
            source.baseurl,
            source.apikey,
            source.api_username,
            target.name,
            target.baseurl,
            target.apikey,
            target.api_username
        ),
    );
    let output = run_dsc(
        &[
            "group",
            "copy",
            &source.name,
            &group_id.to_string(),
            "--target",
            &target.name,
        ],
        &config_path,
    );
    assert!(output.status.success(), "group copy failed");
    let target_client = DiscourseClient::new(&to_config(&target)).expect("client");
    let target_groups = target_client.fetch_groups().expect("groups");
    let found = target_groups
        .iter()
        .find(|group| group.name == format!("{}-copy", slugify(&source_group.name)));
    let Some(found) = found else {
        panic!("copied group not found on target");
    };
    let source_detail = source_client
        .fetch_group_detail(source_group.id, Some(&source_group.name))
        .expect("source detail");
    let target_detail = target_client
        .fetch_group_detail(found.id, Some(&found.name))
        .expect("target detail");
    let source_fields = group_settings(&source_detail);
    let target_fields = group_settings(&target_detail);
    assert_eq!(
        target_fields.get("name"),
        Some(&format!("{}-copy", slugify(&source_detail.name))),
        "copy name mismatch"
    );
    if let Some(full_name) = source_detail.full_name.as_deref() {
        assert_eq!(
            target_fields.get("full_name"),
            Some(&format!("Copy of {}", full_name)),
            "copy full name mismatch"
        );
    }
    let mut expected_fields = source_fields.clone();
    expected_fields.insert(
        "name".to_string(),
        format!("{}-copy", slugify(&source_detail.name)),
    );
    if let Some(full_name) = source_detail.full_name.as_deref() {
        expected_fields.insert("full_name".to_string(), format!("Copy of {}", full_name));
    }
    assert_eq!(expected_fields, target_fields, "group settings differ");
}

fn group_settings(detail: &GroupDetail) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    insert_opt(&mut map, "name", Some(&detail.name));
    if let Some(full_name) = detail.full_name.as_deref() {
        insert_opt(&mut map, "full_name", Some(full_name));
    }
    insert_opt(&mut map, "title", detail.title.as_deref());
    insert_opt(
        &mut map,
        "grant_trust_level",
        detail.grant_trust_level.map(|v| v.to_string()).as_deref(),
    );
    insert_opt(
        &mut map,
        "visibility_level",
        detail.visibility_level.map(|v| v.to_string()).as_deref(),
    );
    insert_opt(
        &mut map,
        "mentionable_level",
        detail.mentionable_level.map(|v| v.to_string()).as_deref(),
    );
    insert_opt(
        &mut map,
        "messageable_level",
        detail.messageable_level.map(|v| v.to_string()).as_deref(),
    );
    insert_opt(
        &mut map,
        "default_notification_level",
        detail
            .default_notification_level
            .map(|v| v.to_string())
            .as_deref(),
    );
    insert_opt(
        &mut map,
        "members_visibility_level",
        detail
            .members_visibility_level
            .map(|v| v.to_string())
            .as_deref(),
    );
    insert_opt(
        &mut map,
        "primary_group",
        detail.primary_group.map(|v| v.to_string()).as_deref(),
    );
    insert_opt(
        &mut map,
        "public_admission",
        detail.public_admission.map(|v| v.to_string()).as_deref(),
    );
    insert_opt(
        &mut map,
        "public_exit",
        detail.public_exit.map(|v| v.to_string()).as_deref(),
    );
    insert_opt(
        &mut map,
        "allow_membership_requests",
        detail
            .allow_membership_requests
            .map(|v| v.to_string())
            .as_deref(),
    );
    insert_opt(
        &mut map,
        "automatic_membership_email_domains",
        detail.automatic_membership_email_domains.as_deref(),
    );
    insert_opt(
        &mut map,
        "automatic_membership_retroactive",
        detail
            .automatic_membership_retroactive
            .map(|v| v.to_string())
            .as_deref(),
    );
    insert_opt(
        &mut map,
        "membership_request_template",
        detail.membership_request_template.as_deref(),
    );
    insert_opt(&mut map, "flair_icon", detail.flair_icon.as_deref());
    insert_opt(
        &mut map,
        "flair_upload_id",
        detail.flair_upload_id.map(|v| v.to_string()).as_deref(),
    );
    insert_opt(&mut map, "flair_color", detail.flair_color.as_deref());
    insert_opt(
        &mut map,
        "flair_background_color",
        detail.flair_background_color.as_deref(),
    );
    insert_opt(&mut map, "bio_raw", detail.bio_raw.as_deref());
    map
}

fn insert_opt(map: &mut BTreeMap<String, String>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        map.insert(key.to_string(), value.to_string());
    }
}

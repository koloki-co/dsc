// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

// Shared test helpers - each test binary pulls in the whole module but uses
// only some helpers, so allow dead code rather than cfg-gate every item.
#![allow(dead_code)]

use dsc::api::DiscourseClient;
use dsc::config::DiscourseConfig;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub fn verbose_enabled() -> bool {
    std::env::var("DSC_TEST_VERBOSE")
        .or_else(|_| std::env::var("TEST_VERBOSE"))
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or_else(|_| std::env::args().any(|arg| arg == "-v" || arg == "--verbose"))
}

pub fn vprintln(message: &str) {
    if verbose_enabled() {
        eprintln!("[e2e] {}", message);
    }
}

#[derive(Debug, Deserialize)]
struct TestConfig {
    #[serde(default)]
    discourse: Vec<TestDiscourse>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TestDiscourse {
    pub name: String,
    pub baseurl: String,
    pub apikey: String,
    pub api_username: String,
    pub changelog_topic_id: Option<u64>,
    pub ssh_host: Option<String>,
    pub test_topic_id: Option<u64>,
    pub test_category_id: Option<u64>,
    pub test_color_scheme_id: Option<u64>,
    pub test_group_id: Option<u64>,
    pub ssh_enabled: Option<bool>,
    pub emoji_path: Option<String>,
    pub emoji_name: Option<String>,
    pub test_plugin_url: Option<String>,
    pub test_plugin_name: Option<String>,
    pub test_theme_url: Option<String>,
    pub test_theme_name: Option<String>,
    pub test_theme_id: Option<u64>,
    pub backup_enabled: Option<bool>,
    pub test_backup_path: Option<String>,
}

fn load_test_config() -> Option<TestConfig> {
    let live_tests_enabled = std::env::var("DSC_LIVE_TESTS")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !live_tests_enabled {
        return None;
    }

    let path = std::env::var("TEST_DSC_CONFIG").ok()?;
    let raw = fs::read_to_string(path).ok()?;
    toml::from_str(&raw).ok()
}

pub fn test_discourse() -> Option<TestDiscourse> {
    load_test_config()?.discourse.into_iter().next()
}

pub fn test_discourse_pair() -> Option<(TestDiscourse, TestDiscourse)> {
    let mut discourses = load_test_config()?.discourse.into_iter();
    let source = discourses.next()?;
    let target = discourses.next()?;
    Some((source, target))
}

pub fn to_config(d: &TestDiscourse) -> DiscourseConfig {
    DiscourseConfig {
        name: d.name.clone(),
        baseurl: d.baseurl.clone(),
        apikey: Some(d.apikey.clone()),
        api_username: Some(d.api_username.clone()),
        changelog_topic_id: d.changelog_topic_id,
        ssh_host: d.ssh_host.clone(),
        ..DiscourseConfig::default()
    }
}

pub fn post_and_verify(d: &TestDiscourse, topic_id: u64, marker: &str) {
    let config = to_config(d);
    let client = DiscourseClient::new(&config).expect("client");
    let body = format!("e2e marker: {}", marker);
    vprintln(&format!(
        "posting marker to topic {} on {}",
        topic_id, d.name
    ));
    let post_id = client.create_post(topic_id, &body).expect("post");
    let _cleanup = DeletePostOnDrop::new(client.clone(), post_id);
    vprintln(&format!("verifying marker on topic {}", topic_id));
    let topic = client.fetch_topic(topic_id, true).expect("fetch topic");
    let found = topic.post_stream.posts.iter().any(|post| {
        post.raw
            .as_ref()
            .map(|raw| raw.contains(marker))
            .unwrap_or(false)
    });
    assert!(found, "marker not found on forum");
}

/// Deletes a test-created post even if the assertion that follows panics.
pub struct DeletePostOnDrop {
    client: DiscourseClient,
    post_id: u64,
}

impl DeletePostOnDrop {
    pub fn new(client: DiscourseClient, post_id: u64) -> Self {
        Self { client, post_id }
    }
}

impl Drop for DeletePostOnDrop {
    fn drop(&mut self) {
        if let Err(error) = self.client.delete_post(self.post_id) {
            eprintln!(
                "[e2e] failed to delete test post {}: {error:#}",
                self.post_id
            );
        }
    }
}

/// Restores a test-mutated site setting even if the test panics.
pub struct RestoreSettingOnDrop {
    client: DiscourseClient,
    setting: String,
    original_value: String,
}

impl RestoreSettingOnDrop {
    pub fn new(client: DiscourseClient, setting: &str, original_value: String) -> Self {
        Self {
            client,
            setting: setting.to_string(),
            original_value,
        }
    }
}

impl Drop for RestoreSettingOnDrop {
    fn drop(&mut self) {
        if let Err(error) = self
            .client
            .update_site_setting(&self.setting, &self.original_value)
        {
            eprintln!(
                "[e2e] failed to restore site setting {}: {error:#}",
                self.setting
            );
        }
    }
}

/// Deletes a test-created theme even if the assertion that follows panics.
pub struct DeleteThemeOnDrop {
    client: DiscourseClient,
    theme_id: u64,
}

impl DeleteThemeOnDrop {
    pub fn new(client: DiscourseClient, theme_id: u64) -> Self {
        Self { client, theme_id }
    }
}

impl Drop for DeleteThemeOnDrop {
    fn drop(&mut self) {
        if let Err(error) = self.client.delete_theme(self.theme_id) {
            eprintln!(
                "[e2e] failed to delete test theme {}: {error:#}",
                self.theme_id
            );
        }
    }
}

pub fn run_dsc(args: &[&str], config_path: &Path) -> std::process::Output {
    vprintln(&format!("running dsc {}", args.join(" ")));
    Command::new(env!("CARGO_BIN_EXE_dsc"))
        .arg("-c")
        .arg(config_path)
        .args(args)
        .output()
        .expect("run dsc")
}

pub fn run_dsc_env(
    args: &[&str],
    config_path: &Path,
    envs: &[(&str, &str)],
) -> std::process::Output {
    vprintln(&format!(
        "running dsc {} with env overrides",
        args.join(" ")
    ));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dsc"));
    cmd.arg("-c").arg(config_path).args(args);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("run dsc")
}

pub fn write_temp_config(dir: &TempDir, content: &str) -> PathBuf {
    let path = dir.path().join("dsc.toml");
    fs::write(&path, content).expect("write config");
    vprintln(&format!("wrote temp config {}", path.display()));
    path
}

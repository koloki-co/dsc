// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

// Shared test helpers - each test binary pulls in the whole module but uses
// only some helpers, so allow dead code rather than cfg-gate every item.
#![allow(dead_code)]

use dsc::api::DiscourseClient;
use dsc::config::DiscourseConfig;
use dsc::utils::{atomic_write_private, normalize_baseurl};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use uuid::Uuid;

const LIVE_MARKER_PREFIX: &str = "dsc-live-";

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
    version: u32,
    #[serde(default)]
    discourse: Vec<TestDiscourse>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TestDiscourse {
    pub name: String,
    pub baseurl: String,
    pub apikey: String,
    pub api_username: String,
    #[serde(default)]
    pub disposable: bool,
    #[serde(default)]
    pub ephemeral: bool,
    pub changelog_topic_id: Option<u64>,
    pub ssh_host: Option<String>,
    pub test_topic_id: Option<u64>,
    pub test_category_id: Option<u64>,
    pub test_color_scheme_id: Option<u64>,
    pub test_group_id: Option<u64>,
    pub ssh_enabled: Option<bool>,
    pub test_theme_id: Option<u64>,
    pub backup_enabled: Option<bool>,
}

fn load_test_config() -> Option<TestConfig> {
    let live_tests = std::env::var("DSC_LIVE_TESTS").unwrap_or_default();
    if live_tests != "1" {
        return None;
    }

    assert_eq!(
        std::env::var("DSC_LIVE_TEST_RUNNER").as_deref(),
        Ok("1"),
        "live tests must run through s/test-live"
    );
    let run_id = std::env::var("DSC_LIVE_TEST_RUN_ID")
        .expect("s/test-live must provide DSC_LIVE_TEST_RUN_ID");
    assert!(!run_id.trim().is_empty(), "live-test run ID is empty");

    let path = PathBuf::from(
        std::env::var("TEST_DSC_CONFIG").expect("DSC_LIVE_TESTS=1 requires TEST_DSC_CONFIG"),
    );
    assert!(path.is_absolute(), "TEST_DSC_CONFIG must be absolute");
    assert!(path.is_file(), "TEST_DSC_CONFIG is not a readable file");
    assert_private_config(&path);

    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
    let config: TestConfig = toml::from_str(&raw)
        .unwrap_or_else(|_| panic!("parsing {} failed; fix the TOML syntax", path.display()));
    validate_test_config(&config).unwrap_or_else(|error| panic!("{error}"));
    Some(config)
}

pub fn validate_test_config_toml(raw: &str) -> Result<(), String> {
    let config: TestConfig =
        toml::from_str(raw).map_err(|_| "live-test config is not valid TOML".to_string())?;
    validate_test_config(&config)
}

fn validate_test_config(config: &TestConfig) -> Result<(), String> {
    if config.version != 1 {
        return Err("live-test config must set version = 1".to_string());
    }
    if config.discourse.is_empty() {
        return Err("live-test config contains no [[discourse]] entries".to_string());
    }
    for discourse in &config.discourse {
        if !discourse.disposable {
            return Err(format!(
                "live-test forum '{}' must set disposable = true",
                discourse.name
            ));
        }
        for (field, value) in [
            ("name", discourse.name.as_str()),
            ("baseurl", discourse.baseurl.as_str()),
            ("apikey", discourse.apikey.as_str()),
            ("api_username", discourse.api_username.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!(
                    "live-test forum '{}' has an empty {field}",
                    discourse.name
                ));
            }
        }
        if !discourse.baseurl.starts_with("https://") {
            return Err(format!(
                "live-test forum '{}' must use HTTPS",
                discourse.name
            ));
        }
        for (field, value) in [
            ("test_topic_id", discourse.test_topic_id),
            ("test_category_id", discourse.test_category_id),
            ("test_color_scheme_id", discourse.test_color_scheme_id),
            ("test_group_id", discourse.test_group_id),
            ("test_theme_id", discourse.test_theme_id),
        ] {
            if !value.is_some_and(|id| id > 0) {
                return Err(format!(
                    "live-test forum '{}' must set a non-zero {field}",
                    discourse.name
                ));
            }
        }
        if discourse.ssh_enabled == Some(true) {
            if !discourse
                .ssh_host
                .as_deref()
                .is_some_and(|host| !host.trim().is_empty())
            {
                return Err(format!(
                    "live-test forum '{}' enables SSH but has no ssh_host",
                    discourse.name
                ));
            }
            if !discourse.changelog_topic_id.is_some_and(|id| id > 0) {
                return Err(format!(
                    "live-test forum '{}' enables SSH but has no changelog_topic_id",
                    discourse.name
                ));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn assert_private_config(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path)
        .unwrap_or_else(|error| panic!("reading metadata for {}: {error}", path.display()))
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o077,
        0,
        "{} contains credentials and must be mode 0600 (or stricter)",
        path.display()
    );
}

#[cfg(not(unix))]
fn assert_private_config(_path: &Path) {}

pub fn live_test_run_id() -> String {
    std::env::var("DSC_LIVE_TEST_RUN_ID").expect("missing live-test run ID")
}

pub fn live_test_marker(label: &str) -> String {
    format!(
        "{LIVE_MARKER_PREFIX}{}-{label}-{}",
        live_test_run_id(),
        Uuid::new_v4()
    )
}

pub fn test_discourse() -> Option<TestDiscourse> {
    load_test_config()?.discourse.into_iter().next()
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

pub fn validate_live_forum(discourse: &TestDiscourse) -> anyhow::Result<()> {
    let client = DiscourseClient::new(&to_config(discourse))?;
    let enabled = client.fetch_site_setting("can_permanently_delete")?;
    if !enabled.eq_ignore_ascii_case("true") {
        anyhow::bail!(
            "live-test forum '{}' must enable can_permanently_delete",
            discourse.name
        );
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum LiveResource {
    Topic {
        topic_id: u64,
        first_post_id: u64,
        marker: String,
    },
    Post {
        post_id: u64,
        topic_id: u64,
        marker: String,
    },
}

impl LiveResource {
    fn check_post_id(&self) -> u64 {
        match self {
            Self::Topic { first_post_id, .. } => *first_post_id,
            Self::Post { post_id, .. } => *post_id,
        }
    }

    fn description(&self) -> String {
        match self {
            Self::Topic { topic_id, .. } => format!("topic {topic_id}"),
            Self::Post { post_id, .. } => format!("post {post_id}"),
        }
    }

    fn record(&self) -> String {
        match self {
            Self::Topic {
                topic_id,
                first_post_id,
                marker,
            } => format!("topic {topic_id} {first_post_id} {marker}"),
            Self::Post {
                post_id,
                topic_id,
                marker,
            } => format!("post {post_id} {topic_id} {marker}"),
        }
    }
}

fn live_test_state_path() -> PathBuf {
    let path = PathBuf::from(
        std::env::var("DSC_LIVE_TEST_STATE").expect("s/test-live must provide DSC_LIVE_TEST_STATE"),
    );
    assert!(path.is_absolute(), "live-test state path must be absolute");
    path
}

fn record_live_resource(forum: &str, resource: LiveResource) -> anyhow::Result<()> {
    let mut resources = recorded_live_resources(forum)?;
    resources.insert(resource);
    write_recorded_live_resources(forum, &resources)
}

fn recorded_live_resources(expected_forum: &str) -> anyhow::Result<HashSet<LiveResource>> {
    let raw = fs::read_to_string(live_test_state_path())?;
    if raw.trim().is_empty() {
        return Ok(HashSet::new());
    }
    let mut lines = raw.lines();
    let forum_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("live-test resource journal has no forum identity"))?;
    let recorded_forum = forum_line
        .strip_prefix("forum ")
        .ok_or_else(|| anyhow::anyhow!("live-test resource journal has no forum identity"))?;
    if recorded_forum != expected_forum {
        anyhow::bail!(
            "live-test resource journal belongs to {recorded_forum}, not {expected_forum}; restore the original config target before cleanup"
        );
    }
    lines
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields.as_slice() {
                ["topic", topic_id, first_post_id, marker]
                    if marker.starts_with(LIVE_MARKER_PREFIX) =>
                {
                    Ok(LiveResource::Topic {
                        topic_id: topic_id.parse()?,
                        first_post_id: first_post_id.parse()?,
                        marker: marker.to_string(),
                    })
                }
                ["post", post_id, topic_id, marker] if marker.starts_with(LIVE_MARKER_PREFIX) => {
                    Ok(LiveResource::Post {
                        post_id: post_id.parse()?,
                        topic_id: topic_id.parse()?,
                        marker: marker.to_string(),
                    })
                }
                _ => anyhow::bail!("invalid live-test resource record: {line}"),
            }
        })
        .collect()
}

fn write_recorded_live_resources(
    forum: &str,
    resources: &HashSet<LiveResource>,
) -> anyhow::Result<()> {
    let mut lines: Vec<String> = resources.iter().map(LiveResource::record).collect();
    lines.sort();
    let content = if lines.is_empty() {
        format!("forum {forum}\n")
    } else {
        format!("forum {forum}\n{}\n", lines.join("\n"))
    };
    atomic_write_private(&live_test_state_path(), content, true)
}

fn topic_resource(
    client: &DiscourseClient,
    topic_id: u64,
    marker: &str,
) -> anyhow::Result<LiveResource> {
    let topic = client.fetch_topic(topic_id, false)?;
    let first_post_id = topic
        .post_stream
        .posts
        .first()
        .map(|post| post.id)
        .ok_or_else(|| anyhow::anyhow!("topic {topic_id} has no first post"))?;
    Ok(LiveResource::Topic {
        topic_id,
        first_post_id,
        marker: marker.to_string(),
    })
}

enum ResourceState {
    Absent,
    Active,
    Deleted,
}

fn inspect_live_resource(
    client: &DiscourseClient,
    resource: &LiveResource,
    category_id: u64,
) -> anyhow::Result<ResourceState> {
    match resource {
        LiveResource::Topic {
            topic_id,
            first_post_id,
            marker,
        } => {
            if client.topic_is_absent(*topic_id)? {
                return Ok(ResourceState::Absent);
            }
            let topic = client.fetch_topic(*topic_id, false)?;
            let actual_first_post_id = topic
                .post_stream
                .posts
                .first()
                .map(|post| post.id)
                .ok_or_else(|| anyhow::anyhow!("topic {topic_id} has no first post"))?;
            if topic.category_id != Some(category_id)
                || actual_first_post_id != *first_post_id
                || !topic
                    .title
                    .as_deref()
                    .is_some_and(|title| title.contains(marker))
            {
                anyhow::bail!(
                    "refusing to clean journalled topic {topic_id}: current resource does not match marker ownership"
                );
            }
            Ok(if topic.deleted_at.is_some() {
                ResourceState::Deleted
            } else {
                ResourceState::Active
            })
        }
        LiveResource::Post {
            post_id,
            topic_id,
            marker,
        } => {
            if client.post_is_absent(*post_id)? {
                return Ok(ResourceState::Absent);
            }
            let post = client.fetch_post(*post_id)?;
            if post.topic_id != *topic_id
                || !post.raw.as_deref().is_some_and(|raw| raw.contains(marker))
            {
                anyhow::bail!(
                    "refusing to clean journalled post {post_id}: current resource does not match marker ownership"
                );
            }
            Ok(if post.deleted_at.is_some() {
                ResourceState::Deleted
            } else {
                ResourceState::Active
            })
        }
    }
}

fn force_destroy_when_ready(
    client: &DiscourseClient,
    resources: HashSet<LiveResource>,
    category_id: u64,
    forum: &str,
) -> anyhow::Result<()> {
    let mut pending = resources;
    let deadline = Instant::now() + Duration::from_secs(6 * 60);
    let mut announced_wait = false;

    for resource in pending.clone() {
        match inspect_live_resource(client, &resource, category_id)? {
            ResourceState::Absent => {
                pending.remove(&resource);
                write_recorded_live_resources(forum, &pending)?;
            }
            ResourceState::Deleted => {}
            ResourceState::Active => anyhow::bail!(
                "refusing to force-destroy active journalled {}",
                resource.description()
            ),
        }
    }

    while !pending.is_empty() {
        let mut ready = Vec::new();
        let mut reasons = Vec::new();
        for resource in &pending {
            let (can_delete, reason) = client.permanent_delete_check(resource.check_post_id())?;
            if can_delete {
                ready.push(resource.clone());
            } else if let Some(reason) = reason {
                reasons.push(reason);
            }
        }

        for resource in ready {
            match inspect_live_resource(client, &resource, category_id)? {
                ResourceState::Absent => {
                    pending.remove(&resource);
                    write_recorded_live_resources(forum, &pending)?;
                    continue;
                }
                ResourceState::Deleted => {}
                ResourceState::Active => anyhow::bail!(
                    "refusing to force-destroy active journalled {}",
                    resource.description()
                ),
            }
            match &resource {
                LiveResource::Topic { topic_id, .. } => {
                    client.delete_topic(*topic_id, true)?;
                    if !client.topic_is_absent(*topic_id)? {
                        anyhow::bail!("topic {topic_id} still exists after force-destroy");
                    }
                }
                LiveResource::Post { post_id, .. } => {
                    client.permanently_delete_post(*post_id)?;
                    if !client.post_is_absent(*post_id)? {
                        anyhow::bail!("post {post_id} still exists after force-destroy");
                    }
                }
            }
            pending.remove(&resource);
            write_recorded_live_resources(forum, &pending)?;
        }

        if pending.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "timed out waiting to permanently delete {}: {}",
                pending
                    .iter()
                    .map(|resource| resource.description())
                    .collect::<Vec<_>>()
                    .join(", "),
                reasons.join("; ")
            );
        }
        if !announced_wait {
            eprintln!(
                "[live] waiting for Discourse's permanent-delete safety window: {}",
                reasons.join("; ")
            );
            announced_wait = true;
        }
        std::thread::sleep(Duration::from_secs(5));
    }

    Ok(())
}

fn cleanup_failed(resource: &str, error: &anyhow::Error) {
    if std::thread::panicking() {
        eprintln!("[live] failed to clean up {resource}: {error:#}");
    } else {
        panic!("failed to clean up {resource}: {error:#}");
    }
}

/// Deletes every topic carrying a per-run marker, including a topic whose
/// create response was lost before its ID could be recorded.
pub struct DeleteTopicsByMarkerOnDrop {
    client: DiscourseClient,
    forum: String,
    category_id: u64,
    marker: String,
}

impl DeleteTopicsByMarkerOnDrop {
    pub fn new(discourse: &TestDiscourse, category_id: u64, marker: &str) -> Self {
        Self {
            client: DiscourseClient::new(&to_config(discourse)).expect("cleanup client"),
            forum: normalize_baseurl(&discourse.baseurl),
            category_id,
            marker: marker.to_string(),
        }
    }
}

impl Drop for DeleteTopicsByMarkerOnDrop {
    fn drop(&mut self) {
        let result = (|| -> anyhow::Result<()> {
            let category = self.client.fetch_category(self.category_id)?;
            for topic in category
                .topic_list
                .topics
                .into_iter()
                .filter(|topic| topic.title.contains(&self.marker))
            {
                record_live_resource(
                    &self.forum,
                    topic_resource(&self.client, topic.id, &self.marker)?,
                )?;
                self.client.delete_topic(topic.id, false)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            cleanup_failed(&format!("topics containing marker {}", self.marker), &error);
        }
    }
}

/// Deletes replies carrying a per-run marker, even if command output failed
/// before the created post ID could be parsed.
pub struct DeletePostsByMarkerOnDrop {
    client: DiscourseClient,
    forum: String,
    topic_id: u64,
    marker: String,
}

impl DeletePostsByMarkerOnDrop {
    pub fn new(discourse: &TestDiscourse, topic_id: u64, marker: &str) -> Self {
        Self {
            client: DiscourseClient::new(&to_config(discourse)).expect("cleanup client"),
            forum: normalize_baseurl(&discourse.baseurl),
            topic_id,
            marker: marker.to_string(),
        }
    }
}

impl Drop for DeletePostsByMarkerOnDrop {
    fn drop(&mut self) {
        let result = (|| -> anyhow::Result<()> {
            let topic = self.client.fetch_topic_all_posts(self.topic_id)?;
            for post in topic.post_stream.posts.into_iter().filter(|post| {
                post.post_number != Some(1)
                    && post
                        .raw
                        .as_deref()
                        .is_some_and(|raw| raw.contains(&self.marker))
            }) {
                record_live_resource(
                    &self.forum,
                    LiveResource::Post {
                        post_id: post.id,
                        topic_id: self.topic_id,
                        marker: self.marker.clone(),
                    },
                )?;
                if post.deleted_at.is_none() {
                    self.client.delete_post(post.id)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            cleanup_failed(&format!("posts containing marker {}", self.marker), &error);
        }
    }
}

pub struct DisposableTopic {
    pub client: DiscourseClient,
    pub id: u64,
    pub marker: String,
    _cleanup: DeleteTopicsByMarkerOnDrop,
}

pub fn create_disposable_topic(discourse: &TestDiscourse, label: &str) -> DisposableTopic {
    let category_id = discourse
        .test_category_id
        .expect("validated live-test category ID");
    let marker = live_test_marker(label);
    let forum = normalize_baseurl(&discourse.baseurl);
    let client = DiscourseClient::new(&to_config(discourse)).expect("client");
    let cleanup = DeleteTopicsByMarkerOnDrop::new(discourse, category_id, &marker);
    let id = client
        .create_topic(
            category_id,
            &format!("DSC live test {marker}"),
            &format!("Disposable topic for {marker}"),
        )
        .expect("create disposable topic");
    record_live_resource(
        &forum,
        topic_resource(&client, id, &marker).expect("created topic first post"),
    )
    .expect("record disposable topic");
    DisposableTopic {
        client,
        id,
        marker,
        _cleanup: cleanup,
    }
}

/// Remove resources left by an interrupted current or previous live run.
/// Returns descriptions so the preflight can report recovery and the
/// postflight can fail if normal per-test cleanup did not complete.
pub fn cleanup_live_resources(discourse: &TestDiscourse) -> anyhow::Result<Vec<String>> {
    let client = DiscourseClient::new(&to_config(discourse))?;
    let forum = normalize_baseurl(&discourse.baseurl);
    let category_id = discourse
        .test_category_id
        .expect("validated live-test category ID");
    let mut resources = recorded_live_resources(&forum)?;
    let mut removed = Vec::new();

    for resource in resources.clone() {
        match inspect_live_resource(&client, &resource, category_id)? {
            ResourceState::Absent => {
                resources.remove(&resource);
            }
            ResourceState::Active => {
                removed.push(resource.description());
                match &resource {
                    LiveResource::Topic { topic_id, .. } => {
                        client.delete_topic(*topic_id, false)?
                    }
                    LiveResource::Post { post_id, .. } => client.delete_post(*post_id)?,
                }
            }
            ResourceState::Deleted => {}
        }
    }

    let category = client.fetch_category(category_id)?;
    for topic in category
        .topic_list
        .topics
        .into_iter()
        .filter(|topic| topic.title.contains(LIVE_MARKER_PREFIX))
    {
        let marker = live_marker_in(&topic.title)?;
        let resource = topic_resource(&client, topic.id, marker)?;
        if resources.insert(resource.clone()) {
            removed.push(resource.description());
        }
        client.delete_topic(topic.id, false)?;
    }

    for topic in client
        .list_deleted_topics_in_category(category_id)?
        .into_iter()
        .filter(|topic| topic.title.contains(LIVE_MARKER_PREFIX))
    {
        let marker = live_marker_in(&topic.title)?;
        let resource = topic_resource(&client, topic.id, marker)?;
        if resources.insert(resource.clone()) {
            removed.push(format!("deleted {}", resource.description()));
        }
    }

    if let Some(topic_id) = discourse.changelog_topic_id {
        let topic = client.fetch_topic_all_posts(topic_id)?;
        for post in topic.post_stream.posts.into_iter().filter(|post| {
            post.post_number != Some(1)
                && post
                    .raw
                    .as_deref()
                    .is_some_and(|raw| raw.contains(LIVE_MARKER_PREFIX))
        }) {
            let marker = live_marker_in(post.raw.as_deref().expect("filtered post raw"))?;
            let resource = LiveResource::Post {
                post_id: post.id,
                topic_id,
                marker: marker.to_string(),
            };
            if resources.insert(resource.clone()) {
                removed.push(resource.description());
            }
            if post.deleted_at.is_none() {
                client.delete_post(post.id)?;
            }
        }
    }

    write_recorded_live_resources(&forum, &resources)?;
    force_destroy_when_ready(&client, resources, category_id, &forum)?;
    Ok(removed)
}

fn live_marker_in(value: &str) -> anyhow::Result<&str> {
    value
        .split_whitespace()
        .find(|part| part.starts_with(LIVE_MARKER_PREFIX))
        .ok_or_else(|| anyhow::anyhow!("marked live-test resource is missing its marker token"))
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

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{DiscourseClient, TagGroupInfo};
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, not_found, select_discourse};
use crate::config::Config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub fn tag_list(config: &Config, discourse_name: &str, format: ListFormat) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let mut tags = client.list_tags()?;
    tags.sort_by(|a, b| a.text.cmp(&b.text));

    match format {
        ListFormat::Text => {
            if tags.is_empty() {
                println!("No tags found.");
                return Ok(());
            }
            let name_width = tags.iter().map(|t| t.text.len()).max().unwrap_or(0).max(4);
            for tag in &tags {
                println!("{:<width$}  {}", tag.text, tag.count, width = name_width);
            }
        }
        ListFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&tags)?);
        }
        ListFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&tags)?);
        }
    }

    Ok(())
}

pub fn tag_apply(
    config: &Config,
    discourse_name: &str,
    topic_id: u64,
    tag: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let current = client.fetch_topic_tags(topic_id)?;
    let Some(next) = next_tags_after_apply(&current, tag) else {
        println!("Topic {} already tagged '{}'", topic_id, tag);
        return Ok(());
    };
    if dry_run {
        println!(
            "[dry-run] would set tags on topic {} to: [{}]",
            topic_id,
            next.join(", ")
        );
        return Ok(());
    }
    let after = client.set_topic_tags(topic_id, &next)?;
    println!("Topic {} tags: [{}]", topic_id, after.join(", "));
    Ok(())
}

/// Compute the resulting tag list when adding `tag` to `current`. Returns
/// None when the tag is already present.
fn next_tags_after_apply(current: &[String], tag: &str) -> Option<Vec<String>> {
    if current.iter().any(|t| t == tag) {
        return None;
    }
    let mut next = current.to_vec();
    next.push(tag.to_string());
    Some(next)
}

/// Compute the resulting tag list when removing `tag` from `current`. Returns
/// None when the tag is not present.
fn next_tags_after_remove(current: &[String], tag: &str) -> Option<Vec<String>> {
    if !current.iter().any(|t| t == tag) {
        return None;
    }
    Some(current.iter().filter(|t| *t != tag).cloned().collect())
}

pub fn tag_remove(
    config: &Config,
    discourse_name: &str,
    topic_id: u64,
    tag: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let current = client.fetch_topic_tags(topic_id)?;
    let Some(next) = next_tags_after_remove(&current, tag) else {
        println!("Topic {} does not have tag '{}'", topic_id, tag);
        return Ok(());
    };
    if dry_run {
        println!(
            "[dry-run] would set tags on topic {} to: [{}]",
            topic_id,
            next.join(", ")
        );
        return Ok(());
    }
    let after = client.set_topic_tags(topic_id, &next)?;
    println!("Topic {} tags: [{}]", topic_id, after.join(", "));
    Ok(())
}

/// Rename a tag on the server, preserving every topic association.
///
/// Discourse's tag-update endpoint accepts a new `id` (slug) which it then
/// applies in-place to every topic carrying the old tag. This is the safe
/// alternative to delete+create, which would unlink every topic.
pub fn tag_rename(
    config: &Config,
    discourse_name: &str,
    old_name: &str,
    new_name: &str,
    dry_run: bool,
) -> Result<()> {
    let (old_norm, new_norm) = validate_rename_names(old_name, new_name)?;

    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    // Look up the old tag and ensure the new name is not already taken.
    let tags = client.list_tags()?;
    if !tags.iter().any(|t| t.text == old_norm) {
        return Err(not_found("tag", &old_norm));
    }
    if tags.iter().any(|t| t.text == new_norm) {
        return Err(anyhow::anyhow!(
            "cannot rename to '{}': a tag with that name already exists on '{}' (would merge; not supported)",
            new_norm,
            discourse_name
        ));
    }

    if dry_run {
        println!(
            "[dry-run] would rename tag '{}' -> '{}' on '{}'",
            old_norm, new_norm, discourse_name
        );
        return Ok(());
    }

    client.rename_tag(&old_norm, &new_norm)?;
    println!("Renamed tag '{}' -> '{}'", old_norm, new_norm);
    Ok(())
}

/// Validate and normalise the rename inputs. Returns `(old, new)` after
/// trimming. Rejects empty names, identical names, and obvious-typo whitespace.
fn validate_rename_names(old: &str, new: &str) -> Result<(String, String)> {
    let old_t = old.trim();
    let new_t = new.trim();
    if old_t.is_empty() {
        return Err(anyhow::anyhow!("old tag name is empty"));
    }
    if new_t.is_empty() {
        return Err(anyhow::anyhow!("new tag name is empty"));
    }
    if old_t == new_t {
        return Err(anyhow::anyhow!(
            "old and new tag names are identical: '{}'",
            old_t
        ));
    }
    if new_t.chars().any(|c| c.is_whitespace()) {
        return Err(anyhow::anyhow!(
            "new tag name '{}' contains whitespace; Discourse tags must be slug-style",
            new_t
        ));
    }
    Ok((old_t.to_string(), new_t.to_string()))
}

// ─── Taxonomy file schema ─────────────────────────────────────────────────────

/// The on-disk taxonomy file (version 1).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TaxonomyFile {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_groups: Vec<TagGroupEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TagEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TagGroupEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub one_per_topic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn is_false(v: &bool) -> bool {
    !v
}

// ─── Pull ─────────────────────────────────────────────────────────────────────

pub fn tag_pull(config: &Config, discourse_name: &str, local_path: &Path) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let server_tags = client.list_tags()?;

    // Collect tag entries with descriptions
    let mut tag_entries: Vec<TagEntry> = Vec::new();
    for t in &server_tags {
        let description = client.get_tag_description(&t.text).unwrap_or(None);
        tag_entries.push(TagEntry {
            name: t.text.clone(),
            description,
        });
    }
    tag_entries.sort_by(|a, b| a.name.cmp(&b.name));

    // Attempt tag groups (admin-only)
    let tag_groups = match client.list_tag_groups()? {
        Some(groups) => {
            let group_names_by_id = group_names_by_id(&client)?;
            let mut entries: Vec<TagGroupEntry> = groups
                .into_iter()
                .map(|g| {
                    let permissions = g
                        .permissions
                        .as_ref()
                        .map(|p| parse_tag_group_permissions(p, &group_names_by_id))
                        .transpose()?
                        .flatten();
                    let mut tags = g.tag_names;
                    tags.sort();
                    Ok(TagGroupEntry {
                        name: g.name,
                        description: None, // not returned by list endpoint
                        one_per_topic: g.one_per_topic,
                        parent_tag: g.parent_tag_name,
                        permissions,
                        tags,
                    })
                })
                .collect::<Result<_>>()?;
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            entries
        }
        None => {
            eprintln!(
                "Warning: tag groups not accessible (requires admin API key); omitting from output."
            );
            Vec::new()
        }
    };

    let taxonomy = TaxonomyFile {
        version: 1,
        tags: tag_entries,
        tag_groups,
    };

    let content = if is_json_path(local_path) {
        serde_json::to_string_pretty(&taxonomy).context("serializing taxonomy as JSON")?
    } else {
        serde_yaml::to_string(&taxonomy).context("serializing taxonomy as YAML")?
    };

    fs::write(local_path, &content).with_context(|| format!("writing {}", local_path.display()))?;
    println!("Wrote taxonomy to {}", local_path.display());
    Ok(())
}

/// Return the site's group names keyed by their numeric IDs. Tag-group
/// permissions use these IDs in the Discourse API, while the taxonomy file uses
/// names so it can be shared across sites. `everyone` is the built-in group 0.
fn group_names_by_id(client: &DiscourseClient) -> Result<BTreeMap<u64, String>> {
    let mut names = client
        .fetch_groups()?
        .into_iter()
        .map(|group| (group.id, group.name))
        .collect::<BTreeMap<_, _>>();
    names.entry(0).or_insert_with(|| "everyone".to_string());
    Ok(names)
}

fn group_ids_by_name(group_names_by_id: &BTreeMap<u64, String>) -> BTreeMap<String, u64> {
    group_names_by_id
        .iter()
        .map(|(id, name)| (name.clone(), *id))
        .collect()
}

/// Normalise a numeric Discourse permission level for the taxonomy file.
fn permission_label(level: &serde_json::Value) -> Result<String> {
    if let Some(level) = level.as_u64() {
        return Ok(match level {
            1 => "full".to_string(),
            2 => "create_post".to_string(),
            3 => "readonly".to_string(),
            other => other.to_string(),
        });
    }

    let label = level
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("tag-group permission level must be a string or number"))?;
    match label {
        "full" | "create_post" | "readonly" => Ok(label.to_string()),
        other => other
            .parse::<u64>()
            .map(|level| match level {
                1 => "full".to_string(),
                2 => "create_post".to_string(),
                3 => "readonly".to_string(),
                other => other.to_string(),
            })
            .with_context(|| format!("unknown tag-group permission level '{other}'")),
    }
}

/// Convert a taxonomy permission label back to the numeric API value.
fn permission_level(label: &str) -> Result<u64> {
    match label {
        "full" => Ok(1),
        "create_post" => Ok(2),
        "readonly" => Ok(3),
        other => other
            .parse()
            .with_context(|| format!("unknown tag-group permission level '{other}'")),
    }
}

/// Parse the API's numeric-group-ID permission map into file-safe group names.
fn parse_tag_group_permissions(
    value: &serde_json::Value,
    group_names_by_id: &BTreeMap<u64, String>,
) -> Result<Option<BTreeMap<String, String>>> {
    let Some(obj) = value.as_object() else {
        return Ok(None);
    };
    if obj.is_empty() {
        return Ok(None);
    }

    let mut map = BTreeMap::new();
    for (group_id, level) in obj {
        // Current Discourse returns numeric IDs, but accepting name-keyed data
        // keeps pulled files portable across older API responses.
        let group_name = match group_id.parse::<u64>() {
            Ok(id) => group_names_by_id.get(&id).cloned().ok_or_else(|| {
                anyhow::anyhow!("tag-group permission references unknown group ID {id}")
            })?,
            Err(_) => group_id.clone(),
        };
        map.insert(group_name, permission_label(level)?);
    }
    Ok(Some(map))
}

fn is_json_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

// ─── Push ─────────────────────────────────────────────────────────────────────

/// The reconciliation plan for tags (not groups), computed before any writes so
/// the dry-run and the apply path share one source of truth.
///
/// Discourse has no admin create-tag endpoint (`PUT /tag/{name}.json` 404s for a
/// non-existent tag); a tag is materialised only by being placed in a tag group
/// or assigned to a topic. So creation is a side effect of group reconciliation,
/// and any desired tag that belongs to no group and does not already exist
/// simply cannot be created - that case is surfaced, not silently dropped.
#[derive(Debug, Default, PartialEq)]
struct TagPlan {
    /// Desired tags absent from the server that a group will materialise
    /// (named in a group). Created as a side effect of group create/update.
    created_via_group: Vec<String>,
    /// `(name, description)` for tags that will exist after group reconciliation
    /// and carry a description to set.
    set_description: Vec<(String, String)>,
    /// Explicit `tags:` entries in no group and absent from the server: no API
    /// can create them. Reported so the run does not pretend to have applied them.
    groupless_missing: Vec<String>,
    /// Tags on the server but not desired (prune only).
    to_delete: Vec<String>,
}

impl TagPlan {
    fn is_empty(&self) -> bool {
        self.created_via_group.is_empty()
            && self.set_description.is_empty()
            && self.groupless_missing.is_empty()
            && self.to_delete.is_empty()
    }
}

/// Partition the desired tags against the server. `group_tags` is the set of
/// tags that group reconciliation will materialise (empty when the admin group
/// endpoint is unreachable).
fn plan_tags(
    explicit: &BTreeMap<String, Option<String>>,
    group_tags: &BTreeSet<String>,
    server_tags: &BTreeSet<String>,
    prune: bool,
) -> TagPlan {
    // Tags that will exist once groups are reconciled.
    let will_exist: BTreeSet<String> = server_tags.iter().chain(group_tags).cloned().collect();

    let created_via_group: Vec<String> = group_tags.difference(server_tags).cloned().collect();

    let mut set_description: Vec<(String, String)> = explicit
        .iter()
        .filter_map(|(name, desc)| match desc {
            Some(d) if will_exist.contains(name) => Some((name.clone(), d.clone())),
            _ => None,
        })
        .collect();
    set_description.sort();

    let groupless_missing: Vec<String> = explicit
        .keys()
        .filter(|name| !group_tags.contains(*name) && !server_tags.contains(*name))
        .cloned()
        .collect();

    let to_delete: Vec<String> = if prune {
        let desired: BTreeSet<&String> = explicit.keys().chain(group_tags).collect();
        server_tags
            .iter()
            .filter(|t| !desired.contains(t))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    TagPlan {
        created_via_group,
        set_description,
        groupless_missing,
        to_delete,
    }
}

pub fn tag_push(
    config: &Config,
    discourse_name: &str,
    local_path: &Path,
    prune: bool,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let content = fs::read_to_string(local_path)
        .with_context(|| format!("reading {}", local_path.display()))?;
    let taxonomy: TaxonomyFile = if is_json_path(local_path) {
        serde_json::from_str(&content).context("parsing taxonomy JSON")?
    } else {
        serde_yaml::from_str(&content).context("parsing taxonomy YAML")?
    };

    if taxonomy.version != 1 {
        anyhow::bail!("unsupported taxonomy file version: {}", taxonomy.version);
    }

    // The file's tags arrive in two forms: explicit `tags:` entries (each may
    // carry a description) and tags named inside a group. A tag group's
    // `POST /tag_groups.json` with `tag_names` is the ONLY admin-API way to
    // create a tag, so groups are reconciled FIRST; their tags then exist and
    // descriptions can be set. See `update_tag` for why.
    let explicit: BTreeMap<String, Option<String>> = taxonomy
        .tags
        .iter()
        .map(|t| (t.name.clone(), t.description.clone()))
        .collect();
    let group_tags: BTreeSet<String> = taxonomy
        .tag_groups
        .iter()
        .flat_map(|g| g.tags.clone())
        .collect();

    let server_tag_names: BTreeSet<String> =
        client.list_tags()?.into_iter().map(|t| t.text).collect();

    // Group reconciliation needs the admin endpoint; fetch it up front so the
    // whole plan reflects what will actually happen. Without it, no group tags
    // can be materialised, so those tags fall through to `groupless_missing`.
    let server_groups = client.list_tag_groups()?;
    let groups_available = server_groups.is_some();
    if !groups_available && !taxonomy.tag_groups.is_empty() {
        eprintln!(
            "Warning: tag groups not accessible (requires admin API key); groups in the file cannot be reconciled and their tags cannot be created."
        );
    }
    let effective_group_tags: BTreeSet<String> = if groups_available {
        group_tags.clone()
    } else {
        BTreeSet::new()
    };
    let server_groups = server_groups.unwrap_or_default();
    let group_names_by_id = if groups_available
        && taxonomy
            .tag_groups
            .iter()
            .any(|group| group.permissions.is_some())
    {
        group_names_by_id(&client)?
    } else {
        BTreeMap::new()
    };
    let group_ids_by_name = group_ids_by_name(&group_names_by_id);

    let tag_plan = plan_tags(&explicit, &effective_group_tags, &server_tag_names, prune);

    // ── Compute the tag-group plan (only when the admin endpoint is reachable) ─
    let server_groups_by_name: BTreeMap<String, &TagGroupInfo> =
        server_groups.iter().map(|g| (g.name.clone(), g)).collect();
    let desired_group_names: BTreeSet<String> =
        taxonomy.tag_groups.iter().map(|g| g.name.clone()).collect();
    let server_group_names: BTreeSet<String> =
        server_groups.iter().map(|g| g.name.clone()).collect();

    let groups_to_create: Vec<&TagGroupEntry> = if groups_available {
        taxonomy
            .tag_groups
            .iter()
            .filter(|g| !server_group_names.contains(&g.name))
            .collect()
    } else {
        Vec::new()
    };
    let mut groups_to_update: Vec<(&TagGroupEntry, u64)> = Vec::new();
    if groups_available {
        for desired in &taxonomy.tag_groups {
            if let Some(server) = server_groups_by_name.get(&desired.name)
                && tag_group_needs_update(desired, server, &group_names_by_id)?
            {
                groups_to_update.push((desired, server.id));
            }
        }
    }
    let groups_to_delete: Vec<(&str, u64)> = if prune && groups_available {
        server_groups
            .iter()
            .filter(|g| !desired_group_names.contains(&g.name))
            .map(|g| (g.name.as_str(), g.id))
            .collect()
    } else {
        Vec::new()
    };

    // ── Dry-run: print the plan (groups first, then tags); apply nothing ──────
    if dry_run {
        println!("[dry-run] Tag group plan:");
        if groups_to_create.is_empty() && groups_to_update.is_empty() && groups_to_delete.is_empty()
        {
            println!("  (no group changes)");
        }
        for g in &groups_to_create {
            println!(
                "  + create group: {} (tags: [{}])",
                g.name,
                g.tags.join(", ")
            );
        }
        for (g, _id) in &groups_to_update {
            println!(
                "  ~ update group: {} (tags: [{}])",
                g.name,
                g.tags.join(", ")
            );
        }
        for (name, _id) in &groups_to_delete {
            println!("  - delete group: {}", name);
        }

        println!("[dry-run] Tag plan:");
        if tag_plan.is_empty() {
            println!("  (no tag changes)");
        }
        for name in &tag_plan.created_via_group {
            println!("  + create tag: {} (via its tag group)", name);
        }
        for (name, desc) in &tag_plan.set_description {
            println!("  ~ set description: {} ({:?})", name, desc);
        }
        for name in &tag_plan.groupless_missing {
            println!(
                "  ! cannot create tag: {} (Discourse has no create-tag API; add it to a tag group or create it by tagging a topic)",
                name
            );
        }
        for name in &tag_plan.to_delete {
            println!("  - delete tag: {}", name);
        }

        println!("[dry-run] No changes applied.");
        return Ok(());
    }

    // ── Apply, groups first so their tags are materialised ────────────────────
    for g in &groups_to_create {
        let payload = build_tag_group_payload(g, &group_ids_by_name)
            .with_context(|| format!("building payload for tag group '{}'", g.name))?;
        client
            .create_tag_group(&payload)
            .with_context(|| format!("creating tag group '{}'", g.name))?;
        println!("  + created group: {}", g.name);
    }
    for (g, id) in &groups_to_update {
        let payload = build_tag_group_payload(g, &group_ids_by_name)
            .with_context(|| format!("building payload for tag group '{}'", g.name))?;
        client
            .update_tag_group(*id, &payload)
            .with_context(|| format!("updating tag group '{}'", g.name))?;
        println!("  ~ updated group: {}", g.name);
    }

    // Re-read tags: group reconciliation just materialised the group tags.
    let now_existing: BTreeSet<String> = client.list_tags()?.into_iter().map(|t| t.text).collect();
    for name in &tag_plan.created_via_group {
        if now_existing.contains(name) {
            println!("  + created tag: {} (via its tag group)", name);
        }
    }

    // Set descriptions on tags that now exist.
    for (name, desc) in &tag_plan.set_description {
        if now_existing.contains(name) {
            client
                .update_tag(name, Some(desc))
                .with_context(|| format!("setting description on tag '{}'", name))?;
            println!("  ~ set description: {}", name);
        }
    }

    // Prune: delete undesired tags (singular endpoint), then undesired groups.
    for name in &tag_plan.to_delete {
        client
            .delete_tag(name)
            .with_context(|| format!("deleting tag '{}'", name))?;
        println!("  - deleted tag: {}", name);
    }
    for (name, id) in &groups_to_delete {
        client
            .delete_tag_group(*id)
            .with_context(|| format!("deleting tag group '{}'", name))?;
        println!("  - deleted group: {}", name);
    }

    // Any desired tag that belongs to no group and did not already exist could
    // not be created (no admin create-tag endpoint). Report after doing all the
    // achievable work, so the exit code reflects the incomplete apply rather
    // than aborting on the first one.
    if !tag_plan.groupless_missing.is_empty() {
        anyhow::bail!(
            "these tags are in no tag group and do not exist on '{}', and Discourse has no admin create-tag endpoint, so they were not created: {}. Add them to a tag group in the file, or create them by tagging a topic.",
            discourse_name,
            tag_plan.groupless_missing.join(", ")
        );
    }

    println!("Push complete.");
    Ok(())
}

/// Whether a server tag group differs from the file definition. Unspecified
/// file permissions are intentionally ignored, preserving Discourse defaults.
fn tag_group_needs_update(
    desired: &TagGroupEntry,
    server: &TagGroupInfo,
    group_names_by_id: &BTreeMap<u64, String>,
) -> Result<bool> {
    let mut server_tags = server.tag_names.clone();
    server_tags.sort();
    let mut desired_tags = desired.tags.clone();
    desired_tags.sort();
    if server_tags != desired_tags
        || server.one_per_topic != desired.one_per_topic
        || server.parent_tag_name != desired.parent_tag
    {
        return Ok(true);
    }

    let Some(desired_permissions) = &desired.permissions else {
        return Ok(false);
    };
    let server_permissions = server
        .permissions
        .as_ref()
        .map(|permissions| parse_tag_group_permissions(permissions, group_names_by_id))
        .transpose()?
        .flatten();
    Ok(server_permissions.as_ref() != Some(desired_permissions))
}

fn build_tag_group_payload(
    entry: &TagGroupEntry,
    group_ids_by_name: &BTreeMap<String, u64>,
) -> Result<serde_json::Value> {
    let mut group = serde_json::Map::new();
    group.insert("name".to_string(), serde_json::json!(entry.name));
    group.insert("tag_names".to_string(), serde_json::json!(entry.tags));
    group.insert(
        "one_per_topic".to_string(),
        serde_json::json!(entry.one_per_topic),
    );
    if let Some(parent) = &entry.parent_tag {
        group.insert("parent_tag_name".to_string(), serde_json::json!([parent]));
    }
    if let Some(perms) = &entry.permissions {
        let mut perm_map = BTreeMap::new();
        for (group_name, level) in perms {
            let group_id = group_ids_by_name.get(group_name).ok_or_else(|| {
                anyhow::anyhow!("tag-group permission references unknown group '{group_name}'")
            })?;
            perm_map.insert(group_id.to_string(), permission_level(level)?);
        }
        group.insert("permissions".to_string(), serde_json::json!(perm_map));
    }
    Ok(serde_json::json!({ "tag_group": group }))
}

#[cfg(test)]
mod tests {
    use super::{
        TagGroupEntry, build_tag_group_payload, next_tags_after_apply, next_tags_after_remove,
        parse_tag_group_permissions, plan_tags, tag_group_needs_update, validate_rename_names,
    };
    use crate::api::TagGroupInfo;
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    fn explicit(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
        pairs
            .iter()
            .map(|(n, d)| (n.to_string(), d.map(|s| s.to_string())))
            .collect()
    }

    fn permissions(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(group, level)| (group.to_string(), level.to_string()))
            .collect()
    }

    fn tag_group(permissions: Option<BTreeMap<String, String>>) -> TagGroupEntry {
        TagGroupEntry {
            name: "Role".to_string(),
            description: None,
            one_per_topic: false,
            parent_tag: None,
            permissions,
            tags: s(&["guitarist"]),
        }
    }

    #[test]
    fn apply_adds_when_absent() {
        let got = next_tags_after_apply(&s(&["foo", "bar"]), "baz").unwrap();
        assert_eq!(got, s(&["foo", "bar", "baz"]));
    }

    #[test]
    fn apply_returns_none_when_already_present() {
        assert!(next_tags_after_apply(&s(&["foo", "bar"]), "foo").is_none());
    }

    #[test]
    fn apply_to_empty_list_works() {
        let got = next_tags_after_apply(&s(&[]), "first").unwrap();
        assert_eq!(got, s(&["first"]));
    }

    #[test]
    fn remove_drops_present_tag() {
        let got = next_tags_after_remove(&s(&["foo", "bar", "baz"]), "bar").unwrap();
        assert_eq!(got, s(&["foo", "baz"]));
    }

    #[test]
    fn remove_returns_none_when_absent() {
        assert!(next_tags_after_remove(&s(&["foo", "bar"]), "baz").is_none());
    }

    #[test]
    fn remove_last_tag_leaves_empty_list() {
        let got = next_tags_after_remove(&s(&["only"]), "only").unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn apply_is_case_sensitive() {
        // Discourse tags are lowercase canonically, but we don't normalize —
        // the API returns and accepts whatever is sent. Document the behaviour.
        let got = next_tags_after_apply(&s(&["Foo"]), "foo").unwrap();
        assert_eq!(got, s(&["Foo", "foo"]));
    }

    #[test]
    fn rename_trims_inputs() {
        let (old, new) = validate_rename_names("  foo  ", "  bar  ").unwrap();
        assert_eq!(old, "foo");
        assert_eq!(new, "bar");
    }

    #[test]
    fn rename_rejects_empty_old() {
        assert!(validate_rename_names("", "bar").is_err());
        assert!(validate_rename_names("   ", "bar").is_err());
    }

    #[test]
    fn rename_rejects_empty_new() {
        assert!(validate_rename_names("foo", "").is_err());
        assert!(validate_rename_names("foo", "   ").is_err());
    }

    #[test]
    fn rename_rejects_identical_names() {
        let err = validate_rename_names("foo", "foo").unwrap_err();
        assert!(err.to_string().contains("identical"));
    }

    #[test]
    fn rename_rejects_whitespace_in_new_name() {
        let err = validate_rename_names("foo", "bar baz").unwrap_err();
        assert!(err.to_string().contains("whitespace"));
    }

    #[test]
    fn rename_treats_trim_only_difference_as_identical() {
        // After trimming, "foo " and "foo" are the same.
        let err = validate_rename_names("foo ", "foo").unwrap_err();
        assert!(err.to_string().contains("identical"));
    }

    // ── plan_tags (bug #2: create-tag ordering) ───────────────────────────────

    #[test]
    fn plan_group_tag_absent_is_created_via_group() {
        // A tag named in a group but missing from the server is materialised by
        // group reconciliation, not a standalone (impossible) create.
        let p = plan_tags(
            &explicit(&[]),
            &set(&["acoustic", "jazz"]),
            &set(&["jazz"]),
            false,
        );
        assert_eq!(p.created_via_group, s(&["acoustic"]));
        assert!(p.groupless_missing.is_empty());
    }

    #[test]
    fn plan_groupless_missing_is_reported_not_created() {
        // An explicit tag in no group and absent from the server cannot be
        // created by any API - it must be surfaced, not silently attempted.
        let p = plan_tags(&explicit(&[("orphan", None)]), &set(&[]), &set(&[]), false);
        assert_eq!(p.groupless_missing, s(&["orphan"]));
        assert!(p.created_via_group.is_empty());
        assert!(p.set_description.is_empty());
    }

    #[test]
    fn plan_sets_description_for_group_created_tag() {
        // In a group (so it will exist) + has a description → set it after group
        // reconciliation.
        let p = plan_tags(
            &explicit(&[("jazz", Some("Jazz music"))]),
            &set(&["jazz"]),
            &set(&[]),
            false,
        );
        assert_eq!(
            p.set_description,
            vec![("jazz".to_string(), "Jazz music".to_string())]
        );
        assert!(p.groupless_missing.is_empty());
    }

    #[test]
    fn plan_no_description_set_for_uncreatable_orphan() {
        // An orphan can't be created, so its description can't be set either.
        let p = plan_tags(
            &explicit(&[("orphan", Some("x"))]),
            &set(&[]),
            &set(&[]),
            false,
        );
        assert!(p.set_description.is_empty());
        assert_eq!(p.groupless_missing, s(&["orphan"]));
    }

    #[test]
    fn plan_sets_description_for_existing_server_tag() {
        let p = plan_tags(
            &explicit(&[("rock", Some("Rock"))]),
            &set(&[]),
            &set(&["rock"]),
            false,
        );
        assert_eq!(
            p.set_description,
            vec![("rock".to_string(), "Rock".to_string())]
        );
        assert!(p.groupless_missing.is_empty());
    }

    #[test]
    fn plan_prune_deletes_undesired_server_tags() {
        let p = plan_tags(
            &explicit(&[("keep", None)]),
            &set(&[]),
            &set(&["keep", "old"]),
            true,
        );
        assert_eq!(p.to_delete, s(&["old"]));
    }

    #[test]
    fn plan_without_prune_deletes_nothing() {
        let p = plan_tags(&explicit(&[]), &set(&[]), &set(&["old"]), false);
        assert!(p.to_delete.is_empty());
    }

    #[test]
    fn plan_group_tag_still_desired_is_not_pruned() {
        // A tag desired only via a group must not be pruned just because it's
        // absent from the explicit list.
        let p = plan_tags(&explicit(&[]), &set(&["jazz"]), &set(&["jazz"]), true);
        assert!(p.to_delete.is_empty());
    }

    #[test]
    fn plan_no_group_access_makes_group_only_explicit_tags_orphans() {
        // When the admin group endpoint is unreachable the caller passes empty
        // group_tags; an explicit tag that only lived in a group can no longer
        // be created and is reported.
        let p = plan_tags(&explicit(&[("jazz", None)]), &set(&[]), &set(&[]), false);
        assert_eq!(p.groupless_missing, s(&["jazz"]));
    }

    #[test]
    fn pull_converts_permission_group_ids_and_labels_create_post() {
        let group_names_by_id = [(0, "everyone"), (12, "members"), (42, "staff")]
            .into_iter()
            .map(|(id, name)| (id, name.to_string()))
            .collect();
        let parsed =
            parse_tag_group_permissions(&json!({"0": 1, "12": 2, "42": 3}), &group_names_by_id)
                .unwrap()
                .unwrap();

        assert_eq!(
            parsed,
            permissions(&[
                ("everyone", "full"),
                ("members", "create_post"),
                ("staff", "readonly"),
            ])
        );
    }

    #[test]
    fn push_converts_permission_names_to_group_ids() {
        let group_ids_by_name = [("everyone", 0), ("members", 12), ("staff", 42)]
            .into_iter()
            .map(|(name, id)| (name.to_string(), id))
            .collect();
        let entry = tag_group(Some(permissions(&[
            ("everyone", "full"),
            ("members", "create_post"),
            ("staff", "readonly"),
        ])));

        let payload = build_tag_group_payload(&entry, &group_ids_by_name).unwrap();
        assert_eq!(
            payload["tag_group"]["permissions"],
            json!({"0": 1, "12": 2, "42": 3})
        );
    }

    #[test]
    fn permission_only_change_requires_tag_group_update() {
        let group_names_by_id = [(0, "everyone")]
            .into_iter()
            .map(|(id, name)| (id, name.to_string()))
            .collect();
        let desired = tag_group(Some(permissions(&[("everyone", "full")])));
        let mut server = TagGroupInfo {
            id: 9,
            name: "Role".to_string(),
            tag_names: s(&["guitarist"]),
            one_per_topic: false,
            parent_tag_name: None,
            permissions: Some(json!({"0": 1})),
        };

        assert!(!tag_group_needs_update(&desired, &server, &group_names_by_id).unwrap());
        server.permissions = Some(json!({"0": 3}));
        assert!(tag_group_needs_update(&desired, &server, &group_names_by_id).unwrap());
    }

    #[test]
    fn push_rejects_unknown_permission_group() {
        let entry = tag_group(Some(permissions(&[("missing", "full")])));
        let err = build_tag_group_payload(&entry, &BTreeMap::new()).unwrap_err();
        assert!(err.to_string().contains("unknown group 'missing'"));
    }
}

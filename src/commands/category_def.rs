// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Category *definition* sync: `category def pull/push` (declarative, all
//! categories) and `category show/get/set` (imperative, one field). Distinct
//! from `category pull/push` in `category.rs`, which sync topic *content*.
//!
//! See `spec/commands/category-definition-sync.md`.

use crate::api::{CategoryDefinition, DiscourseClient};
use crate::cli::ListFormat;
use crate::commands::common::{emit_result, ensure_api_credentials, not_found, select_discourse};
use crate::config::Config;
use crate::utils::atomic_write;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

/// The definition fields a user may `get`/`set` and that appear in the file.
const VALID_FIELDS: &[&str] = &[
    "name",
    "slug",
    "color",
    "text_color",
    "style_type",
    "icon",
    "emoji",
    "position",
    "parent",
    "read_restricted",
    "description",
    "topic_template",
    "topic_title_placeholder",
    "permissions",
    "allowed_tags",
    "allowed_tag_groups",
    "minimum_required_tags",
    "required_tag_groups",
    "category_types",
    "custom_fields",
    "sort_order",
    "default_view",
    "subcategory_list_style",
    "num_featured_topics",
    "show_subcategory_list",
];
const CATEGORIES_FILE_VERSION: u32 = 2;

// ─── File schema ──────────────────────────────────────────────────────────────

/// The on-disk `categories.yaml` (or `.json`) document.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CategoriesFile {
    pub version: u32,
    #[serde(default)]
    pub categories: Vec<CategoryDefEntry>,
}

/// One category's definition in the file. Every field beyond `name` is optional;
/// an omitted field is left untouched on push.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CategoryDefEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,
    /// Category style: `square` (colour swatch, the default), `icon`, or `emoji`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_type: Option<String>,
    /// FontAwesome icon name, used when `style_type` is `icon`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Emoji shortcode (without colons), used when `style_type` is `emoji`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    /// Parent category slug, unambiguous name, or existing ID. `Some(None)`
    /// explicitly moves the category to the top level; `None` leaves its current
    /// parent untouched.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_parent"
    )]
    pub parent: Option<Option<String>>,
    #[serde(skip)]
    server_parent_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_restricted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_template: Option<String>,
    /// Placeholder text shown in the topic-title field when composing a new
    /// topic in this category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_title_placeholder: Option<String>,
    /// group_name -> level (`full` | `create_post` | `readonly`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tag_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_required_tags: Option<u64>,
    /// Tag groups whose tags are required on new topics in this category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_tag_groups: Option<Vec<RequiredTagGroupEntry>>,
    /// Enabled category type IDs beyond the built-in `discussion` type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_types: Option<Vec<String>>,
    /// Complete category custom-field map with stable scalar values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_fields: Option<BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_view: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subcategory_list_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_featured_topics: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_subcategory_list: Option<bool>,
}

/// One required tag-group rule in the portable category-definition file.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequiredTagGroupEntry {
    pub name: String,
    pub min_count: u64,
}

fn deserialize_present_parent<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

// ─── Permission level <-> label ───────────────────────────────────────────────

fn perm_label(t: u8) -> &'static str {
    match t {
        2 => "create_post",
        3 => "readonly",
        _ => "full",
    }
}

fn perm_type(label: &str) -> Result<u8> {
    match label.trim() {
        "full" => Ok(1),
        "create_post" => Ok(2),
        "readonly" => Ok(3),
        other => Err(anyhow!(
            "invalid permission level '{}' (expected full|create_post|readonly)",
            other
        )),
    }
}

/// Discourse strips terminal line endings from category descriptions. Normalise
/// them at the file boundary so a YAML literal block remains idempotent.
fn normalize_description(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .map(|value| value.trim_end_matches(['\r', '\n']).to_string())
}

fn validate_style_type(value: &str) -> Result<&str> {
    let value = value.trim();
    if matches!(value, "square" | "icon" | "emoji") {
        Ok(value)
    } else {
        Err(anyhow!(
            "invalid category style_type '{}' (expected square|icon|emoji)",
            value
        ))
    }
}

fn validate_style_state(
    category: &str,
    style_type: Option<&str>,
    icon: Option<&str>,
    emoji: Option<&str>,
) -> Result<()> {
    match validate_style_type(style_type.unwrap_or("square"))? {
        "icon" if icon.is_none_or(|value| value.trim().is_empty()) => Err(anyhow!(
            "category '{}' uses style_type 'icon' but has no icon; set icon before selecting the icon style",
            category
        )),
        "emoji" if emoji.is_none_or(|value| value.trim().is_empty()) => Err(anyhow!(
            "category '{}' uses style_type 'emoji' but has no emoji; set emoji before selecting the emoji style",
            category
        )),
        _ => Ok(()),
    }
}

// ─── API model <-> file entry ─────────────────────────────────────────────────

/// Convert a server definition to a file entry. `id_to_slug` resolves the parent
/// id to its slug.
fn def_to_entry(def: &CategoryDefinition, id_to_slug: &BTreeMap<u64, String>) -> CategoryDefEntry {
    let permissions = def.group_permissions.as_ref().and_then(|perms| {
        let map: BTreeMap<String, String> = perms
            .iter()
            .filter_map(|p| {
                p.group_name
                    .as_ref()
                    .map(|g| (g.clone(), perm_label(p.permission_type).to_string()))
            })
            .collect();
        (!map.is_empty()).then_some(map)
    });
    let parent = Some(
        def.parent_category_id
            .and_then(|pid| id_to_slug.get(&pid).cloned()),
    );
    let nonempty = |s: &Option<String>| s.clone().filter(|v| !v.is_empty());
    let nonempty_list = |v: &Option<Vec<String>>| v.clone().filter(|l| !l.is_empty());
    let description = nonempty(&def.description_text).or_else(|| nonempty(&def.description));

    CategoryDefEntry {
        name: def.name.clone(),
        id: def.id,
        slug: nonempty(&def.slug),
        color: nonempty(&def.color),
        text_color: nonempty(&def.text_color),
        style_type: nonempty(&def.style_type),
        icon: nonempty(&def.icon),
        emoji: nonempty(&def.emoji),
        position: def.position,
        parent,
        server_parent_id: def.parent_category_id,
        read_restricted: def.read_restricted,
        // Prefer the plain-text description over the cooked HTML `description`
        // so pull -> push -> pull is idempotent (see CategoryDefinition).
        description: normalize_description(&description),
        topic_template: nonempty(&def.topic_template),
        topic_title_placeholder: nonempty(&def.topic_title_placeholder),
        permissions,
        allowed_tags: nonempty_list(&def.allowed_tags),
        allowed_tag_groups: nonempty_list(&def.allowed_tag_groups),
        minimum_required_tags: def.minimum_required_tags.filter(|n| *n > 0),
        required_tag_groups: def.required_tag_groups.as_ref().and_then(|groups| {
            let groups: Vec<RequiredTagGroupEntry> = groups
                .iter()
                .filter_map(|group| {
                    group.name.as_ref().map(|name| RequiredTagGroupEntry {
                        name: name.clone(),
                        min_count: group.min_count.unwrap_or_default(),
                    })
                })
                .collect();
            (!groups.is_empty()).then_some(groups)
        }),
        category_types: def.category_types.as_ref().and_then(|types| {
            let types: Vec<String> = types
                .keys()
                .filter(|category_type| category_type.as_str() != "discussion")
                .cloned()
                .collect();
            (!types.is_empty()).then_some(types)
        }),
        custom_fields: def
            .custom_fields
            .clone()
            .filter(|fields| !fields.is_empty()),
        sort_order: nonempty(&def.sort_order),
        default_view: nonempty(&def.default_view),
        subcategory_list_style: nonempty(&def.subcategory_list_style),
        num_featured_topics: def.num_featured_topics,
        show_subcategory_list: def.show_subcategory_list,
    }
}

/// Enrich category-list definitions with the complete custom-field maps exposed
/// by the per-category endpoint. The list endpoint may only preload a subset.
fn enrich_custom_fields(client: &DiscourseClient, defs: &mut [CategoryDefinition]) -> Result<()> {
    for def in defs {
        let id = def
            .id
            .ok_or_else(|| anyhow!("category definition is missing its id"))?;
        def.custom_fields = client.fetch_category_definition(id)?.custom_fields;
    }
    Ok(())
}

/// Build a JSON merge patch for category custom fields. Discourse removes a
/// custom field when it receives a null value for that key.
fn custom_fields_patch(
    desired: &BTreeMap<String, Value>,
    current: Option<&BTreeMap<String, Value>>,
) -> BTreeMap<String, Value> {
    let mut patch = BTreeMap::new();
    let current = current.cloned().unwrap_or_default();
    for (key, value) in desired {
        if current.get(key) != Some(value) {
            patch.insert(key.clone(), value.clone());
        }
    }
    for key in current.keys() {
        if !desired.contains_key(key) {
            patch.insert(key.clone(), Value::Null);
        }
    }
    patch
}

/// Arbitrary custom-field objects and arrays are stringified by Discourse's
/// category endpoint. Refuse them rather than create a non-idempotent file.
fn validate_custom_fields(fields: &BTreeMap<String, Value>) -> Result<()> {
    for (key, value) in fields {
        if !matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_)) {
            return Err(anyhow!(
                "custom field '{}' must have a string, number, or boolean value",
                key
            ));
        }
    }
    Ok(())
}

fn id_to_slug_map(defs: &[CategoryDefinition]) -> BTreeMap<u64, String> {
    defs.iter()
        .filter_map(|d| match (d.id, &d.slug) {
            (Some(id), Some(slug)) => Some((id, slug.clone())),
            _ => None,
        })
        .collect()
}

fn slug_to_ids_map(defs: &[CategoryDefinition]) -> BTreeMap<String, Vec<u64>> {
    let mut slugs = BTreeMap::<String, Vec<u64>>::new();
    for def in defs {
        if let (Some(slug), Some(id)) = (&def.slug, def.id) {
            slugs.entry(slug.clone()).or_default().push(id);
        }
    }
    slugs
}

fn name_to_ids_map(defs: &[CategoryDefinition]) -> BTreeMap<String, Vec<u64>> {
    let mut names = BTreeMap::<String, Vec<u64>>::new();
    for def in defs {
        if let Some(id) = def.id {
            names.entry(def.name.clone()).or_default().push(id);
        }
    }
    names
}

/// Resolve a `parent` reference against the server's current categories,
/// trying slug first (stable, preferred) then name.
fn resolve_parent_id(
    parent: &str,
    slug_to_ids: &BTreeMap<String, Vec<u64>>,
    name_to_ids: &BTreeMap<String, Vec<u64>>,
) -> Result<Option<u64>> {
    if let Ok(id) = parent.parse::<u64>()
        && (slug_to_ids.values().any(|ids| ids.contains(&id))
            || name_to_ids.values().any(|ids| ids.contains(&id)))
    {
        return Ok(Some(id));
    }
    match slug_to_ids.get(parent).map(Vec::as_slice) {
        Some([id]) => return Ok(Some(*id)),
        Some(ids) => {
            return Err(anyhow!(
                "parent category slug '{}' is ambiguous (matches {} categories); use an unambiguous name",
                parent,
                ids.len()
            ));
        }
        None => {}
    }
    match name_to_ids.get(parent).map(Vec::as_slice) {
        None => Ok(None),
        Some([id]) => Ok(Some(*id)),
        Some(ids) => Err(anyhow!(
            "parent category name '{}' is ambiguous (matches {} categories); use an unambiguous slug",
            parent,
            ids.len()
        )),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParentTarget {
    Server(u64),
    File(usize),
}

struct FileIndex<'a> {
    by_slug: HashMap<&'a str, Vec<usize>>,
    by_name: HashMap<&'a str, Vec<usize>>,
}

impl<'a> FileIndex<'a> {
    fn build(file: &'a [CategoryDefEntry]) -> Self {
        let mut by_slug = HashMap::<&str, Vec<usize>>::new();
        let mut by_name = HashMap::<&str, Vec<usize>>::new();
        for (index, entry) in file.iter().enumerate() {
            if let Some(slug) = &entry.slug {
                by_slug.entry(slug).or_default().push(index);
            }
            by_name.entry(&entry.name).or_default().push(index);
        }
        Self { by_slug, by_name }
    }
}

fn unique_file_parent(
    matches: Option<&Vec<usize>>,
    parent: &str,
    field: &str,
    current_parent_id: Option<u64>,
    plan: &[DefAction],
) -> Result<Option<usize>> {
    match matches.map(Vec::as_slice) {
        None => Ok(None),
        Some([index]) => Ok(Some(*index)),
        Some(indices)
            if current_parent_id.is_some()
                && indices
                    .iter()
                    .filter(|index| plan[**index].server_id == current_parent_id)
                    .count()
                    == 1 =>
        {
            Ok(indices
                .iter()
                .find(|index| plan[**index].server_id == current_parent_id)
                .copied())
        }
        Some(indices) => Err(anyhow!(
            "parent category {} '{}' is ambiguous (matches {} entries in this file)",
            field,
            parent,
            indices.len()
        )),
    }
}

fn unique_server_parent(
    matches: Option<&Vec<&CategoryDefEntry>>,
    parent: &str,
    field: &str,
) -> Result<Option<u64>> {
    match matches.map(Vec::as_slice) {
        None => Ok(None),
        Some([entry]) => entry
            .id
            .map(Some)
            .ok_or_else(|| anyhow!("internal: server category without an id")),
        Some(entries) => Err(anyhow!(
            "parent category {} '{}' is ambiguous (matches {} server categories)",
            field,
            parent,
            entries.len()
        )),
    }
}

/// Resolve every explicit parent once, before writes. Same-file desired aliases
/// take precedence so children can refer to a parent whose name or slug is being
/// changed in this push.
fn resolve_parent_targets(
    file: &[CategoryDefEntry],
    server: &[CategoryDefEntry],
    plan: &[DefAction],
) -> Result<Vec<Option<ParentTarget>>> {
    let file_index = FileIndex::build(file);
    let server_index = ServerIndex::build(server);
    let server_to_file: HashMap<u64, usize> = plan
        .iter()
        .enumerate()
        .filter_map(|(index, action)| action.server_id.map(|id| (id, index)))
        .collect();
    let mut targets = Vec::with_capacity(file.len());
    let mut invalid = Vec::new();

    for (entry_index, entry) in file.iter().enumerate() {
        let parent = match &entry.parent {
            Some(Some(parent)) => parent,
            None | Some(None) => {
                targets.push(None);
                continue;
            }
        };
        let current_parent_id = plan[entry_index].server_id.and_then(|id| {
            server
                .iter()
                .find(|category| category.id == Some(id))
                .and_then(|category| category.server_parent_id)
        });

        let resolved = (|| -> Result<Option<ParentTarget>> {
            if let Ok(id) = parent.parse::<u64>()
                && server_index.by_id.contains_key(&id)
            {
                return Ok(Some(
                    server_to_file
                        .get(&id)
                        .copied()
                        .map_or(ParentTarget::Server(id), ParentTarget::File),
                ));
            }
            if let Some(index) = unique_file_parent(
                file_index.by_slug.get(parent.as_str()),
                parent,
                "slug",
                current_parent_id,
                plan,
            )? {
                return Ok(Some(ParentTarget::File(index)));
            }
            if let Some(id) =
                unique_server_parent(server_index.by_slug.get(parent.as_str()), parent, "slug")?
            {
                return Ok(Some(
                    server_to_file
                        .get(&id)
                        .copied()
                        .map_or(ParentTarget::Server(id), ParentTarget::File),
                ));
            }
            if let Some(index) = unique_file_parent(
                file_index.by_name.get(parent.as_str()),
                parent,
                "name",
                current_parent_id,
                plan,
            )? {
                return Ok(Some(ParentTarget::File(index)));
            }
            if let Some(id) =
                unique_server_parent(server_index.by_name.get(parent.as_str()), parent, "name")?
            {
                return Ok(Some(
                    server_to_file
                        .get(&id)
                        .copied()
                        .map_or(ParentTarget::Server(id), ParentTarget::File),
                ));
            }
            Ok(None)
        })();

        match resolved {
            Ok(Some(target)) => targets.push(Some(target)),
            Ok(None) => {
                invalid.push(format!(
                    "'{}' -> parent '{}' was not found by slug or name on the server or in this file",
                    entry.name, parent
                ));
                targets.push(None);
            }
            Err(error) => {
                invalid.push(format!("'{}' -> {error}", entry.name));
                targets.push(None);
            }
        }
    }

    if invalid.is_empty() {
        Ok(targets)
    } else {
        Err(anyhow!("invalid parent categories: {}", invalid.join(", ")))
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
enum CategoryNode {
    Server(u64),
    File(usize),
}

fn file_node(index: usize, plan: &[DefAction]) -> Result<CategoryNode> {
    match plan[index].server_id {
        Some(id) => Ok(CategoryNode::Server(id)),
        None if plan[index].kind == DefActionKind::Create => Ok(CategoryNode::File(index)),
        None => Err(anyhow!("internal: planned category has no identity")),
    }
}

fn target_node(target: ParentTarget, plan: &[DefAction]) -> Result<CategoryNode> {
    match target {
        ParentTarget::Server(id) => Ok(CategoryNode::Server(id)),
        ParentTarget::File(index) => file_node(index, plan),
    }
}

/// Validate the complete hierarchy that would exist after the push, including
/// current parent edges for server categories omitted from the file.
fn validate_hierarchy(
    file: &[CategoryDefEntry],
    defs: &[CategoryDefinition],
    plan: &[DefAction],
    targets: &[Option<ParentTarget>],
) -> Result<()> {
    let mut parents = HashMap::<CategoryNode, Option<CategoryNode>>::new();
    for def in defs {
        if let Some(id) = def.id {
            parents.insert(
                CategoryNode::Server(id),
                def.parent_category_id.map(CategoryNode::Server),
            );
        }
    }
    for (index, entry) in file.iter().enumerate() {
        let node = file_node(index, plan)?;
        if entry.parent.is_some() || plan[index].kind == DefActionKind::Create {
            let parent = targets[index]
                .map(|target| target_node(target, plan))
                .transpose()?;
            parents.insert(node, parent);
        }
    }

    let mut finished = HashMap::<CategoryNode, bool>::new();
    let mut starts: Vec<_> = parents.keys().copied().collect();
    starts.sort();
    for start in starts {
        if finished.contains_key(&start) {
            continue;
        }
        let mut path = Vec::new();
        let mut positions = HashMap::new();
        let mut current = Some(start);
        while let Some(node) = current {
            if finished.contains_key(&node) {
                break;
            }
            if let Some(&position) = positions.get(&node) {
                let names: Vec<&str> = path[position..]
                    .iter()
                    .map(|cycle_node| match cycle_node {
                        CategoryNode::File(index) => file[*index].name.as_str(),
                        CategoryNode::Server(id) => file
                            .iter()
                            .zip(plan)
                            .find(|(_, action)| action.server_id == Some(*id))
                            .map_or_else(
                                || {
                                    defs.iter()
                                        .find(|def| def.id == Some(*id))
                                        .map_or("<unknown>", |def| def.name.as_str())
                                },
                                |(entry, _)| entry.name.as_str(),
                            ),
                    })
                    .collect();
                return Err(anyhow!(
                    "circular parent reference among categories: {}",
                    names.join(", ")
                ));
            }
            positions.insert(node, path.len());
            path.push(node);
            current = parents.get(&node).copied().flatten();
        }
        for node in path {
            finished.insert(node, true);
        }
    }
    Ok(())
}

/// Refuse duplicate desired identities that Discourse would reject after an
/// earlier entry has already been applied.
fn validate_desired_identities(
    file: &[CategoryDefEntry],
    server: &[CategoryDefEntry],
    plan: &[DefAction],
    targets: &[Option<ParentTarget>],
) -> Result<()> {
    #[derive(Clone)]
    struct Identity {
        label: String,
        parent: Option<CategoryNode>,
        name: String,
        slug: Option<String>,
    }

    let mut identities = HashMap::<CategoryNode, Identity>::new();
    for category in server {
        if let Some(id) = category.id {
            identities.insert(
                CategoryNode::Server(id),
                Identity {
                    label: category.name.clone(),
                    parent: category.server_parent_id.map(CategoryNode::Server),
                    name: category.name.clone(),
                    slug: category.slug.clone(),
                },
            );
        }
    }
    for (index, entry) in file.iter().enumerate() {
        let node = file_node(index, plan)?;
        let current = identities.get(&node).cloned();
        let parent = if entry.parent.is_some() || plan[index].kind == DefActionKind::Create {
            targets[index]
                .map(|target| target_node(target, plan))
                .transpose()?
        } else {
            current.as_ref().and_then(|identity| identity.parent)
        };
        identities.insert(
            node,
            Identity {
                label: entry.name.clone(),
                parent,
                name: entry.name.clone(),
                slug: entry
                    .slug
                    .clone()
                    .or_else(|| current.as_ref().and_then(|identity| identity.slug.clone())),
            },
        );
    }

    let mut names = HashMap::<(Option<CategoryNode>, String), CategoryNode>::new();
    let mut slugs = HashMap::<(Option<CategoryNode>, String), CategoryNode>::new();
    for (node, identity) in &identities {
        let name_key = (identity.parent, identity.name.to_lowercase());
        if let Some(previous) = names.insert(name_key, *node)
            && previous != *node
        {
            return Err(anyhow!(
                "categories '{}' and '{}' would have duplicate name '{}' under the same parent",
                identities[&previous].label,
                identity.label,
                identity.name
            ));
        }
        if let Some(slug) = &identity.slug
            && let Some(previous) = slugs.insert((identity.parent, slug.to_lowercase()), *node)
            && previous != *node
        {
            return Err(anyhow!(
                "categories '{}' and '{}' would have duplicate slug '{}' under the same parent",
                identities[&previous].label,
                identity.label,
                slug
            ));
        }
    }
    Ok(())
}

/// Parent references are compared by resolved identity, not by their textual
/// slug/name/ID form, so equivalent aliases remain idempotent.
fn reconcile_parent_changes(
    file: &[CategoryDefEntry],
    server: &[CategoryDefEntry],
    plan: &mut [DefAction],
    targets: &[Option<ParentTarget>],
) {
    for (index, entry) in file.iter().enumerate() {
        if entry.parent.is_none() || plan[index].kind == DefActionKind::Create {
            continue;
        }
        let current_parent = plan[index].server_id.and_then(|id| {
            server
                .iter()
                .find(|category| category.id == Some(id))
                .and_then(|category| category.server_parent_id)
        });
        let desired_parent = match targets[index] {
            None => None,
            Some(ParentTarget::Server(id)) => Some(id),
            Some(ParentTarget::File(parent)) => plan[parent].server_id,
        };
        let same_parent = desired_parent.is_some() && desired_parent == current_parent
            || desired_parent.is_none()
                && current_parent.is_none()
                && !matches!(targets[index], Some(ParentTarget::File(_)));

        if same_parent {
            plan[index]
                .changed_fields
                .retain(|field| *field != "parent");
        } else if !plan[index].changed_fields.contains(&"parent") {
            plan[index].changed_fields.push("parent");
        }
        plan[index].kind = if plan[index].changed_fields.is_empty() {
            DefActionKind::Unchanged
        } else {
            DefActionKind::Update
        };
    }
}

/// Return file indices in dependency order. Parents are applied before children,
/// and categories release occupied names/slugs before another entry claims them.
fn order_for_push(
    file: &[CategoryDefEntry],
    server: &[CategoryDefEntry],
    plan: &[DefAction],
    targets: &[Option<ParentTarget>],
) -> Result<Vec<usize>> {
    let server_to_file: HashMap<u64, usize> = plan
        .iter()
        .enumerate()
        .filter_map(|(index, action)| action.server_id.map(|id| (id, index)))
        .collect();
    let mut depends_on = vec![Vec::<usize>::new(); file.len()];
    for (index, target) in targets.iter().enumerate() {
        if let Some(ParentTarget::File(parent)) = target
            && plan[*parent].kind == DefActionKind::Create
        {
            depends_on[index].push(*parent);
        }
    }
    for (index, entry) in file.iter().enumerate() {
        if plan[index].kind == DefActionKind::Unchanged {
            continue;
        }
        let current = plan[index]
            .server_id
            .and_then(|id| server.iter().find(|category| category.id == Some(id)));
        let desired_parent = if entry.parent.is_some() || plan[index].kind == DefActionKind::Create
        {
            targets[index]
                .map(|target| target_node(target, plan))
                .transpose()?
        } else {
            current
                .and_then(|category| category.server_parent_id)
                .map(CategoryNode::Server)
        };
        let desired_name = entry.name.to_lowercase();
        let desired_slug = entry
            .slug
            .as_deref()
            .or_else(|| current.and_then(|category| category.slug.as_deref()))
            .map(str::to_lowercase);

        for occupied in server {
            let Some(occupied_id) = occupied.id else {
                continue;
            };
            if plan[index].server_id == Some(occupied_id)
                || occupied.server_parent_id.map(CategoryNode::Server) != desired_parent
            {
                continue;
            }
            let occupies_name = occupied.name.to_lowercase() == desired_name;
            let occupies_slug = desired_slug.as_ref().is_some_and(|slug| {
                occupied
                    .slug
                    .as_ref()
                    .is_some_and(|occupied| occupied.to_lowercase() == *slug)
            });
            if (occupies_name || occupies_slug)
                && let Some(release) = server_to_file.get(&occupied_id).copied()
                && !depends_on[index].contains(&release)
            {
                depends_on[index].push(release);
            }
        }
    }

    let mut current_parents = HashMap::<CategoryNode, Option<CategoryNode>>::new();
    for category in server {
        if let Some(id) = category.id {
            current_parents.insert(
                CategoryNode::Server(id),
                category.server_parent_id.map(CategoryNode::Server),
            );
        }
    }
    let mut placed = vec![false; file.len()];
    let mut order = Vec::with_capacity(file.len());
    while order.len() < file.len() {
        let mut progressed = false;
        for i in 0..file.len() {
            if placed[i] {
                continue;
            }
            let node = file_node(i, plan)?;
            let desired_parent =
                if file[i].parent.is_some() || plan[i].kind == DefActionKind::Create {
                    targets[i]
                        .map(|target| target_node(target, plan))
                        .transpose()?
                } else {
                    current_parents.get(&node).copied().flatten()
                };
            let mut ancestor = desired_parent;
            let mut seen = HashSet::new();
            let mut creates_cycle = false;
            while let Some(parent) = ancestor {
                if parent == node {
                    creates_cycle = true;
                    break;
                }
                if !seen.insert(parent) {
                    creates_cycle = true;
                    break;
                }
                ancestor = current_parents.get(&parent).copied().flatten();
            }
            let ready =
                !creates_cycle && depends_on[i].iter().all(|dependency| placed[*dependency]);
            if ready {
                placed[i] = true;
                order.push(i);
                if file[i].parent.is_some() || plan[i].kind == DefActionKind::Create {
                    current_parents.insert(node, desired_parent);
                }
                progressed = true;
            }
        }
        if !progressed {
            let stuck: Vec<&str> = (0..file.len())
                .filter(|&i| !placed[i])
                .map(|i| file[i].name.as_str())
                .collect();
            return Err(anyhow!(
                "circular category dependencies among entries in this file: {}",
                stuck.join(", ")
            ));
        }
    }
    Ok(order)
}

/// Build the form params for a whole entry (create or full update).
fn entry_to_params(
    entry: &CategoryDefEntry,
    parent_id: Option<u64>,
) -> Result<Vec<(String, String)>> {
    let mut p: Vec<(String, String)> = vec![("name".to_string(), entry.name.clone())];
    let push_opt = |p: &mut Vec<(String, String)>, key: &str, v: &Option<String>| {
        if let Some(val) = v {
            p.push((key.to_string(), val.clone()));
        }
    };
    push_opt(&mut p, "slug", &entry.slug);
    push_opt(&mut p, "color", &entry.color);
    push_opt(&mut p, "text_color", &entry.text_color);
    if let Some(style_type) = &entry.style_type {
        p.push((
            "style_type".to_string(),
            validate_style_type(style_type)?.to_string(),
        ));
    }
    push_opt(&mut p, "icon", &entry.icon);
    push_opt(&mut p, "emoji", &entry.emoji);
    if let Some(v) = entry.position {
        p.push(("position".to_string(), v.to_string()));
    }
    match &entry.parent {
        Some(Some(_)) => {
            let parent_id =
                parent_id.ok_or_else(|| anyhow!("internal: unresolved category parent"))?;
            p.push(("parent_category_id".to_string(), parent_id.to_string()));
        }
        Some(None) => p.push(("parent_category_id".to_string(), String::new())),
        None => {}
    }
    if let Some(v) = entry.read_restricted {
        p.push(("read_restricted".to_string(), v.to_string()));
    }
    if let Some(description) = normalize_description(&entry.description) {
        p.push(("description".to_string(), description));
    }
    push_opt(&mut p, "topic_template", &entry.topic_template);
    push_opt(
        &mut p,
        "topic_title_placeholder",
        &entry.topic_title_placeholder,
    );
    if let Some(perms) = &entry.permissions {
        for (group, level) in perms {
            p.push((
                format!("permissions[{}]", group),
                perm_type(level)?.to_string(),
            ));
        }
    }
    if let Some(tags) = &entry.allowed_tags {
        for t in tags {
            p.push(("allowed_tags[]".to_string(), t.clone()));
        }
    }
    if let Some(groups) = &entry.allowed_tag_groups {
        for g in groups {
            p.push(("allowed_tag_groups[]".to_string(), g.clone()));
        }
    }
    if let Some(v) = entry.minimum_required_tags {
        p.push(("minimum_required_tags".to_string(), v.to_string()));
    }
    if let Some(groups) = &entry.required_tag_groups {
        if groups.is_empty() {
            p.push(("required_tag_groups[][name]".to_string(), String::new()));
        } else {
            for group in groups {
                p.push((
                    "required_tag_groups[][name]".to_string(),
                    group.name.clone(),
                ));
                p.push((
                    "required_tag_groups[][min_count]".to_string(),
                    group.min_count.to_string(),
                ));
            }
        }
    }
    if let Some(types) = &entry.category_types {
        if types.is_empty() {
            p.push(("category_types[]".to_string(), String::new()));
        } else {
            for category_type in types {
                p.push(("category_types[]".to_string(), category_type.clone()));
            }
        }
    }
    push_opt(&mut p, "sort_order", &entry.sort_order);
    push_opt(&mut p, "default_view", &entry.default_view);
    push_opt(
        &mut p,
        "subcategory_list_style",
        &entry.subcategory_list_style,
    );
    if let Some(v) = entry.num_featured_topics {
        p.push(("num_featured_topics".to_string(), v.to_string()));
    }
    if let Some(v) = entry.show_subcategory_list {
        p.push(("show_subcategory_list".to_string(), v.to_string()));
    }
    Ok(p)
}

// ─── Push planning ────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum DefActionKind {
    Create,
    Update,
    Unchanged,
}

#[derive(Debug, PartialEq)]
struct DefAction {
    name: String,
    kind: DefActionKind,
    server_id: Option<u64>,
    changed_fields: Vec<&'static str>,
    /// A no-`id` file entry that matched nothing: `def push` would CREATE it,
    /// but if the user meant to rename an existing category this would orphan
    /// its topics. Warned in the plan.
    rename_warning: bool,
}

/// Lookup indices over a server category list. Slugs and names map to every
/// match because Discourse scopes their uniqueness to the parent category.
struct ServerIndex<'a> {
    by_id: HashMap<u64, &'a CategoryDefEntry>,
    by_slug: HashMap<&'a str, Vec<&'a CategoryDefEntry>>,
    by_name: HashMap<&'a str, Vec<&'a CategoryDefEntry>>,
}

impl<'a> ServerIndex<'a> {
    fn build(server: &'a [CategoryDefEntry]) -> Self {
        let mut by_id = HashMap::new();
        let mut by_slug = HashMap::<&str, Vec<&CategoryDefEntry>>::new();
        let mut by_name = HashMap::<&str, Vec<&CategoryDefEntry>>::new();
        for s in server {
            if let Some(id) = s.id {
                by_id.entry(id).or_insert(s);
            }
            if let Some(slug) = &s.slug {
                by_slug.entry(slug.as_str()).or_default().push(s);
            }
            by_name.entry(s.name.as_str()).or_default().push(s);
        }
        Self {
            by_id,
            by_slug,
            by_name,
        }
    }
}

fn parent_scope_matches(
    desired: &CategoryDefEntry,
    candidate: &CategoryDefEntry,
    index: &ServerIndex<'_>,
    file: &[CategoryDefEntry],
) -> bool {
    match &desired.parent {
        None => false,
        Some(None) => candidate.server_parent_id.is_none(),
        Some(Some(parent)) => {
            let Some(parent_id) = candidate.server_parent_id else {
                return false;
            };
            if parent.parse::<u64>() == Ok(parent_id) {
                return true;
            }
            let server_alias_matches = index.by_id.get(&parent_id).is_some_and(|actual| {
                actual.slug.as_deref() == Some(parent) || actual.name == *parent
            });
            server_alias_matches
                || file.iter().any(|entry| {
                    entry.id == Some(parent_id)
                        && (entry.slug.as_deref() == Some(parent) || entry.name == *parent)
                })
        }
    }
}

fn select_server_match<'a>(
    desired: &CategoryDefEntry,
    matches: &[&'a CategoryDefEntry],
    field: &str,
    value: &str,
    index: &ServerIndex<'a>,
    file: &[CategoryDefEntry],
) -> Result<Option<&'a CategoryDefEntry>> {
    if desired.parent.is_none() {
        return match matches {
            [server] => Ok(Some(*server)),
            _ => Err(anyhow!(
                "category '{}' matches {} server categories by {} '{}'; add its id or parent to select one safely",
                desired.name,
                matches.len(),
                field,
                value
            )),
        };
    }
    let scoped: Vec<_> = matches
        .iter()
        .copied()
        .filter(|candidate| parent_scope_matches(desired, candidate, index, file))
        .collect();
    if let [server] = scoped.as_slice() {
        Ok(Some(*server))
    } else if scoped.is_empty() {
        Ok(None)
    } else {
        Err(anyhow!(
            "category '{}' matches {} server categories by {} '{}'; add its id to select one safely",
            desired.name,
            matches.len(),
            field,
            value
        ))
    }
}

/// Match a file entry to a server entry: by `id`, else parent-scoped `slug`,
/// else parent-scoped `name`.
fn match_server<'a>(
    e: &CategoryDefEntry,
    index: &ServerIndex<'a>,
    file: &[CategoryDefEntry],
) -> Result<(Option<&'a CategoryDefEntry>, bool)> {
    if let Some(id) = e.id {
        // id given but absent -> treat as create, no rename ambiguity.
        return Ok((index.by_id.get(&id).copied(), false));
    }
    if let Some(slug) = &e.slug
        && let Some(matches) = index.by_slug.get(slug.as_str())
        && let Some(server) = select_server_match(e, matches, "slug", slug, index, file)?
    {
        return Ok((Some(server), false));
    }
    if let Some(matches) = index.by_name.get(e.name.as_str())
        && let Some(server) = select_server_match(e, matches, "name", &e.name, index, file)?
    {
        return Ok((Some(server), false));
    }
    Ok((None, true))
}

/// Validate every explicitly managed style against the effective state after
/// applying its partial file entry. This runs before planning so dry-runs and
/// real pushes reject the same invalid input before any write can happen.
fn validate_styles(file: &[CategoryDefEntry], server: &[CategoryDefEntry]) -> Result<()> {
    let index = ServerIndex::build(server);
    let mut invalid = Vec::new();

    for entry in file {
        if entry.style_type.is_none() && entry.icon.is_none() && entry.emoji.is_none() {
            continue;
        }
        let current = match_server(entry, &index, file)?.0;
        let style_type = entry
            .style_type
            .as_deref()
            .or_else(|| current.and_then(|value| value.style_type.as_deref()));
        let icon = entry
            .icon
            .as_deref()
            .or_else(|| current.and_then(|value| value.icon.as_deref()));
        let emoji = entry
            .emoji
            .as_deref()
            .or_else(|| current.and_then(|value| value.emoji.as_deref()));

        if let Err(error) = validate_style_state(&entry.name, style_type, icon, emoji) {
            invalid.push(error.to_string());
        }
    }

    if invalid.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("invalid category styles: {}", invalid.join(", ")))
    }
}

fn opt_diff<T: PartialEq>(a: &Option<T>, b: &Option<T>) -> bool {
    a.is_some() && a != b
}

/// Empty strings explicitly clear nullable string fields. Discourse returns a
/// cleared value as null, so compare empty and absent as equivalent.
fn opt_nullable_string_diff(a: &Option<String>, b: &Option<String>) -> bool {
    a.as_ref()
        .is_some_and(|desired| desired != b.as_deref().unwrap_or_default())
}

fn opt_style_type_diff(a: &Option<String>, b: &Option<String>) -> bool {
    a.as_ref()
        .is_some_and(|desired| desired.trim() != b.as_deref().unwrap_or("square"))
}

fn opt_list_diff(a: &Option<Vec<String>>, b: &Option<Vec<String>>) -> bool {
    match a {
        Some(av) => {
            let mut a2 = av.clone();
            a2.sort();
            let mut b2 = b.clone().unwrap_or_default();
            b2.sort();
            a2 != b2
        }
        None => false,
    }
}

/// Discourse serialises a cleared list as absent, so treat empty and absent as
/// equal while still preserving omission as "leave untouched".
fn opt_vec_diff<T: PartialEq>(a: &Option<Vec<T>>, b: &Option<Vec<T>>) -> bool {
    a.as_ref()
        .is_some_and(|values| values.as_slice() != b.as_deref().unwrap_or_default())
}

fn opt_map_diff(a: &Option<BTreeMap<String, Value>>, b: &Option<BTreeMap<String, Value>>) -> bool {
    a.as_ref()
        .is_some_and(|values| values != b.as_ref().unwrap_or(&BTreeMap::new()))
}

/// Return the specified file fields that differ from the server. Omitted file
/// fields are intentionally absent because `def push` leaves them untouched.
fn changed_fields(e: &CategoryDefEntry, s: &CategoryDefEntry) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if e.name != s.name {
        fields.push("name");
    }
    if opt_diff(&e.slug, &s.slug) {
        fields.push("slug");
    }
    if opt_diff(&e.color, &s.color) {
        fields.push("color");
    }
    if opt_diff(&e.text_color, &s.text_color) {
        fields.push("text_color");
    }
    if opt_style_type_diff(&e.style_type, &s.style_type) {
        fields.push("style_type");
    }
    if opt_nullable_string_diff(&e.icon, &s.icon) {
        fields.push("icon");
    }
    if opt_nullable_string_diff(&e.emoji, &s.emoji) {
        fields.push("emoji");
    }
    if opt_diff(&e.position, &s.position) {
        fields.push("position");
    }
    if opt_diff(&e.parent, &s.parent) {
        fields.push("parent");
    }
    if opt_diff(&e.read_restricted, &s.read_restricted) {
        fields.push("read_restricted");
    }
    if e.description.is_some()
        && normalize_description(&e.description) != normalize_description(&s.description)
    {
        fields.push("description");
    }
    if opt_diff(&e.topic_template, &s.topic_template) {
        fields.push("topic_template");
    }
    if opt_diff(&e.topic_title_placeholder, &s.topic_title_placeholder) {
        fields.push("topic_title_placeholder");
    }
    if opt_diff(&e.permissions, &s.permissions) {
        fields.push("permissions");
    }
    if opt_list_diff(&e.allowed_tags, &s.allowed_tags) {
        fields.push("allowed_tags");
    }
    if opt_list_diff(&e.allowed_tag_groups, &s.allowed_tag_groups) {
        fields.push("allowed_tag_groups");
    }
    if opt_diff(&e.minimum_required_tags, &s.minimum_required_tags) {
        fields.push("minimum_required_tags");
    }
    if opt_vec_diff(&e.required_tag_groups, &s.required_tag_groups) {
        fields.push("required_tag_groups");
    }
    if opt_list_diff(&e.category_types, &s.category_types) {
        fields.push("category_types");
    }
    if opt_map_diff(&e.custom_fields, &s.custom_fields) {
        fields.push("custom_fields");
    }
    if opt_diff(&e.sort_order, &s.sort_order) {
        fields.push("sort_order");
    }
    if opt_diff(&e.default_view, &s.default_view) {
        fields.push("default_view");
    }
    if opt_diff(&e.subcategory_list_style, &s.subcategory_list_style) {
        fields.push("subcategory_list_style");
    }
    if opt_diff(&e.num_featured_topics, &s.num_featured_topics) {
        fields.push("num_featured_topics");
    }
    if opt_diff(&e.show_subcategory_list, &s.show_subcategory_list) {
        fields.push("show_subcategory_list");
    }
    fields
}

/// Classify each file entry against the server (upsert; never delete).
fn plan_push(file: &[CategoryDefEntry], server: &[CategoryDefEntry]) -> Result<Vec<DefAction>> {
    let mut file_ids = HashMap::new();
    for (index, entry) in file.iter().enumerate() {
        if let Some(id) = entry.id
            && let Some(previous) = file_ids.insert(id, index)
        {
            return Err(anyhow!(
                "file entries '{}' and '{}' both declare category id {}; keep only one entry per category",
                file[previous].name,
                entry.name,
                id
            ));
        }
    }
    let index = ServerIndex::build(server);
    let plan: Vec<DefAction> = file
        .iter()
        .map(|e| {
            let (matched, rename_warning) = match_server(e, &index, file)?;
            let (kind, server_id, changed_fields) = match matched {
                Some(s) => {
                    let changed_fields = changed_fields(e, s);
                    (
                        if changed_fields.is_empty() {
                            DefActionKind::Unchanged
                        } else {
                            DefActionKind::Update
                        },
                        s.id,
                        changed_fields,
                    )
                }
                None => (DefActionKind::Create, None, Vec::new()),
            };
            Ok(DefAction {
                name: e.name.clone(),
                kind,
                server_id,
                changed_fields,
                rename_warning,
            })
        })
        .collect::<Result<_>>()?;

    let mut targeted = HashMap::new();
    for (index, action) in plan.iter().enumerate() {
        if let Some(id) = action.server_id
            && let Some(previous) = targeted.insert(id, index)
        {
            return Err(anyhow!(
                "file entries '{}' and '{}' both target server category id {}; keep only one entry per category",
                file[previous].name,
                file[index].name,
                id
            ));
        }
    }
    Ok(plan)
}

// ─── Commands: def pull / def push ────────────────────────────────────────────

fn is_json_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

fn contains_style_fields(value: &Value) -> bool {
    value
        .get("categories")
        .and_then(Value::as_array)
        .is_some_and(|categories| {
            categories.iter().any(|category| {
                category.as_object().is_some_and(|category| {
                    ["style_type", "icon", "emoji"]
                        .iter()
                        .any(|field| category.contains_key(*field))
                })
            })
        })
}

fn validate_file_version(file: &CategoriesFile, contains_style_fields: bool) -> Result<()> {
    match file.version {
        1 if contains_style_fields => Err(anyhow!(
            "categories file version 1 cannot contain style_type, icon, or emoji; set version to {}",
            CATEGORIES_FILE_VERSION
        )),
        1 | CATEGORIES_FILE_VERSION => Ok(()),
        version => Err(anyhow!("unsupported categories file version: {}", version)),
    }
}

pub fn category_def_pull(
    config: &Config,
    discourse_name: &str,
    local_path: Option<&Path>,
    force: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let mut defs = client.fetch_category_definitions()?;
    enrich_custom_fields(&client, &mut defs)?;
    let id_to_slug = id_to_slug_map(&defs);

    let mut entries: Vec<CategoryDefEntry> =
        defs.iter().map(|d| def_to_entry(d, &id_to_slug)).collect();
    // Stable order for clean diffs: by position, then name.
    entries.sort_by(|a, b| {
        a.position
            .unwrap_or(i64::MAX)
            .cmp(&b.position.unwrap_or(i64::MAX))
            .then_with(|| a.parent.cmp(&b.parent))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });

    let file = CategoriesFile {
        version: CATEGORIES_FILE_VERSION,
        categories: entries,
    };

    let default_path = Path::new("categories.yaml");
    let path = local_path.unwrap_or(default_path);
    let content = if is_json_path(path) {
        serde_json::to_string_pretty(&file).context("serializing categories as JSON")?
    } else {
        serde_yaml::to_string(&file).context("serializing categories as YAML")?
    };
    atomic_write(path, &content, force)?;
    println!(
        "Wrote {} category definition(s) to {}",
        file.categories.len(),
        path.display()
    );
    Ok(())
}

pub fn category_def_push(
    config: &Config,
    discourse_name: &str,
    local_path: &Path,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let content = fs::read_to_string(local_path)
        .with_context(|| format!("reading {}", local_path.display()))?;
    let raw: Value = if is_json_path(local_path) {
        serde_json::from_str(&content).context("parsing categories JSON")?
    } else {
        serde_yaml::from_str(&content).context("parsing categories YAML")?
    };
    let file: CategoriesFile = if is_json_path(local_path) {
        serde_json::from_str(&content).context("parsing categories JSON")?
    } else {
        serde_yaml::from_str(&content).context("parsing categories YAML")?
    };
    validate_file_version(&file, contains_style_fields(&raw))?;

    let mut defs = client.fetch_category_definitions()?;
    if file
        .categories
        .iter()
        .any(|entry| entry.custom_fields.is_some())
    {
        enrich_custom_fields(&client, &mut defs)?;
    }
    let id_to_slug = id_to_slug_map(&defs);
    let server_entries: Vec<CategoryDefEntry> =
        defs.iter().map(|d| def_to_entry(d, &id_to_slug)).collect();

    let mut plan = plan_push(&file.categories, &server_entries)?;
    validate_styles(&file.categories, &server_entries)?;
    let parent_targets = resolve_parent_targets(&file.categories, &server_entries, &plan)?;
    reconcile_parent_changes(
        &file.categories,
        &server_entries,
        &mut plan,
        &parent_targets,
    );
    validate_hierarchy(&file.categories, &defs, &plan, &parent_targets)?;
    validate_desired_identities(&file.categories, &server_entries, &plan, &parent_targets)?;
    let order = order_for_push(&file.categories, &server_entries, &plan, &parent_targets)?;

    // Parse and validate every entry before the first request can mutate the server.
    for entry in &file.categories {
        entry_to_params(
            entry,
            entry
                .parent
                .as_ref()
                .and_then(|parent| parent.as_ref().map(|_| 0)),
        )?;
        if let Some(custom_fields) = &entry.custom_fields {
            validate_custom_fields(custom_fields)?;
        }
    }

    if dry_run {
        println!(
            "[dry-run] Category definition plan for '{}':",
            discourse_name
        );
        let mut changes = 0;
        for &i in &order {
            let action = &plan[i];
            match action.kind {
                DefActionKind::Create => {
                    println!("  + create: {}", action.name);
                    changes += 1;
                    if action.rename_warning {
                        println!(
                            "      ! no id and no slug/name match - this CREATES a new category. \
                             If you meant to rename an existing one, use its id (or a future \
                             `category rename`) to preserve its topics."
                        );
                    }
                }
                DefActionKind::Update => {
                    println!(
                        "  ~ update: {} ({})",
                        action.name,
                        action.changed_fields.join(", ")
                    );
                    changes += 1;
                }
                DefActionKind::Unchanged => println!("  = unchanged: {}", action.name),
            }
        }
        if changes == 0 {
            println!("  (no changes)");
        }
        println!("[dry-run] No changes applied.");
        return Ok(());
    }

    let mut created_ids = HashMap::new();
    for &i in &order {
        let entry = &file.categories[i];
        let action = &plan[i];
        let parent_id = match parent_targets[i] {
            None => None,
            Some(ParentTarget::Server(id)) => Some(id),
            Some(ParentTarget::File(parent)) => plan[parent]
                .server_id
                .or_else(|| created_ids.get(&parent).copied())
                .ok_or_else(|| anyhow!("internal: parent category was not applied first"))
                .map(Some)?,
        };
        match action.kind {
            DefActionKind::Create => {
                let params = entry_to_params(entry, parent_id)?;
                let id = client
                    .create_category_def(&params)
                    .with_context(|| format!("creating category '{}'", entry.name))?;
                created_ids.insert(i, id);
                if let Some(custom_fields) = &entry.custom_fields {
                    client
                        .update_category_custom_fields(
                            id,
                            &custom_fields_patch(custom_fields, None),
                        )
                        .with_context(|| {
                            format!("setting custom fields on category '{}'", entry.name)
                        })?;
                }
                println!("  + created: {} (id {})", entry.name, id);
            }
            DefActionKind::Update => {
                let id = action
                    .server_id
                    .ok_or_else(|| anyhow!("internal: update without a server id"))?;
                if action
                    .changed_fields
                    .iter()
                    .any(|field| *field != "custom_fields")
                {
                    let params = entry_to_params(entry, parent_id)?;
                    client
                        .update_category(id, &params)
                        .with_context(|| format!("updating category '{}'", entry.name))?;
                }
                if action.changed_fields.contains(&"custom_fields") {
                    let current = server_entries
                        .iter()
                        .find(|server| server.id == Some(id))
                        .and_then(|server| server.custom_fields.as_ref());
                    let custom_fields = entry
                        .custom_fields
                        .as_ref()
                        .ok_or_else(|| anyhow!("internal: custom-fields update without a value"))?;
                    client
                        .update_category_custom_fields(
                            id,
                            &custom_fields_patch(custom_fields, current),
                        )
                        .with_context(|| {
                            format!("updating custom fields on category '{}'", entry.name)
                        })?;
                }
                println!("  ~ updated: {} (id {})", entry.name, id);
            }
            DefActionKind::Unchanged => {}
        }
    }
    println!("Push complete.");
    Ok(())
}

// ─── Diff (compare two live category definitions) ─────────────────────────────

struct CategoryDiffSide {
    label: String,
    entry: CategoryDefEntry,
}

#[derive(Debug, Serialize, PartialEq)]
struct CategoryDiffRow {
    field: String,
    a: Option<Value>,
    b: Option<Value>,
}

fn load_category_diff_side(
    config: &Config,
    discourse_name: &str,
    category: &str,
) -> Result<CategoryDiffSide> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let entry = resolve_entry(&client, category, true)?;
    Ok(CategoryDiffSide {
        label: format!("{} / {}", discourse.name, entry.name),
        entry,
    })
}

/// Compare the stable, editable definition fields of two resolved categories.
/// Server-specific ids and volatile usage fields are deliberately excluded.
fn diff_entries(a: &CategoryDefEntry, b: &CategoryDefEntry) -> Result<Vec<CategoryDiffRow>> {
    let mut rows = Vec::new();
    for field in VALID_FIELDS {
        let (_, va) = entry_field(a, field)?;
        let (_, vb) = entry_field(b, field)?;
        if va != vb {
            rows.push(CategoryDiffRow {
                field: (*field).to_string(),
                a: (!va.is_null()).then_some(va),
                b: (!vb.is_null()).then_some(vb),
            });
        }
    }
    Ok(rows)
}

fn fmt_diff_value(v: &Option<Value>) -> String {
    match v {
        Some(Value::String(s)) if s.is_empty() => "\"\"".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(value) => value.to_string(),
        None => "(unset)".to_string(),
    }
}

fn print_category_diff(
    rows: &[CategoryDiffRow],
    label_a: &str,
    label_b: &str,
    format: ListFormat,
) -> Result<()> {
    match format {
        ListFormat::Text => {
            if rows.is_empty() {
                println!("{} and {}: no differences.", label_a, label_b);
                return Ok(());
            }
            println!(
                "{} differing definition field{} between {} and {}:",
                rows.len(),
                if rows.len() == 1 { "" } else { "s" },
                label_a,
                label_b
            );
            for row in rows {
                println!("  {}", row.field);
                println!("    {}: {}", label_a, fmt_diff_value(&row.a));
                println!("    {}: {}", label_b, fmt_diff_value(&row.b));
            }
        }
        ListFormat::Json => {
            let payload = json!({
                "a": label_a,
                "b": label_b,
                "differences": rows,
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        ListFormat::Yaml => {
            let payload = json!({
                "a": label_a,
                "b": label_b,
                "differences": rows,
            });
            print!("{}", serde_yaml::to_string(&payload)?);
        }
    }
    Ok(())
}

/// Compare two explicitly selected live category definitions.
pub fn category_diff(
    config: &Config,
    discourse_a: &str,
    category_a: &str,
    discourse_b: &str,
    category_b: &str,
    format: ListFormat,
) -> Result<()> {
    let a = load_category_diff_side(config, discourse_a, category_a)?;
    let b = load_category_diff_side(config, discourse_b, category_b)?;
    let rows = diff_entries(&a.entry, &b.entry)?;
    print_category_diff(&rows, &a.label, &b.label, format)
}

// ─── Commands: show / get / set ───────────────────────────────────────────────

fn find_def<'a>(defs: &'a [CategoryDefinition], category: &str) -> Result<&'a CategoryDefinition> {
    if let Ok(id) = category.parse::<u64>() {
        return defs
            .iter()
            .find(|d| d.id == Some(id))
            .ok_or_else(|| not_found("category", category));
    }
    let slugs: Vec<_> = defs
        .iter()
        .filter(|def| def.slug.as_deref() == Some(category))
        .collect();
    match slugs.as_slice() {
        [category] => return Ok(*category),
        [_, _, ..] => {
            return Err(anyhow!(
                "category slug '{}' is ambiguous (matches {} categories); use an id",
                category,
                slugs.len()
            ));
        }
        [] => {}
    }
    let names: Vec<_> = defs.iter().filter(|def| def.name == category).collect();
    match names.as_slice() {
        [category] => Ok(*category),
        [_, _, ..] => Err(anyhow!(
            "category name '{}' is ambiguous (matches {} categories); use an id",
            category,
            names.len()
        )),
        [] => Err(not_found("category", category)),
    }
}

fn resolve_entry(
    client: &DiscourseClient,
    category: &str,
    include_custom_fields: bool,
) -> Result<CategoryDefEntry> {
    let mut defs = client.fetch_category_definitions()?;
    if include_custom_fields {
        enrich_custom_fields(client, &mut defs)?;
    }
    let id_to_slug = id_to_slug_map(&defs);
    let def = find_def(&defs, category)?;
    Ok(def_to_entry(def, &id_to_slug))
}

pub fn category_show(
    config: &Config,
    discourse_name: &str,
    category: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let entry = resolve_entry(&client, category, true)?;
    emit_result(format, &entry, &entry_text(&entry))
}

fn entry_text(e: &CategoryDefEntry) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut push = |k: &str, v: String| lines.push(format!("{:<22} {}", format!("{}:", k), v));
    push("name", e.name.clone());
    if let Some(id) = e.id {
        push("id", id.to_string());
    }
    for field in VALID_FIELDS.iter().filter(|f| **f != "name") {
        if let Ok((text, val)) = entry_field(e, field)
            && !val.is_null()
        {
            push(field, text);
        }
    }
    lines.join("\n")
}

pub fn category_get(
    config: &Config,
    discourse_name: &str,
    category: &str,
    field: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let entry = resolve_entry(&client, category, field.trim() == "custom_fields")?;
    let (text, value) = entry_field(&entry, field)?;
    emit_result(format, &value, &text)
}

/// One field's `(text, json)` value for `get`/`show`. Null value = unset.
fn entry_field(e: &CategoryDefEntry, field: &str) -> Result<(String, Value)> {
    let optstr = |v: &Option<String>| match v {
        Some(s) => (s.clone(), json!(s)),
        None => ("(unset)".to_string(), Value::Null),
    };
    let optnum = |v: Option<u64>| match v {
        Some(n) => (n.to_string(), json!(n)),
        None => ("(unset)".to_string(), Value::Null),
    };
    let optbool = |v: Option<bool>| match v {
        Some(b) => (b.to_string(), json!(b)),
        None => ("(unset)".to_string(), Value::Null),
    };
    let optlist = |v: &Option<Vec<String>>| match v {
        Some(l) => (l.join(", "), json!(l)),
        None => ("(unset)".to_string(), Value::Null),
    };
    let required_tag_groups = |v: &Option<Vec<RequiredTagGroupEntry>>| match v {
        Some(groups) => (
            groups
                .iter()
                .map(|group| format!("{}:{}", group.name, group.min_count))
                .collect::<Vec<_>>()
                .join(", "),
            json!(groups),
        ),
        None => ("(unset)".to_string(), Value::Null),
    };
    let out = match field.trim() {
        "name" => (e.name.clone(), json!(e.name)),
        "slug" => optstr(&e.slug),
        "color" => optstr(&e.color),
        "text_color" => optstr(&e.text_color),
        "style_type" => optstr(&e.style_type),
        "icon" => optstr(&e.icon),
        "emoji" => optstr(&e.emoji),
        "position" => match e.position {
            Some(n) => (n.to_string(), json!(n)),
            None => ("(unset)".to_string(), Value::Null),
        },
        "parent" => match &e.parent {
            Some(Some(parent)) => (parent.clone(), json!(parent)),
            Some(None) => ("(top-level)".to_string(), Value::Null),
            None => ("(unset)".to_string(), Value::Null),
        },
        "read_restricted" => optbool(e.read_restricted),
        "description" => optstr(&e.description),
        "topic_template" => optstr(&e.topic_template),
        "topic_title_placeholder" => optstr(&e.topic_title_placeholder),
        "permissions" => match &e.permissions {
            Some(m) => (
                m.iter()
                    .map(|(k, v)| format!("{}:{}", k, v))
                    .collect::<Vec<_>>()
                    .join(","),
                json!(m),
            ),
            None => ("(unset)".to_string(), Value::Null),
        },
        "allowed_tags" => optlist(&e.allowed_tags),
        "allowed_tag_groups" => optlist(&e.allowed_tag_groups),
        "minimum_required_tags" => optnum(e.minimum_required_tags),
        "required_tag_groups" => required_tag_groups(&e.required_tag_groups),
        "category_types" => optlist(&e.category_types),
        "custom_fields" => match &e.custom_fields {
            Some(fields) => (serde_json::to_string(fields)?, json!(fields)),
            None => ("(unset)".to_string(), Value::Null),
        },
        "sort_order" => optstr(&e.sort_order),
        "default_view" => optstr(&e.default_view),
        "subcategory_list_style" => optstr(&e.subcategory_list_style),
        "num_featured_topics" => optnum(e.num_featured_topics),
        "show_subcategory_list" => optbool(e.show_subcategory_list),
        other => {
            return Err(anyhow!(
                "unknown category field '{}'. Valid fields: {}",
                other,
                VALID_FIELDS.join(", ")
            ));
        }
    };
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryListEdit {
    Append,
    Remove,
}

pub fn category_set(
    config: &Config,
    discourse_name: &str,
    category: &str,
    field: &str,
    value: &str,
    list_edit: Option<CategoryListEdit>,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let mut defs = client.fetch_category_definitions()?;
    if field.trim() == "custom_fields" {
        enrich_custom_fields(&client, &mut defs)?;
    }
    let slug_to_ids = slug_to_ids_map(&defs);
    let name_to_ids = name_to_ids_map(&defs);
    let def = find_def(&defs, category)?;
    let id = def.id.ok_or_else(|| not_found("category", category))?;

    if field.trim() == "custom_fields" && list_edit.is_none() {
        let desired = parse_custom_fields(value)?;
        let patch = custom_fields_patch(&desired, def.custom_fields.as_ref());
        if patch.is_empty() {
            println!(
                "Category '{}' (id {}) custom_fields unchanged",
                category, id
            );
            return Ok(());
        }
        if dry_run {
            println!(
                "[dry-run] would PUT /categories/{}.json with JSON: {}",
                id,
                serde_json::to_string(&json!({ "custom_fields": patch }))?
            );
            return Ok(());
        }
        client
            .update_category_custom_fields(id, &patch)
            .with_context(|| format!("setting custom_fields on category '{}'", category))?;
        println!("Set custom_fields on category '{}' (id {})", category, id);
        return Ok(());
    }

    let params = if let Some(edit) = list_edit {
        let Some(params) = list_field_edit_params(
            field,
            &def.allowed_tags,
            &def.allowed_tag_groups,
            value,
            edit,
        )?
        else {
            println!("Category '{}' (id {}) {} unchanged", category, id, field);
            return Ok(());
        };
        params
    } else {
        if matches!(field.trim(), "style_type" | "icon" | "emoji") {
            let style_type = if field.trim() == "style_type" {
                Some(value)
            } else {
                def.style_type.as_deref()
            };
            let icon = if field.trim() == "icon" {
                Some(value)
            } else {
                def.icon.as_deref()
            };
            let emoji = if field.trim() == "emoji" {
                Some(value)
            } else {
                def.emoji.as_deref()
            };
            validate_style_state(&def.name, style_type, icon, emoji)?;
        }
        field_to_set_params(field, value, &slug_to_ids, &name_to_ids)?
    };

    if dry_run {
        println!(
            "[dry-run] would PUT /categories/{}.json with: {}",
            id,
            params
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(());
    }
    client
        .update_category(id, &params)
        .with_context(|| format!("setting {} on category '{}'", field, category))?;
    println!("Set {} on category '{}' (id {})", field, category, id);
    Ok(())
}

/// Resolve and validate a rename against the server's current category
/// definitions. Pure (no network): looks up `category` by id/slug/name,
/// rejects an empty/identical new name, and rejects a new name already used
/// by a sibling category. Returns `(id, old_name, normalised_new_name)`.
fn plan_rename(
    defs: &[CategoryDefinition],
    category: &str,
    new_name: &str,
) -> Result<(u64, String, String)> {
    let new_norm = new_name.trim();
    if new_norm.is_empty() {
        return Err(anyhow!("new category name is empty"));
    }

    let def = find_def(defs, category)?;
    let id = def.id.ok_or_else(|| not_found("category", category))?;
    let old_name = def.name.clone();

    if old_name == new_norm {
        return Err(anyhow!(
            "old and new category names are identical: '{}'",
            old_name
        ));
    }
    if defs.iter().any(|candidate| {
        candidate.id != Some(id)
            && candidate.parent_category_id == def.parent_category_id
            && candidate.name.to_lowercase() == new_norm.to_lowercase()
    }) {
        return Err(anyhow!(
            "cannot rename to '{}': a category with that name already exists",
            new_norm
        ));
    }

    Ok((id, old_name, new_norm.to_string()))
}

/// Rename a category, preserving its topics: a safe `PUT /categories/{id}`
/// by resolved id, in contrast to editing `name` in a no-`id` file entry and
/// running `def push`, which cannot tell a rename from a delete+create.
pub fn category_rename(
    config: &Config,
    discourse_name: &str,
    category: &str,
    new_name: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let defs = client.fetch_category_definitions()?;
    let (id, old_name, new_norm) = plan_rename(&defs, category, new_name)?;

    if dry_run {
        println!(
            "[dry-run] would rename category '{}' -> '{}' on '{}' (id {})",
            old_name, new_norm, discourse_name, id
        );
        return Ok(());
    }

    client
        .update_category(id, &[("name".to_string(), new_norm.clone())])
        .with_context(|| format!("renaming category '{}'", old_name))?;
    println!(
        "Renamed category '{}' -> '{}' (id {})",
        old_name, new_norm, id
    );
    Ok(())
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        other => Err(anyhow!("expected a boolean (true/false), got '{}'", other)),
    }
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Parse `everyone:full,staff:create_post` into `permissions[group]=level` form
/// params; adds `read_restricted=true` when any non-`everyone` group is granted.
fn parse_permissions(value: &str) -> Result<Vec<(String, String)>> {
    let mut params = Vec::new();
    let mut non_everyone = false;
    for pair in value.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let (group, level) = pair
            .split_once(':')
            .ok_or_else(|| anyhow!("permission '{}' must be group:level", pair))?;
        let group = group.trim();
        if group != "everyone" {
            non_everyone = true;
        }
        params.push((
            format!("permissions[{}]", group),
            perm_type(level)?.to_string(),
        ));
    }
    if params.is_empty() {
        return Err(anyhow!("no permissions parsed from '{}'", value));
    }
    if non_everyone {
        params.push(("read_restricted".to_string(), "true".to_string()));
    }
    Ok(params)
}

/// Parse `Role:1,Genre:2` into the nested form Discourse expects.
fn parse_required_tag_groups(value: &str) -> Result<Vec<(String, String)>> {
    let mut params = Vec::new();
    for pair in value.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let (name, min_count) = pair
            .rsplit_once(':')
            .ok_or_else(|| anyhow!("required tag group '{}' must be name:min_count", pair))?;
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("required tag group name is empty"));
        }
        let min_count = min_count
            .trim()
            .parse::<u64>()
            .with_context(|| format!("required tag group '{}' has an invalid min_count", name))?;
        params.push(("required_tag_groups[][name]".to_string(), name.to_string()));
        params.push((
            "required_tag_groups[][min_count]".to_string(),
            min_count.to_string(),
        ));
    }
    if params.is_empty() {
        return Ok(vec![(
            "required_tag_groups[][name]".to_string(),
            String::new(),
        )]);
    }
    Ok(params)
}

fn parse_custom_fields(value: &str) -> Result<BTreeMap<String, Value>> {
    let value: Value =
        serde_json::from_str(value).context("custom_fields must be a JSON object")?;
    let Value::Object(fields) = value else {
        return Err(anyhow!("custom_fields must be a JSON object"));
    };
    let fields: BTreeMap<String, Value> = fields.into_iter().collect();
    validate_custom_fields(&fields)?;
    Ok(fields)
}

/// Merge `edits` into `current` for a `--append`/`--remove` list-field edit.
/// Append dedupes (an item already present is left in place, not duplicated);
/// remove is a no-op for items not present. Pure, order-preserving.
fn merge_list_field(
    current: &Option<Vec<String>>,
    edits: &[String],
    edit: CategoryListEdit,
) -> Vec<String> {
    let mut items = current.clone().unwrap_or_default();
    match edit {
        CategoryListEdit::Append => {
            for e in edits {
                if !items.contains(e) {
                    items.push(e.clone());
                }
            }
        }
        CategoryListEdit::Remove => items.retain(|i| !edits.contains(i)),
    }
    items
}

/// Build the form params for a `--append`/`--remove` edit of a list field,
/// against the field's current server-side value.
fn list_field_edit_params(
    field: &str,
    current_allowed_tags: &Option<Vec<String>>,
    current_allowed_tag_groups: &Option<Vec<String>>,
    value: &str,
    edit: CategoryListEdit,
) -> Result<Option<Vec<(String, String)>>> {
    let current = match field.trim() {
        "allowed_tags" => current_allowed_tags,
        "allowed_tag_groups" => current_allowed_tag_groups,
        other => {
            return Err(anyhow!(
                "--append/--remove only apply to list fields (allowed_tags, allowed_tag_groups), not '{}'",
                other
            ));
        }
    };
    let edits = split_csv(value);
    if edits.is_empty() {
        return Err(anyhow!(
            "--append/--remove requires a non-empty comma-separated value"
        ));
    }
    let merged = merge_list_field(current, &edits, edit);
    if merged == current.as_deref().unwrap_or_default() {
        return Ok(None);
    }
    let key = format!("{}[]", field.trim());
    if merged.is_empty() {
        Ok(Some(vec![(key, String::new())]))
    } else {
        Ok(Some(merged.into_iter().map(|v| (key.clone(), v)).collect()))
    }
}

/// Build the form params for setting a single field.
fn field_to_set_params(
    field: &str,
    value: &str,
    slug_to_ids: &BTreeMap<String, Vec<u64>>,
    name_to_ids: &BTreeMap<String, Vec<u64>>,
) -> Result<Vec<(String, String)>> {
    let one = |k: &str, v: String| vec![(k.to_string(), v)];
    let list = |key: &str, value: &str| -> Vec<(String, String)> {
        let items = split_csv(value);
        if items.is_empty() {
            // An empty value clears the list.
            vec![(key.to_string(), String::new())]
        } else {
            items.into_iter().map(|t| (key.to_string(), t)).collect()
        }
    };
    let params = match field.trim() {
        "name" => one("name", value.to_string()),
        "slug" => one("slug", value.to_string()),
        "color" => one("color", value.trim_start_matches('#').to_string()),
        "text_color" => one("text_color", value.trim_start_matches('#').to_string()),
        "style_type" => one("style_type", validate_style_type(value)?.to_string()),
        "icon" => one("icon", value.to_string()),
        "emoji" => one("emoji", value.to_string()),
        "position" => {
            value
                .parse::<i64>()
                .with_context(|| format!("position must be an integer, got '{}'", value))?;
            one("position", value.to_string())
        }
        "parent" => {
            let parent = value.trim();
            if parent.is_empty() {
                return Ok(one("parent_category_id", String::new()));
            }
            let pid = resolve_parent_id(parent, slug_to_ids, name_to_ids)?.ok_or_else(|| {
                anyhow!(
                    "parent category '{}' not found on the server by slug or name",
                    parent
                )
            })?;
            one("parent_category_id", pid.to_string())
        }
        "read_restricted" => one("read_restricted", parse_bool(value)?.to_string()),
        "description" => one("description", value.to_string()),
        "topic_template" => one("topic_template", value.to_string()),
        "topic_title_placeholder" => one("topic_title_placeholder", value.to_string()),
        "minimum_required_tags" => {
            value.parse::<u64>().with_context(|| {
                format!("minimum_required_tags must be an integer, got '{}'", value)
            })?;
            one("minimum_required_tags", value.to_string())
        }
        "required_tag_groups" => parse_required_tag_groups(value)?,
        "category_types" => list("category_types[]", value),
        "allowed_tags" => list("allowed_tags[]", value),
        "allowed_tag_groups" => list("allowed_tag_groups[]", value),
        "permissions" => parse_permissions(value)?,
        "sort_order" => one("sort_order", value.to_string()),
        "default_view" => one("default_view", value.to_string()),
        "subcategory_list_style" => one("subcategory_list_style", value.to_string()),
        "num_featured_topics" => {
            value.parse::<u64>().with_context(|| {
                format!("num_featured_topics must be an integer, got '{}'", value)
            })?;
            one("num_featured_topics", value.to_string())
        }
        "show_subcategory_list" => one("show_subcategory_list", parse_bool(value)?.to_string()),
        other => {
            return Err(anyhow!(
                "unknown category field '{}'. Valid fields: {}",
                other,
                VALID_FIELDS.join(", ")
            ));
        }
    };
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> CategoryDefEntry {
        CategoryDefEntry {
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn perm_round_trip() {
        for (t, label) in [(1u8, "full"), (2, "create_post"), (3, "readonly")] {
            assert_eq!(perm_label(t), label);
            assert_eq!(perm_type(label).unwrap(), t);
        }
        assert!(perm_type("bogus").is_err());
    }

    #[test]
    fn plan_creates_when_absent() {
        let file = vec![entry("New")];
        let plan = plan_push(&file, &[]).unwrap();
        assert_eq!(plan[0].kind, DefActionKind::Create);
        // No id and no server match -> rename warning fires.
        assert!(plan[0].rename_warning);
    }

    #[test]
    fn plan_matches_by_id_and_detects_change() {
        let mut server = entry("Old Name");
        server.id = Some(7);
        server.slug = Some("general".to_string());
        let mut file = entry("New Name");
        file.id = Some(7);
        let plan = plan_push(&[file], &[server]).unwrap();
        assert_eq!(plan[0].kind, DefActionKind::Update);
        assert_eq!(plan[0].server_id, Some(7));
        assert!(!plan[0].rename_warning);
    }

    #[test]
    fn plan_unchanged_when_specified_fields_match() {
        let mut server = entry("General");
        server.id = Some(3);
        server.slug = Some("general".to_string());
        server.description = Some("desc".to_string());
        // File specifies only the name + a matching description.
        let mut file = entry("General");
        file.slug = Some("general".to_string());
        file.description = Some("desc".to_string());
        let plan = plan_push(&[file], &[server]).unwrap();
        assert_eq!(plan[0].kind, DefActionKind::Unchanged);
    }

    #[test]
    fn plan_matches_by_slug_without_id() {
        let mut server = entry("General");
        server.id = Some(3);
        server.slug = Some("general".to_string());
        let mut file = entry("General");
        file.slug = Some("general".to_string());
        let plan = plan_push(&[file], &[server]).unwrap();
        assert_eq!(plan[0].kind, DefActionKind::Unchanged);
        assert!(!plan[0].rename_warning);
    }

    #[test]
    fn plan_rejects_duplicate_server_name_without_an_id() {
        let mut first = entry("Dup");
        first.id = Some(1);
        let mut second = entry("Dup");
        second.id = Some(2);
        let file = entry("Dup");
        let error = plan_push(&[file], &[first, second]).unwrap_err();
        assert!(error.to_string().contains("add its id"));
    }

    #[test]
    fn plan_disambiguates_duplicate_aliases_by_parent() {
        let mut first_parent = entry("First Parent");
        first_parent.id = Some(1);
        first_parent.slug = Some("first".to_string());
        let mut second_parent = entry("Second Parent");
        second_parent.id = Some(2);
        second_parent.slug = Some("second".to_string());
        let mut first_child = entry("General");
        first_child.id = Some(3);
        first_child.slug = Some("general".to_string());
        first_child.server_parent_id = Some(1);
        let mut second_child = entry("General");
        second_child.id = Some(4);
        second_child.slug = Some("general".to_string());
        second_child.server_parent_id = Some(2);
        let mut desired = entry("General");
        desired.slug = Some("general".to_string());
        desired.parent = Some(Some("second".to_string()));

        let plan = plan_push(
            &[desired],
            &[first_parent, second_parent, first_child, second_child],
        )
        .unwrap();
        assert_eq!(plan[0].server_id, Some(4));
    }

    #[test]
    fn resolved_parent_aliases_are_idempotent() {
        let mut parent = entry("Parent");
        parent.id = Some(1);
        parent.slug = Some("parent".to_string());
        let mut child = entry("Child");
        child.id = Some(2);
        child.slug = Some("child".to_string());
        child.parent = Some(Some("parent".to_string()));
        child.server_parent_id = Some(1);
        let server = vec![parent, child];
        let mut desired = entry("Child");
        desired.slug = Some("child".to_string());
        desired.parent = Some(Some("Parent".to_string()));
        let file = vec![desired];
        let mut plan = plan_push(&file, &server).unwrap();
        let targets = resolve_parent_targets(&file, &server, &plan).unwrap();
        reconcile_parent_changes(&file, &server, &mut plan, &targets);
        assert_eq!(plan[0].kind, DefActionKind::Unchanged);
    }

    #[test]
    fn plan_rejects_duplicate_explicit_file_ids_even_when_absent_on_server() {
        let mut first = entry("First");
        first.id = Some(99);
        let mut second = entry("Second");
        second.id = Some(99);
        let error = plan_push(&[first, second], &[]).unwrap_err();
        assert!(error.to_string().contains("both declare category id 99"));
    }

    #[test]
    fn changed_fields_ignores_fields_the_file_omits() {
        let mut server = entry("General");
        server.description = Some("server desc".to_string());
        server.color = Some("ABABAB".to_string());
        // File omits description and color -> not a change.
        let file = entry("General");
        assert!(changed_fields(&file, &server).is_empty());
    }

    #[test]
    fn topic_title_placeholder_round_trips_through_entry_and_params() {
        let mut category = def(3, "Marketplace");
        category.topic_title_placeholder = Some("Genre, instrument, and location".to_string());

        let entry = def_to_entry(&category, &BTreeMap::new());
        assert_eq!(
            entry.topic_title_placeholder,
            Some("Genre, instrument, and location".to_string())
        );

        let params = entry_to_params(&entry, None).unwrap();
        assert!(params.contains(&(
            "topic_title_placeholder".to_string(),
            "Genre, instrument, and location".to_string()
        )));
    }

    #[test]
    fn set_params_topic_title_placeholder() {
        let params = field_to_set_params(
            "topic_title_placeholder",
            "Genre, instrument, and location",
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            params,
            vec![(
                "topic_title_placeholder".to_string(),
                "Genre, instrument, and location".to_string()
            )]
        );
    }

    #[test]
    fn set_params_empty_parent_moves_to_top_level() {
        let params = field_to_set_params("parent", "", &BTreeMap::new(), &BTreeMap::new()).unwrap();
        assert_eq!(
            params,
            vec![("parent_category_id".to_string(), String::new())]
        );
    }

    #[test]
    fn icon_and_emoji_round_trip_through_entry_and_params() {
        let category: CategoryDefinition = serde_json::from_value(json!({
            "id": 3,
            "name": "Marketplace",
            "style_type": "icon",
            "icon": "star",
            "emoji": "guitar"
        }))
        .unwrap();

        let entry = def_to_entry(&category, &BTreeMap::new());
        assert_eq!(entry.style_type, Some("icon".to_string()));
        assert_eq!(entry.icon, Some("star".to_string()));
        assert_eq!(entry.emoji, Some("guitar".to_string()));

        let params = entry_to_params(&entry, None).unwrap();
        assert!(params.contains(&("style_type".to_string(), "icon".to_string())));
        assert!(params.contains(&("icon".to_string(), "star".to_string())));
        assert!(params.contains(&("emoji".to_string(), "guitar".to_string())));
    }

    #[test]
    fn set_params_icon_and_emoji() {
        assert_eq!(
            field_to_set_params("icon", "star", &BTreeMap::new(), &BTreeMap::new()).unwrap(),
            vec![("icon".to_string(), "star".to_string())]
        );
        assert_eq!(
            field_to_set_params("emoji", "guitar", &BTreeMap::new(), &BTreeMap::new()).unwrap(),
            vec![("emoji".to_string(), "guitar".to_string())]
        );
        assert_eq!(
            field_to_set_params("style_type", "emoji", &BTreeMap::new(), &BTreeMap::new()).unwrap(),
            vec![("style_type".to_string(), "emoji".to_string())]
        );
        assert!(
            field_to_set_params("style_type", "image", &BTreeMap::new(), &BTreeMap::new()).is_err()
        );
    }

    #[test]
    fn changed_fields_detects_every_style_change() {
        let mut server = entry("General");
        server.style_type = Some("icon".to_string());
        server.icon = Some("star".to_string());
        server.emoji = Some("wave".to_string());
        let mut file = entry("General");
        file.style_type = Some("emoji".to_string());
        file.icon = Some("heart".to_string());
        file.emoji = Some("guitar".to_string());
        assert_eq!(
            changed_fields(&file, &server),
            vec!["style_type", "icon", "emoji"]
        );
    }

    #[test]
    fn cleared_icon_and_emoji_match_absent_server_values() {
        let mut file = entry("General");
        file.icon = Some(String::new());
        file.emoji = Some(String::new());
        assert!(changed_fields(&file, &entry("General")).is_empty());

        let mut server = entry("General");
        server.icon = Some("star".to_string());
        server.emoji = Some("wave".to_string());
        assert_eq!(changed_fields(&file, &server), vec!["icon", "emoji"]);
    }

    #[test]
    fn style_type_comparison_uses_the_normalized_value_sent_to_discourse() {
        let mut server = entry("General");
        server.style_type = Some("icon".to_string());
        let mut file = entry("General");
        file.style_type = Some(" icon ".to_string());
        file.icon = Some("star".to_string());
        server.icon = Some("star".to_string());
        assert!(changed_fields(&file, &server).is_empty());
    }

    #[test]
    fn validates_effective_style_state_before_push() {
        let mut create = entry("New");
        create.style_type = Some("icon".to_string());
        let error = validate_styles(&[create], &[]).unwrap_err();
        assert!(error.to_string().contains("has no icon"));

        let mut server = entry("Existing");
        server.style_type = Some("icon".to_string());
        server.icon = Some("star".to_string());
        let mut update = entry("Existing");
        update.style_type = Some("icon".to_string());
        assert!(validate_styles(&[update.clone()], &[server.clone()]).is_ok());

        update.icon = Some(String::new());
        let error = validate_styles(&[update], &[server]).unwrap_err();
        assert!(error.to_string().contains("has no icon"));
    }

    #[test]
    fn validates_style_type_and_emoji_companion() {
        assert!(validate_style_state("General", Some("image"), None, None).is_err());
        let error = validate_style_state("General", Some("emoji"), None, None).unwrap_err();
        assert!(error.to_string().contains("has no emoji"));
        assert!(validate_style_state("General", Some("emoji"), None, Some("wave")).is_ok());
    }

    #[test]
    fn changed_fields_flags_specified_mismatch() {
        let mut server = entry("General");
        server.color = Some("ABABAB".to_string());
        let mut file = entry("General");
        file.color = Some("FF0000".to_string());
        assert!(!changed_fields(&file, &server).is_empty());
    }

    #[test]
    fn changed_fields_ignores_terminal_description_line_endings() {
        let mut server = entry("General");
        server.description = Some("Description".to_string());
        let mut file = entry("General");
        file.description = Some("Description\n".to_string());
        assert!(changed_fields(&file, &server).is_empty());
    }

    #[test]
    fn category_definition_params_normalize_terminal_description_line_endings() {
        let mut category = entry("General");
        category.description = Some("Description\r\n".to_string());
        let params = entry_to_params(&category, None).unwrap();
        assert!(params.contains(&("description".to_string(), "Description".to_string())));
    }

    #[test]
    fn changed_fields_names_every_specified_difference() {
        let mut server = entry("General");
        server.color = Some("ABABAB".to_string());
        let mut file = entry("Renamed");
        file.color = Some("FF0000".to_string());
        assert_eq!(changed_fields(&file, &server), vec!["name", "color"]);
    }

    #[test]
    fn category_definition_rejects_unknown_fields() {
        let error = serde_yaml::from_str::<CategoriesFile>(
            "version: 1\ncategories:\n  - name: Marketplace\n    solved_enabled: true\n",
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn category_definition_rejects_unknown_top_level_fields() {
        let error = serde_yaml::from_str::<CategoriesFile>(
            "version: 1\nunknown: true\ncategories:\n  - name: Marketplace\n",
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn category_definition_rejects_unknown_json_fields() {
        let error = serde_json::from_str::<CategoriesFile>(
            r#"{"version":1,"categories":[{"name":"Marketplace","solved_enabled":true}]}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn category_definition_versions_style_fields_explicitly() {
        let version_one_source = "version: 1\ncategories:\n  - name: Marketplace\n";
        let version_one: CategoriesFile = serde_yaml::from_str(version_one_source).unwrap();
        let version_one_raw: Value = serde_yaml::from_str(version_one_source).unwrap();
        assert!(
            validate_file_version(&version_one, contains_style_fields(&version_one_raw)).is_ok()
        );

        let version_one_source =
            "version: 1\ncategories:\n  - name: Marketplace\n    style_type: null\n";
        let version_one_with_style: CategoriesFile =
            serde_yaml::from_str(version_one_source).unwrap();
        let version_one_with_style_raw: Value = serde_yaml::from_str(version_one_source).unwrap();
        assert!(
            validate_file_version(
                &version_one_with_style,
                contains_style_fields(&version_one_with_style_raw)
            )
            .unwrap_err()
            .to_string()
            .contains("set version to 2")
        );

        let version_two_source = "version: 2\ncategories:\n  - name: Marketplace\n    style_type: icon\n    icon: star\n";
        let version_two: CategoriesFile = serde_yaml::from_str(version_two_source).unwrap();
        let version_two_raw: Value = serde_yaml::from_str(version_two_source).unwrap();
        assert!(
            validate_file_version(&version_two, contains_style_fields(&version_two_raw)).is_ok()
        );
    }

    #[test]
    fn changed_fields_compares_lists_order_insensitively() {
        let mut server = entry("General");
        server.allowed_tags = Some(vec!["b".to_string(), "a".to_string()]);
        let mut file = entry("General");
        file.allowed_tags = Some(vec!["a".to_string(), "b".to_string()]);
        assert!(changed_fields(&file, &server).is_empty());
    }

    #[test]
    fn required_tag_groups_round_trip_through_entry_and_params() {
        let mut category = def(3, "Marketplace");
        category.required_tag_groups = Some(vec![crate::api::RequiredTagGroup {
            name: Some("Role".to_string()),
            min_count: Some(2),
        }]);

        let entry = def_to_entry(&category, &BTreeMap::new());
        assert_eq!(
            entry.required_tag_groups,
            Some(vec![RequiredTagGroupEntry {
                name: "Role".to_string(),
                min_count: 2,
            }])
        );

        let params = entry_to_params(&entry, None).unwrap();
        assert!(params.contains(&(
            "required_tag_groups[][name]".to_string(),
            "Role".to_string()
        )));
        assert!(params.contains(&(
            "required_tag_groups[][min_count]".to_string(),
            "2".to_string()
        )));
    }

    #[test]
    fn category_types_round_trip_as_sorted_extra_ids() {
        let mut category = def(3, "Marketplace");
        category.category_types = Some(BTreeMap::from([
            ("support".to_string(), json!({})),
            ("discussion".to_string(), json!({})),
        ]));

        let entry = def_to_entry(&category, &BTreeMap::new());
        assert_eq!(entry.category_types, Some(vec!["support".to_string()]));

        let params = entry_to_params(&entry, None).unwrap();
        assert!(params.contains(&("category_types[]".to_string(), "support".to_string())));
    }

    #[test]
    fn empty_required_tag_groups_match_an_absent_server_value() {
        let mut file = entry("Marketplace");
        file.required_tag_groups = Some(Vec::new());
        assert!(changed_fields(&file, &entry("Marketplace")).is_empty());
    }

    #[test]
    fn custom_fields_patch_updates_and_removes_only_changed_keys() {
        let desired = BTreeMap::from([
            ("enabled".to_string(), json!(true)),
            ("priority".to_string(), json!(2)),
        ]);
        let current = BTreeMap::from([
            ("enabled".to_string(), json!(false)),
            ("obsolete".to_string(), json!("remove me")),
            ("unchanged".to_string(), json!("keep")),
        ]);
        let mut desired = desired;
        desired.insert("unchanged".to_string(), json!("keep"));

        assert_eq!(
            custom_fields_patch(&desired, Some(&current)),
            BTreeMap::from([
                ("enabled".to_string(), json!(true)),
                ("obsolete".to_string(), Value::Null),
                ("priority".to_string(), json!(2)),
            ])
        );
    }

    #[test]
    fn custom_fields_compare_empty_with_an_absent_server_value() {
        let mut file = entry("Marketplace");
        file.custom_fields = Some(BTreeMap::new());
        assert!(changed_fields(&file, &entry("Marketplace")).is_empty());
    }

    #[test]
    fn parse_custom_fields_requires_a_json_object() {
        assert_eq!(
            parse_custom_fields(r#"{"enabled":true,"priority":2}"#).unwrap(),
            BTreeMap::from([
                ("enabled".to_string(), json!(true)),
                ("priority".to_string(), json!(2)),
            ])
        );
        assert!(parse_custom_fields("[]").is_err());
        assert!(parse_custom_fields(r#"{"roles":["staff"]}"#).is_err());
        assert!(parse_custom_fields("not json").is_err());
    }

    #[test]
    fn set_params_parse_required_tag_groups_and_empty_value_clears_them() {
        let params = field_to_set_params(
            "required_tag_groups",
            "Role:1,Genre:2",
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            params,
            vec![
                (
                    "required_tag_groups[][name]".to_string(),
                    "Role".to_string()
                ),
                (
                    "required_tag_groups[][min_count]".to_string(),
                    "1".to_string(),
                ),
                (
                    "required_tag_groups[][name]".to_string(),
                    "Genre".to_string()
                ),
                (
                    "required_tag_groups[][min_count]".to_string(),
                    "2".to_string(),
                ),
            ]
        );
        assert_eq!(
            field_to_set_params(
                "required_tag_groups",
                "",
                &BTreeMap::new(),
                &BTreeMap::new()
            )
            .unwrap(),
            vec![("required_tag_groups[][name]".to_string(), String::new(),)]
        );
    }

    #[test]
    fn set_params_rejects_invalid_required_tag_group() {
        let err = field_to_set_params(
            "required_tag_groups",
            "Role:many",
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid min_count"));
    }

    #[test]
    fn set_params_permissions_imply_read_restricted() {
        let params = field_to_set_params(
            "permissions",
            "staff:full",
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(params.contains(&("permissions[staff]".to_string(), "1".to_string())));
        assert!(params.contains(&("read_restricted".to_string(), "true".to_string())));
    }

    #[test]
    fn set_params_everyone_only_stays_public() {
        let params = field_to_set_params(
            "permissions",
            "everyone:full",
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(params.contains(&("permissions[everyone]".to_string(), "1".to_string())));
        assert!(!params.iter().any(|(k, _)| k == "read_restricted"));
    }

    #[test]
    fn set_params_unknown_field_errors() {
        let err =
            field_to_set_params("bogus", "x", &BTreeMap::new(), &BTreeMap::new()).unwrap_err();
        assert!(err.to_string().contains("unknown category field"));
    }

    #[test]
    fn set_params_list_clears_on_empty() {
        let params =
            field_to_set_params("allowed_tags", "", &BTreeMap::new(), &BTreeMap::new()).unwrap();
        assert_eq!(params, vec![("allowed_tags[]".to_string(), String::new())]);
    }

    #[test]
    fn merge_list_field_appends_and_dedupes() {
        let current = Some(vec!["a".to_string(), "b".to_string()]);
        let edits = vec!["b".to_string(), "c".to_string()];
        let merged = merge_list_field(&current, &edits, CategoryListEdit::Append);
        assert_eq!(merged, vec!["a", "b", "c"]);
    }

    #[test]
    fn merge_list_field_appends_onto_empty_current() {
        let merged = merge_list_field(&None, &["a".to_string()], CategoryListEdit::Append);
        assert_eq!(merged, vec!["a"]);
    }

    #[test]
    fn merge_list_field_removes_present_items() {
        let current = Some(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        let edits = vec!["b".to_string()];
        let merged = merge_list_field(&current, &edits, CategoryListEdit::Remove);
        assert_eq!(merged, vec!["a", "c"]);
    }

    #[test]
    fn merge_list_field_remove_missing_item_is_noop() {
        let current = Some(vec!["a".to_string()]);
        let edits = vec!["z".to_string()];
        let merged = merge_list_field(&current, &edits, CategoryListEdit::Remove);
        assert_eq!(merged, vec!["a"]);
    }

    #[test]
    fn list_field_edit_params_appends_to_current() {
        let current_tags = Some(vec!["one".to_string()]);
        let params = list_field_edit_params(
            "allowed_tags",
            &current_tags,
            &None,
            "two,three",
            CategoryListEdit::Append,
        )
        .unwrap();
        assert_eq!(
            params,
            Some(vec![
                ("allowed_tags[]".to_string(), "one".to_string()),
                ("allowed_tags[]".to_string(), "two".to_string()),
                ("allowed_tags[]".to_string(), "three".to_string()),
            ])
        );
    }

    #[test]
    fn list_field_edit_params_remove_to_empty_clears_list() {
        let current_tags = Some(vec!["one".to_string()]);
        let params = list_field_edit_params(
            "allowed_tags",
            &current_tags,
            &None,
            "one",
            CategoryListEdit::Remove,
        )
        .unwrap();
        assert_eq!(
            params,
            Some(vec![("allowed_tags[]".to_string(), String::new())])
        );
    }

    #[test]
    fn list_field_edit_params_rejects_non_list_field() {
        let err = list_field_edit_params("name", &None, &None, "x", CategoryListEdit::Append)
            .unwrap_err();
        assert!(err.to_string().contains("list fields"));
    }

    #[test]
    fn list_field_edit_params_rejects_empty_value() {
        let err =
            list_field_edit_params("allowed_tags", &None, &None, "", CategoryListEdit::Append)
                .unwrap_err();
        assert!(err.to_string().contains("non-empty"));
    }

    #[test]
    fn list_field_edit_params_skips_unchanged_list() {
        let current_tags = Some(vec!["one".to_string()]);
        assert_eq!(
            list_field_edit_params(
                "allowed_tags",
                &current_tags,
                &None,
                "one",
                CategoryListEdit::Append,
            )
            .unwrap(),
            None
        );
        assert_eq!(
            list_field_edit_params(
                "allowed_tags",
                &current_tags,
                &None,
                "missing",
                CategoryListEdit::Remove,
            )
            .unwrap(),
            None
        );
    }

    fn def(id: u64, name: &str) -> CategoryDefinition {
        CategoryDefinition {
            id: Some(id),
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn rename_plans_by_resolved_id() {
        let defs = vec![def(3, "General"), def(7, "Off Topic")];
        let (id, old, new) = plan_rename(&defs, "3", "Announcements").unwrap();
        assert_eq!(id, 3);
        assert_eq!(old, "General");
        assert_eq!(new, "Announcements");
    }

    #[test]
    fn rename_resolves_by_name_and_trims_new_name() {
        let defs = vec![def(3, "General")];
        let (id, _, new) = plan_rename(&defs, "General", "  Announcements  ").unwrap();
        assert_eq!(id, 3);
        assert_eq!(new, "Announcements");
    }

    #[test]
    fn rename_rejects_unknown_category() {
        let defs = vec![def(3, "General")];
        assert!(plan_rename(&defs, "nope", "New Name").is_err());
    }

    #[test]
    fn rename_rejects_empty_new_name() {
        let defs = vec![def(3, "General")];
        let err = plan_rename(&defs, "3", "   ").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn rename_rejects_identical_names() {
        let defs = vec![def(3, "General")];
        let err = plan_rename(&defs, "3", "General").unwrap_err();
        assert!(err.to_string().contains("identical"));
    }

    #[test]
    fn rename_rejects_name_already_used_by_another_category() {
        let defs = vec![def(3, "General"), def(7, "Off Topic")];
        let err = plan_rename(&defs, "3", "Off Topic").unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn rename_allows_a_name_used_under_another_parent() {
        let mut source = def(3, "General");
        source.parent_category_id = Some(1);
        let mut other = def(7, "Support");
        other.parent_category_id = Some(2);
        let (id, _, new) = plan_rename(&[source, other], "3", "Support").unwrap();
        assert_eq!(id, 3);
        assert_eq!(new, "Support");
    }

    #[test]
    fn rename_allows_case_change_only_via_self_match() {
        // Renaming a category to its own current name (post-trim) is still
        // treated as "identical", not as a self-collision error.
        let defs = vec![def(3, "General")];
        let err = plan_rename(&defs, "3", " General ").unwrap_err();
        assert!(err.to_string().contains("identical"));
    }

    #[test]
    fn entry_to_params_uses_pre_resolved_parent_id() {
        let mut e = entry("Child");
        e.parent = Some(Some("parent-cat".to_string()));
        let params = entry_to_params(&e, Some(42)).unwrap();
        assert!(params.contains(&("parent_category_id".to_string(), "42".to_string())));
    }

    #[test]
    fn entry_to_params_requires_a_resolved_parent_id() {
        let mut e = entry("Child");
        e.parent = Some(Some("Parent Cat".to_string()));
        let error = entry_to_params(&e, None).unwrap_err();
        assert!(error.to_string().contains("unresolved category parent"));
    }

    #[test]
    fn entry_to_params_clears_an_explicit_null_parent() {
        let mut category = entry("Child");
        category.parent = Some(None);
        let params = entry_to_params(&category, None).unwrap();
        assert!(params.contains(&("parent_category_id".to_string(), String::new())));
    }

    #[test]
    fn parent_deserialization_distinguishes_null_from_omission() {
        let explicit: CategoryDefEntry =
            serde_yaml::from_str("name: Child\nparent: null\n").unwrap();
        let omitted: CategoryDefEntry = serde_yaml::from_str("name: Child\n").unwrap();
        assert_eq!(explicit.parent, Some(None));
        assert_eq!(omitted.parent, None);
    }

    #[test]
    fn resolve_parent_id_prefers_slug_over_name_on_conflict() {
        let mut slug_to_ids = BTreeMap::new();
        slug_to_ids.insert("parent-cat".to_string(), vec![42u64]);
        let mut name_to_ids = BTreeMap::new();
        name_to_ids.insert("parent-cat".to_string(), vec![99u64]);
        assert_eq!(
            resolve_parent_id("parent-cat", &slug_to_ids, &name_to_ids).unwrap(),
            Some(42)
        );
    }

    #[test]
    fn resolve_parent_id_rejects_an_ambiguous_parent_name() {
        let mut name_to_ids = BTreeMap::new();
        name_to_ids.insert("Repeated Name".to_string(), vec![42u64, 99u64]);
        let err = resolve_parent_id("Repeated Name", &BTreeMap::new(), &name_to_ids).unwrap_err();
        assert!(err.to_string().contains("ambiguous"));
        assert!(err.to_string().contains("unambiguous slug"));
    }

    #[test]
    fn resolve_parent_targets_reports_every_unresolvable_entry() {
        let mut a = entry("Child A");
        a.parent = Some(Some("nope".to_string()));
        let mut b = entry("Child B");
        b.parent = Some(Some("also-nope".to_string()));
        let file = vec![a, b];
        let plan = plan_push(&file, &[]).unwrap();
        let err = resolve_parent_targets(&file, &[], &plan).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Child A"));
        assert!(msg.contains("nope"));
        assert!(msg.contains("Child B"));
        assert!(msg.contains("also-nope"));
    }

    #[test]
    fn resolve_parent_targets_accepts_a_brand_new_parent_defined_in_the_same_file() {
        let parent = entry("New Parent");
        let mut child = entry("Child");
        child.parent = Some(Some("New Parent".to_string()));
        let file = vec![parent, child];
        let plan = plan_push(&file, &[]).unwrap();
        assert_eq!(
            resolve_parent_targets(&file, &[], &plan).unwrap(),
            vec![None, Some(ParentTarget::File(0))]
        );
    }

    #[test]
    fn resolve_parent_targets_rejects_an_ambiguous_server_slug() {
        let mut first = entry("First Parent");
        first.id = Some(1);
        first.slug = Some("shared".to_string());
        let mut second = entry("Second Parent");
        second.id = Some(2);
        second.slug = Some("shared".to_string());
        let mut child = entry("Child");
        child.parent = Some(Some("shared".to_string()));
        let file = vec![child];
        let plan = plan_push(&file, &[first.clone(), second.clone()]).unwrap();
        let error = resolve_parent_targets(&file, &[first, second], &plan).unwrap_err();
        assert!(error.to_string().contains("ambiguous"));
        assert!(error.to_string().contains("server categories"));
    }

    #[test]
    fn resolve_parent_targets_uses_current_id_to_disambiguate_a_pulled_parent() {
        let mut root_one = entry("Root One");
        root_one.id = Some(1);
        root_one.slug = Some("root-one".to_string());
        let mut root_two = entry("Root Two");
        root_two.id = Some(2);
        root_two.slug = Some("root-two".to_string());
        let mut section_one = entry("Section");
        section_one.id = Some(3);
        section_one.slug = Some("section".to_string());
        section_one.parent = Some(Some("root-one".to_string()));
        section_one.server_parent_id = Some(1);
        let mut section_two = entry("Section");
        section_two.id = Some(4);
        section_two.slug = Some("section".to_string());
        section_two.parent = Some(Some("root-two".to_string()));
        section_two.server_parent_id = Some(2);
        let mut child = entry("Child");
        child.id = Some(5);
        child.slug = Some("child".to_string());
        child.parent = Some(Some("section".to_string()));
        child.server_parent_id = Some(3);
        let server = vec![root_one, root_two, section_one, section_two, child];
        let file = server.clone();
        let plan = plan_push(&file, &server).unwrap();
        let targets = resolve_parent_targets(&file, &server, &plan).unwrap();
        assert_eq!(targets[4], Some(ParentTarget::File(2)));
    }

    #[test]
    fn resolve_parent_targets_prefers_an_existing_numeric_id_over_aliases() {
        let mut by_id = entry("ID Parent");
        by_id.id = Some(42);
        by_id.slug = Some("id-parent".to_string());
        let mut by_slug = entry("Slug Parent");
        by_slug.id = Some(7);
        by_slug.slug = Some("42".to_string());
        let mut child = entry("Child");
        child.parent = Some(Some("42".to_string()));
        let file = vec![child];
        let server = vec![by_id, by_slug];
        let plan = plan_push(&file, &server).unwrap();
        let targets = resolve_parent_targets(&file, &server, &plan).unwrap();
        assert_eq!(targets[0], Some(ParentTarget::Server(42)));
    }

    #[test]
    fn order_for_push_places_a_same_file_parent_before_its_child() {
        let mut child = entry("Child");
        child.parent = Some(Some("Parent".to_string()));
        let parent = entry("Parent");
        // File declares the child first; the parent must still be created first.
        let file = vec![child, parent];
        let plan = plan_push(&file, &[]).unwrap();
        let targets = resolve_parent_targets(&file, &[], &plan).unwrap();
        let order = order_for_push(&file, &[], &plan, &targets).unwrap();
        assert_eq!(order, vec![1, 0]);
    }

    #[test]
    fn order_for_push_resolves_a_renamed_parent_by_its_new_slug() {
        let mut server_parent = entry("Existing Parent");
        server_parent.id = Some(9);
        server_parent.slug = Some("old-parent".to_string());
        let mut desired_parent = entry("Renamed Parent");
        desired_parent.id = Some(9);
        desired_parent.slug = Some("new-parent".to_string());
        let mut child = entry("Child");
        child.parent = Some(Some("new-parent".to_string()));
        let file = vec![child, desired_parent];
        let plan = plan_push(&file, std::slice::from_ref(&server_parent)).unwrap();
        let server = vec![server_parent];
        let targets = resolve_parent_targets(&file, &server, &plan).unwrap();
        assert_eq!(targets[0], Some(ParentTarget::File(1)));
        assert_eq!(
            order_for_push(&file, &server, &plan, &targets).unwrap(),
            vec![0, 1]
        );
    }

    #[test]
    fn order_for_push_leaves_independent_entries_in_file_order() {
        let a = entry("A");
        let b = entry("B");
        let file = vec![a, b];
        let plan = plan_push(&file, &[]).unwrap();
        let targets = vec![None, None];
        let order = order_for_push(&file, &[], &plan, &targets).unwrap();
        assert_eq!(order, vec![0, 1]);
    }

    #[test]
    fn order_for_push_releases_an_occupied_name_before_claiming_it() {
        let mut server_a = entry("A");
        server_a.id = Some(1);
        let mut server_b = entry("B");
        server_b.id = Some(2);
        let server = vec![server_a, server_b];
        let mut desired_a = entry("B");
        desired_a.id = Some(1);
        let mut desired_b = entry("C");
        desired_b.id = Some(2);
        let file = vec![desired_a, desired_b];
        let plan = plan_push(&file, &server).unwrap();
        let targets = vec![None, None];
        assert_eq!(
            order_for_push(&file, &server, &plan, &targets).unwrap(),
            vec![1, 0]
        );
    }

    #[test]
    fn order_for_push_moves_an_occupant_before_renaming_its_parent() {
        let mut server_parent = entry("Parent");
        server_parent.id = Some(1);
        let mut server_child = entry("Taken");
        server_child.id = Some(2);
        let server = vec![server_parent, server_child];
        let mut desired_parent = entry("Taken");
        desired_parent.id = Some(1);
        let mut desired_child = entry("Taken");
        desired_child.id = Some(2);
        desired_child.parent = Some(Some("1".to_string()));
        let file = vec![desired_parent, desired_child];
        let plan = plan_push(&file, &server).unwrap();
        let targets = resolve_parent_targets(&file, &server, &plan).unwrap();
        validate_desired_identities(&file, &server, &plan, &targets).unwrap();
        assert_eq!(
            order_for_push(&file, &server, &plan, &targets).unwrap(),
            vec![1, 0]
        );
    }

    #[test]
    fn order_for_push_detaches_a_parent_before_inverting_the_relationship() {
        let mut server_a = entry("A");
        server_a.id = Some(1);
        server_a.server_parent_id = Some(2);
        let mut server_b = entry("B");
        server_b.id = Some(2);
        let server = vec![server_a, server_b];
        let mut desired_b = entry("B");
        desired_b.id = Some(2);
        desired_b.parent = Some(Some("1".to_string()));
        let mut desired_a = entry("A");
        desired_a.id = Some(1);
        desired_a.parent = Some(None);
        let file = vec![desired_b, desired_a];
        let mut plan = plan_push(&file, &server).unwrap();
        let targets = resolve_parent_targets(&file, &server, &plan).unwrap();
        reconcile_parent_changes(&file, &server, &mut plan, &targets);
        validate_hierarchy(&file, &[], &plan, &targets).unwrap();
        assert_eq!(
            order_for_push(&file, &server, &plan, &targets).unwrap(),
            vec![1, 0]
        );
    }

    #[test]
    fn order_for_push_simulates_deeper_hierarchy_moves() {
        let mut server_a = entry("A");
        server_a.id = Some(1);
        let mut server_b = entry("B");
        server_b.id = Some(2);
        server_b.server_parent_id = Some(1);
        let mut server_c = entry("C");
        server_c.id = Some(3);
        server_c.server_parent_id = Some(1);
        let server = vec![server_a, server_b, server_c];

        let mut desired_b = entry("B");
        desired_b.id = Some(2);
        desired_b.parent = Some(Some("3".to_string()));
        let mut desired_a = entry("A");
        desired_a.id = Some(1);
        desired_a.parent = Some(Some("2".to_string()));
        let mut desired_c = entry("C");
        desired_c.id = Some(3);
        desired_c.parent = Some(None);
        let file = vec![desired_b, desired_a, desired_c];

        let mut plan = plan_push(&file, &server).unwrap();
        let targets = resolve_parent_targets(&file, &server, &plan).unwrap();
        reconcile_parent_changes(&file, &server, &mut plan, &targets);
        assert_eq!(
            order_for_push(&file, &server, &plan, &targets).unwrap(),
            vec![0, 2, 1]
        );
    }

    #[test]
    fn validate_desired_identities_rejects_duplicates_under_the_same_parent() {
        let parent = entry("Parent");
        let mut first = entry("Duplicate");
        first.parent = Some(Some("Parent".to_string()));
        let mut second = entry("Duplicate");
        second.parent = Some(Some("Parent".to_string()));
        let file = vec![parent, first, second];
        let plan = plan_push(&file, &[]).unwrap();
        let targets = resolve_parent_targets(&file, &[], &plan).unwrap();
        let error = validate_desired_identities(&file, &[], &plan, &targets).unwrap_err();
        assert!(error.to_string().contains("duplicate name"));
    }

    #[test]
    fn validate_desired_identities_includes_untouched_server_categories() {
        let mut existing = entry("General");
        existing.id = Some(1);
        let mut renamed = entry("Other");
        renamed.id = Some(2);
        let server = vec![existing, renamed];
        let mut desired = entry("general");
        desired.id = Some(2);
        let file = vec![desired];
        let plan = plan_push(&file, &server).unwrap();
        let targets = vec![None];
        let error = validate_desired_identities(&file, &server, &plan, &targets).unwrap_err();
        assert!(error.to_string().contains("duplicate name"));
        assert!(error.to_string().contains("General"));
    }

    #[test]
    fn validate_hierarchy_detects_a_two_entry_create_cycle() {
        let mut a = entry("A");
        a.parent = Some(Some("B".to_string()));
        let mut b = entry("B");
        b.parent = Some(Some("A".to_string()));
        let file = vec![a, b];
        let plan = plan_push(&file, &[]).unwrap();
        let targets = resolve_parent_targets(&file, &[], &plan).unwrap();
        let err = validate_hierarchy(&file, &[], &plan, &targets).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("circular"));
        assert!(msg.contains('A'));
        assert!(msg.contains('B'));
    }

    #[test]
    fn validate_hierarchy_detects_a_cycle_through_an_unchanged_server_category() {
        let mut a = def(1, "A");
        a.slug = Some("a".to_string());
        let mut b = def(2, "B");
        b.slug = Some("b".to_string());
        b.parent_category_id = Some(1);
        let defs = vec![a, b];
        let id_to_slug = id_to_slug_map(&defs);
        let server: Vec<_> = defs
            .iter()
            .map(|def| def_to_entry(def, &id_to_slug))
            .collect();
        let mut desired_a = entry("A");
        desired_a.id = Some(1);
        desired_a.parent = Some(Some("b".to_string()));
        let file = vec![desired_a];
        let plan = plan_push(&file, &server).unwrap();
        let targets = resolve_parent_targets(&file, &server, &plan).unwrap();
        let error = validate_hierarchy(&file, &defs, &plan, &targets).unwrap_err();
        assert!(error.to_string().contains("A, B"));
    }

    #[test]
    fn validate_hierarchy_detects_self_parenting() {
        let mut category = entry("A");
        category.parent = Some(Some("A".to_string()));
        let file = vec![category];
        let plan = plan_push(&file, &[]).unwrap();
        let targets = resolve_parent_targets(&file, &[], &plan).unwrap();
        let error = validate_hierarchy(&file, &[], &plan, &targets).unwrap_err();
        assert!(error.to_string().contains("A"));
    }

    #[test]
    fn validate_hierarchy_honours_an_explicit_top_level_move() {
        let mut a = def(1, "A");
        a.slug = Some("a".to_string());
        a.parent_category_id = Some(2);
        let mut b = def(2, "B");
        b.slug = Some("b".to_string());
        let defs = vec![a, b];
        let id_to_slug = id_to_slug_map(&defs);
        let server: Vec<_> = defs
            .iter()
            .map(|def| def_to_entry(def, &id_to_slug))
            .collect();
        let mut desired_a = entry("A");
        desired_a.id = Some(1);
        desired_a.parent = Some(None);
        let mut desired_b = entry("B");
        desired_b.id = Some(2);
        desired_b.parent = Some(Some("a".to_string()));
        let file = vec![desired_a, desired_b];
        let plan = plan_push(&file, &server).unwrap();
        let targets = resolve_parent_targets(&file, &server, &plan).unwrap();
        validate_hierarchy(&file, &defs, &plan, &targets).unwrap();
    }

    #[test]
    fn diff_finds_no_rows_for_identical_categories() {
        let mut e = entry("General");
        e.slug = Some("general".to_string());
        e.color = Some("ABABAB".to_string());
        assert!(diff_entries(&e, &e).unwrap().is_empty());
    }

    #[test]
    fn diff_flags_a_changed_string_field() {
        let mut ea = entry("General");
        ea.slug = Some("general".to_string());
        ea.color = Some("ABABAB".to_string());
        let mut eb = ea.clone();
        eb.color = Some("FF0000".to_string());
        let rows = diff_entries(&ea, &eb).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].field, "color");
        assert_eq!(rows[0].a, Some(json!("ABABAB")));
        assert_eq!(rows[0].b, Some(json!("FF0000")));
    }

    #[test]
    fn diff_reports_name_and_slug_changes_between_explicit_categories() {
        let mut ea = entry("General");
        ea.slug = Some("general".to_string());
        let mut eb = entry("Announcements");
        eb.slug = Some("announcements".to_string());
        let rows = diff_entries(&ea, &eb).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].field, "name");
        assert_eq!(rows[1].field, "slug");
    }

    #[test]
    fn diff_preserves_native_json_types() {
        let mut ea = entry("General");
        ea.read_restricted = Some(false);
        ea.minimum_required_tags = Some(1);
        ea.allowed_tags = Some(vec!["one".to_string()]);
        let mut eb = ea.clone();
        eb.read_restricted = Some(true);
        eb.minimum_required_tags = Some(2);
        eb.allowed_tags = Some(vec!["one".to_string(), "two".to_string()]);

        let rows = diff_entries(&ea, &eb).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].a, Some(json!(false)));
        assert_eq!(rows[0].b, Some(json!(true)));
        assert_eq!(rows[1].a, Some(json!(["one"])));
        assert_eq!(rows[1].b, Some(json!(["one", "two"])));
        assert_eq!(rows[2].a, Some(json!(1)));
        assert_eq!(rows[2].b, Some(json!(2)));
    }

    #[test]
    fn diff_ignores_unset_fields_on_both_sides() {
        // Neither side sets `description` - not a difference, just both unset.
        let ea = entry("General");
        let eb = ea.clone();
        assert!(diff_entries(&ea, &eb).unwrap().is_empty());
    }

    #[test]
    fn diff_represents_a_field_unset_on_one_side_as_none() {
        let mut ea = entry("General");
        ea.description = Some("Welcome".to_string());
        let eb = entry("General");
        let rows = diff_entries(&ea, &eb).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].field, "description");
        assert_eq!(rows[0].a, Some(json!("Welcome")));
        assert_eq!(rows[0].b, None);
    }
}

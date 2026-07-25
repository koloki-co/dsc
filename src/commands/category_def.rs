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
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// The definition fields a user may `get`/`set` and that appear in the file.
const VALID_FIELDS: &[&str] = &[
    "name",
    "slug",
    "color",
    "text_color",
    "position",
    "parent",
    "read_restricted",
    "description",
    "topic_template",
    "permissions",
    "allowed_tags",
    "allowed_tag_groups",
    "minimum_required_tags",
    "sort_order",
    "default_view",
    "subcategory_list_style",
    "num_featured_topics",
    "show_subcategory_list",
];

// ─── File schema (version 1) ──────────────────────────────────────────────────

/// The on-disk `categories.yaml` (or `.json`) document.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CategoriesFile {
    pub version: u32,
    #[serde(default)]
    pub categories: Vec<CategoryDefEntry>,
}

/// One category's definition in the file. Every field beyond `name` is optional;
/// an omitted field is left untouched on push.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    /// Parent category slug (or null for a top-level category).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_restricted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_template: Option<String>,
    /// group_name -> level (`full` | `create_post` | `readonly`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tag_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_required_tags: Option<u64>,
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
    let parent = def
        .parent_category_id
        .and_then(|pid| id_to_slug.get(&pid).cloned());
    let nonempty = |s: &Option<String>| s.clone().filter(|v| !v.is_empty());
    let nonempty_list = |v: &Option<Vec<String>>| v.clone().filter(|l| !l.is_empty());

    CategoryDefEntry {
        name: def.name.clone(),
        id: def.id,
        slug: nonempty(&def.slug),
        color: nonempty(&def.color),
        text_color: nonempty(&def.text_color),
        position: def.position,
        parent,
        read_restricted: def.read_restricted,
        // Prefer the plain-text description over the cooked HTML `description`
        // so pull -> push -> pull is idempotent (see CategoryDefinition).
        description: nonempty(&def.description_text).or_else(|| nonempty(&def.description)),
        topic_template: nonempty(&def.topic_template),
        permissions,
        allowed_tags: nonempty_list(&def.allowed_tags),
        allowed_tag_groups: nonempty_list(&def.allowed_tag_groups),
        minimum_required_tags: def.minimum_required_tags.filter(|n| *n > 0),
        sort_order: nonempty(&def.sort_order),
        default_view: nonempty(&def.default_view),
        subcategory_list_style: nonempty(&def.subcategory_list_style),
        num_featured_topics: def.num_featured_topics,
        show_subcategory_list: def.show_subcategory_list,
    }
}

fn id_to_slug_map(defs: &[CategoryDefinition]) -> BTreeMap<u64, String> {
    defs.iter()
        .filter_map(|d| match (d.id, &d.slug) {
            (Some(id), Some(slug)) => Some((id, slug.clone())),
            _ => None,
        })
        .collect()
}

fn slug_to_id_map(defs: &[CategoryDefinition]) -> BTreeMap<String, u64> {
    defs.iter()
        .filter_map(|d| match (&d.slug, d.id) {
            (Some(slug), Some(id)) => Some((slug.clone(), id)),
            _ => None,
        })
        .collect()
}

/// Build the form params for a whole entry (create or full update).
fn entry_to_params(
    entry: &CategoryDefEntry,
    slug_to_id: &BTreeMap<String, u64>,
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
    if let Some(v) = entry.position {
        p.push(("position".to_string(), v.to_string()));
    }
    if let Some(parent) = &entry.parent {
        let pid = slug_to_id.get(parent).ok_or_else(|| {
            anyhow!(
                "parent category '{}' not found on the server (create it first, or fix the slug)",
                parent
            )
        })?;
        p.push(("parent_category_id".to_string(), pid.to_string()));
    }
    if let Some(v) = entry.read_restricted {
        p.push(("read_restricted".to_string(), v.to_string()));
    }
    push_opt(&mut p, "description", &entry.description);
    push_opt(&mut p, "topic_template", &entry.topic_template);
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
    /// A no-`id` file entry that matched nothing: `def push` would CREATE it,
    /// but if the user meant to rename an existing category this would orphan
    /// its topics. Warned in the plan.
    rename_warning: bool,
}

/// Match a file entry to a server entry: by `id`, else `slug`, else `name`.
fn match_server<'a>(
    e: &CategoryDefEntry,
    server: &'a [CategoryDefEntry],
) -> (Option<&'a CategoryDefEntry>, bool) {
    if let Some(id) = e.id {
        // id given but absent -> treat as create, no rename ambiguity.
        return (server.iter().find(|s| s.id == Some(id)), false);
    }
    if let Some(sl) = &e.slug
        && let Some(s) = server
            .iter()
            .find(|s| s.slug.as_deref() == Some(sl.as_str()))
    {
        return (Some(s), false);
    }
    if let Some(s) = server.iter().find(|s| s.name == e.name) {
        return (Some(s), false);
    }
    (None, true)
}

fn opt_diff<T: PartialEq>(a: &Option<T>, b: &Option<T>) -> bool {
    a.is_some() && a != b
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

/// Does the file entry specify any field that differs from the server entry?
/// Only fields the file actually sets are compared - omitted fields are ignored.
fn differs(e: &CategoryDefEntry, s: &CategoryDefEntry) -> bool {
    e.name != s.name
        || opt_diff(&e.slug, &s.slug)
        || opt_diff(&e.color, &s.color)
        || opt_diff(&e.text_color, &s.text_color)
        || opt_diff(&e.position, &s.position)
        || opt_diff(&e.parent, &s.parent)
        || opt_diff(&e.read_restricted, &s.read_restricted)
        || opt_diff(&e.description, &s.description)
        || opt_diff(&e.topic_template, &s.topic_template)
        || opt_diff(&e.permissions, &s.permissions)
        || opt_list_diff(&e.allowed_tags, &s.allowed_tags)
        || opt_list_diff(&e.allowed_tag_groups, &s.allowed_tag_groups)
        || opt_diff(&e.minimum_required_tags, &s.minimum_required_tags)
        || opt_diff(&e.sort_order, &s.sort_order)
        || opt_diff(&e.default_view, &s.default_view)
        || opt_diff(&e.subcategory_list_style, &s.subcategory_list_style)
        || opt_diff(&e.num_featured_topics, &s.num_featured_topics)
        || opt_diff(&e.show_subcategory_list, &s.show_subcategory_list)
}

/// Classify each file entry against the server (upsert; never delete).
fn plan_push(file: &[CategoryDefEntry], server: &[CategoryDefEntry]) -> Vec<DefAction> {
    file.iter()
        .map(|e| {
            let (matched, rename_warning) = match_server(e, server);
            let (kind, server_id) = match matched {
                Some(s) => (
                    if differs(e, s) {
                        DefActionKind::Update
                    } else {
                        DefActionKind::Unchanged
                    },
                    s.id,
                ),
                None => (DefActionKind::Create, None),
            };
            DefAction {
                name: e.name.clone(),
                kind,
                server_id,
                rename_warning,
            }
        })
        .collect()
}

// ─── Commands: def pull / def push ────────────────────────────────────────────

fn is_json_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

pub fn category_def_pull(
    config: &Config,
    discourse_name: &str,
    local_path: Option<&Path>,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let defs = client.fetch_category_definitions()?;
    let id_to_slug = id_to_slug_map(&defs);

    let mut entries: Vec<CategoryDefEntry> =
        defs.iter().map(|d| def_to_entry(d, &id_to_slug)).collect();
    // Stable order for clean diffs: by position, then name.
    entries.sort_by(|a, b| {
        a.position
            .unwrap_or(i64::MAX)
            .cmp(&b.position.unwrap_or(i64::MAX))
            .then_with(|| a.name.cmp(&b.name))
    });

    let file = CategoriesFile {
        version: 1,
        categories: entries,
    };

    let default_path = Path::new("categories.yaml");
    let path = local_path.unwrap_or(default_path);
    let content = if is_json_path(path) {
        serde_json::to_string_pretty(&file).context("serializing categories as JSON")?
    } else {
        serde_yaml::to_string(&file).context("serializing categories as YAML")?
    };
    fs::write(path, &content).with_context(|| format!("writing {}", path.display()))?;
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
    let file: CategoriesFile = if is_json_path(local_path) {
        serde_json::from_str(&content).context("parsing categories JSON")?
    } else {
        serde_yaml::from_str(&content).context("parsing categories YAML")?
    };
    if file.version != 1 {
        anyhow::bail!("unsupported categories file version: {}", file.version);
    }

    let defs = client.fetch_category_definitions()?;
    let id_to_slug = id_to_slug_map(&defs);
    let slug_to_id = slug_to_id_map(&defs);
    let server_entries: Vec<CategoryDefEntry> =
        defs.iter().map(|d| def_to_entry(d, &id_to_slug)).collect();

    let plan = plan_push(&file.categories, &server_entries);

    if dry_run {
        println!(
            "[dry-run] Category definition plan for '{}':",
            discourse_name
        );
        let mut changes = 0;
        for action in &plan {
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
                    println!("  ~ update: {}", action.name);
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

    for (entry, action) in file.categories.iter().zip(&plan) {
        match action.kind {
            DefActionKind::Create => {
                let params = entry_to_params(entry, &slug_to_id)?;
                let id = client
                    .create_category_def(&params)
                    .with_context(|| format!("creating category '{}'", entry.name))?;
                println!("  + created: {} (id {})", entry.name, id);
            }
            DefActionKind::Update => {
                let id = action
                    .server_id
                    .ok_or_else(|| anyhow!("internal: update without a server id"))?;
                let params = entry_to_params(entry, &slug_to_id)?;
                client
                    .update_category(id, &params)
                    .with_context(|| format!("updating category '{}'", entry.name))?;
                println!("  ~ updated: {} (id {})", entry.name, id);
            }
            DefActionKind::Unchanged => {}
        }
    }
    println!("Push complete.");
    Ok(())
}

// ─── Commands: show / get / set ───────────────────────────────────────────────

fn find_def<'a>(defs: &'a [CategoryDefinition], category: &str) -> Result<&'a CategoryDefinition> {
    if let Ok(id) = category.parse::<u64>() {
        return defs
            .iter()
            .find(|d| d.id == Some(id))
            .ok_or_else(|| not_found("category", category));
    }
    defs.iter()
        .find(|d| d.slug.as_deref() == Some(category))
        .or_else(|| defs.iter().find(|d| d.name == category))
        .ok_or_else(|| not_found("category", category))
}

fn resolve_entry(client: &DiscourseClient, category: &str) -> Result<CategoryDefEntry> {
    let defs = client.fetch_category_definitions()?;
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
    let entry = resolve_entry(&client, category)?;
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
    let entry = resolve_entry(&client, category)?;
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
    let out = match field.trim() {
        "name" => (e.name.clone(), json!(e.name)),
        "slug" => optstr(&e.slug),
        "color" => optstr(&e.color),
        "text_color" => optstr(&e.text_color),
        "position" => match e.position {
            Some(n) => (n.to_string(), json!(n)),
            None => ("(unset)".to_string(), Value::Null),
        },
        "parent" => optstr(&e.parent),
        "read_restricted" => optbool(e.read_restricted),
        "description" => optstr(&e.description),
        "topic_template" => optstr(&e.topic_template),
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

pub fn category_set(
    config: &Config,
    discourse_name: &str,
    category: &str,
    field: &str,
    value: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let defs = client.fetch_category_definitions()?;
    let slug_to_id = slug_to_id_map(&defs);
    let def = find_def(&defs, category)?;
    let id = def.id.ok_or_else(|| not_found("category", category))?;

    let params = field_to_set_params(field, value, &slug_to_id)?;

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

/// Build the form params for setting a single field.
fn field_to_set_params(
    field: &str,
    value: &str,
    slug_to_id: &BTreeMap<String, u64>,
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
        "position" => {
            value
                .parse::<i64>()
                .with_context(|| format!("position must be an integer, got '{}'", value))?;
            one("position", value.to_string())
        }
        "parent" => {
            let slug = value.trim();
            let pid = slug_to_id
                .get(slug)
                .ok_or_else(|| anyhow!("parent category '{}' not found on the server", slug))?;
            one("parent_category_id", pid.to_string())
        }
        "read_restricted" => one("read_restricted", parse_bool(value)?.to_string()),
        "description" => one("description", value.to_string()),
        "topic_template" => one("topic_template", value.to_string()),
        "minimum_required_tags" => {
            value.parse::<u64>().with_context(|| {
                format!("minimum_required_tags must be an integer, got '{}'", value)
            })?;
            one("minimum_required_tags", value.to_string())
        }
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
        let plan = plan_push(&file, &[]);
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
        let plan = plan_push(&[file], &[server]);
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
        let plan = plan_push(&[file], &[server]);
        assert_eq!(plan[0].kind, DefActionKind::Unchanged);
    }

    #[test]
    fn plan_matches_by_slug_without_id() {
        let mut server = entry("General");
        server.id = Some(3);
        server.slug = Some("general".to_string());
        let mut file = entry("General");
        file.slug = Some("general".to_string());
        let plan = plan_push(&[file], &[server]);
        assert_eq!(plan[0].kind, DefActionKind::Unchanged);
        assert!(!plan[0].rename_warning);
    }

    #[test]
    fn differs_ignores_fields_the_file_omits() {
        let mut server = entry("General");
        server.description = Some("server desc".to_string());
        server.color = Some("ABABAB".to_string());
        // File omits description and color -> not a change.
        let file = entry("General");
        assert!(!differs(&file, &server));
    }

    #[test]
    fn differs_flags_specified_mismatch() {
        let mut server = entry("General");
        server.color = Some("ABABAB".to_string());
        let mut file = entry("General");
        file.color = Some("FF0000".to_string());
        assert!(differs(&file, &server));
    }

    #[test]
    fn differs_compares_lists_order_insensitively() {
        let mut server = entry("General");
        server.allowed_tags = Some(vec!["b".to_string(), "a".to_string()]);
        let mut file = entry("General");
        file.allowed_tags = Some(vec!["a".to_string(), "b".to_string()]);
        assert!(!differs(&file, &server));
    }

    #[test]
    fn set_params_permissions_imply_read_restricted() {
        let params = field_to_set_params("permissions", "staff:full", &BTreeMap::new()).unwrap();
        assert!(params.contains(&("permissions[staff]".to_string(), "1".to_string())));
        assert!(params.contains(&("read_restricted".to_string(), "true".to_string())));
    }

    #[test]
    fn set_params_everyone_only_stays_public() {
        let params = field_to_set_params("permissions", "everyone:full", &BTreeMap::new()).unwrap();
        assert!(params.contains(&("permissions[everyone]".to_string(), "1".to_string())));
        assert!(!params.iter().any(|(k, _)| k == "read_restricted"));
    }

    #[test]
    fn set_params_unknown_field_errors() {
        let err = field_to_set_params("bogus", "x", &BTreeMap::new()).unwrap_err();
        assert!(err.to_string().contains("unknown category field"));
    }

    #[test]
    fn set_params_list_clears_on_empty() {
        let params = field_to_set_params("allowed_tags", "", &BTreeMap::new()).unwrap();
        assert_eq!(params, vec![("allowed_tags[]".to_string(), String::new())]);
    }

    #[test]
    fn entry_to_params_resolves_parent_slug() {
        let mut slug_to_id = BTreeMap::new();
        slug_to_id.insert("parent-cat".to_string(), 42u64);
        let mut e = entry("Child");
        e.parent = Some("parent-cat".to_string());
        let params = entry_to_params(&e, &slug_to_id).unwrap();
        assert!(params.contains(&("parent_category_id".to_string(), "42".to_string())));
    }

    #[test]
    fn entry_to_params_unknown_parent_errors() {
        let mut e = entry("Child");
        e.parent = Some("nope".to_string());
        assert!(entry_to_params(&e, &BTreeMap::new()).is_err());
    }
}

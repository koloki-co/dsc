// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{CategoryInfo, DiscourseClient, PostEditOptions, TopicSummary};
use crate::cli::{AdmonitionStyle, ListFormat};
use crate::commands::common::{ensure_api_credentials, not_found, select_discourse};
use crate::config::Config;
use crate::utils::{
    current_utc_iso8601, ensure_dir, ensure_output_available, normalize_baseurl, read_markdown,
    slugify, strip_frontmatter, write_markdown_output, yaml_scalar,
};
use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn category_list(
    config: &Config,
    discourse_name: &str,
    format: ListFormat,
    verbose: bool,
    tree: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let categories = client.fetch_categories()?;
    let mut flat = Vec::new();
    for category in categories {
        flatten_categories(&category, &mut flat);
    }
    match format {
        ListFormat::Text => {
            if tree {
                if flat.is_empty() && !verbose {
                    println!("No categories found.");
                    return Ok(());
                }
                print_category_tree(&flat);
            } else {
                let unique = unique_categories(flat);
                if unique.is_empty() && !verbose {
                    println!("No categories found.");
                    return Ok(());
                }
                for category in unique {
                    let id = category.id.unwrap_or_default();
                    println!("{} - {}", id, category.name);
                }
            }
        }
        ListFormat::Json => {
            if tree {
                return Err(anyhow!("--tree is only supported with --format text"));
            }
            let unique = unique_categories(flat);
            let raw = serde_json::to_string_pretty(&unique)?;
            println!("{}", raw);
        }
        ListFormat::Yaml => {
            if tree {
                return Err(anyhow!("--tree is only supported with --format text"));
            }
            let unique = unique_categories(flat);
            let raw = serde_yaml::to_string(&unique)?;
            println!("{}", raw);
        }
    }
    Ok(())
}

pub fn category_copy(
    config: &Config,
    source: &str,
    target: Option<&str>,
    category: &str,
    dry_run: bool,
) -> Result<()> {
    let source_discourse = select_discourse(config, Some(source))?;
    let target_name = target.unwrap_or(source);
    let target_discourse = select_discourse(config, Some(target_name))?;
    ensure_api_credentials(source_discourse)?;
    ensure_api_credentials(target_discourse)?;
    let source_client = DiscourseClient::new(source_discourse)?;
    let (category_id, maybe_category) = resolve_category_with_object(&source_client, category)?;
    let category = if let Some(cat) = maybe_category {
        cat
    } else {
        let categories = source_client.fetch_categories()?;
        categories
            .into_iter()
            .find(|cat| cat.id == Some(category_id))
            .ok_or_else(|| not_found("category", category_id))?
    };
    let mut copied = category.clone();
    copied.name = format!("Copy of {}", category.name);
    copied.slug = format!("{}-copy", category.slug);
    copied.id = None;
    if dry_run {
        println!(
            "[dry-run] would create category \"{}\" (slug: {}) on {}",
            copied.name, copied.slug, target_discourse.name
        );
        return Ok(());
    }
    let target_client = DiscourseClient::new(target_discourse)?;
    let new_id = target_client.create_category(&copied)?;
    let url = format!(
        "{}/c/{}",
        normalize_baseurl(&target_discourse.baseurl),
        new_id
    );
    println!("{}", url);
    Ok(())
}

pub fn category_pull(
    config: &Config,
    discourse_name: &str,
    category: &str,
    local_path: Option<&Path>,
    admonition_style: Option<AdmonitionStyle>,
    rewrite_links: bool,
    force: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let category_id = resolve_category_id(&client, category)?;
    let category = client.fetch_category(category_id)?;
    let dir = match local_path {
        Some(path) => path.to_path_buf(),
        None => {
            let name = category
                .category
                .as_ref()
                .map(|c| c.slug.clone())
                .unwrap_or_else(|| format!("category-{}", category_id));
            std::env::current_dir()?.join(name)
        }
    };
    ensure_dir(&dir)?;
    let topics = category.topic_list.topics;
    let local_paths = category_local_paths(&topics);
    for filename in local_paths.values() {
        ensure_output_available(&dir.join(filename), force)?;
    }
    for topic in topics {
        let topic_detail = client.fetch_topic(topic.id, true)?;
        let raw = topic_detail
            .post_stream
            .posts
            .first()
            .and_then(|p| p.raw.clone())
            .unwrap_or_default();
        let body = match admonition_style {
            Some(style) => convert_discourse_admonitions(&raw, style),
            None => raw,
        };
        let body = if rewrite_links {
            rewrite_forum_links(&body, &discourse.baseurl, &local_paths)
        } else {
            body
        };
        let filename = local_paths
            .get(&topic.id)
            .ok_or_else(|| anyhow!("missing local filename for topic {}", topic.id))?;
        let contents = render_category_topic(&topic, &discourse.baseurl, &body);
        write_markdown_output(&dir.join(filename), &contents, force)?;
    }
    println!("{}", dir.display());
    Ok(())
}

fn category_local_paths(topics: &[TopicSummary]) -> HashMap<u64, String> {
    let mut slug_counts: HashMap<String, usize> = HashMap::new();
    for topic in topics {
        *slug_counts.entry(slugify(&topic.title)).or_default() += 1;
    }
    topics
        .iter()
        .map(|topic| {
            let slug = slugify(&topic.title);
            let filename = if slug_counts.get(&slug).copied().unwrap_or(0) > 1 {
                format!("{}-{}.md", slug, topic.id)
            } else {
                format!("{}.md", slug)
            };
            (topic.id, filename)
        })
        .collect()
}

/// Render one pulled topic as a Markdown document with YAML front matter
/// (`title` / `topic_id` / `url` / `pulled_at`) followed by the OP's raw body.
///
/// The `topic_id` is the durable binding `category push` routes on, so edits
/// to the title or filename no longer risk creating a duplicate topic. Mirrors
/// the front matter written by `topic pull --full`.
fn render_category_topic(topic: &TopicSummary, baseurl: &str, raw: &str) -> String {
    let base = normalize_baseurl(baseurl);
    let url = format!("{}/t/{}/{}", base, topic.slug, topic.id);
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("title: {}\n", yaml_scalar(&topic.title)));
    out.push_str(&format!("topic_id: {}\n", topic.id));
    out.push_str(&format!("url: {}\n", url));
    out.push_str(&format!("pulled_at: {}\n", current_utc_iso8601()));
    out.push_str("---\n\n");
    out.push_str(raw.trim_end());
    out.push('\n');
    out
}

/// Controls the non-content behaviour of [`category_push`]. Grouped so the
/// command boundary remains stable as category sync gains opt-in transforms.
#[derive(Clone, Copy)]
pub struct CategoryPushOptions {
    /// Refuse to create a topic for a file without a remote match.
    pub updates_only: bool,
    /// Whether successful post edits should bump a topic or record a revision.
    pub edit: PostEditOptions,
    /// Optional local MkDocs/Zensical admonition conversion before push.
    pub admonition_style: Option<AdmonitionStyle>,
    /// Rewrite local Markdown links to same-category forum topic URLs.
    pub rewrite_links: bool,
    /// Render `{{ variable }}` template placeholders in each file before pushing.
    pub render: bool,
}

/// One planned change for `category push`, decided before any mutation so
/// the whole plan can be printed and reviewed up front. This satisfies the
/// governance requirement driving the spec: never push without a reviewable
/// plan, never create a topic without deliberate intent.
enum PushAction {
    /// Body differs from the remote OP; update post `post_id`.
    Update {
        path: PathBuf,
        topic_id: u64,
        post_id: u64,
        body: String,
    },
    /// Body matches the remote OP (modulo trailing whitespace); no write.
    Unchanged { path: PathBuf, topic_id: u64 },
    /// No remote match; create a new topic (skipped under `--updates-only`).
    Create {
        path: PathBuf,
        title: String,
        body: String,
    },
}

pub fn category_push(
    config: &Config,
    discourse_name: &str,
    category: &str,
    local_path: &Path,
    dry_run: bool,
    options: CategoryPushOptions,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let category_id = resolve_category_id(&client, category)?;
    let existing = client.fetch_category(category_id)?;
    let topics = existing.topic_list.topics;
    // Build a topic-ID index once so membership checks are O(1) rather
    // than O(topics) per file in the planning loop below.
    let topic_ids: std::collections::HashSet<u64> = topics.iter().map(|t| t.id).collect();

    // Collect Markdown files in a stable order so the plan is deterministic.
    let mut paths: Vec<PathBuf> = fs::read_dir(local_path)
        .with_context(|| format!("reading {}", local_path.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    paths.sort();
    let links = if options.rewrite_links {
        local_topic_links(&paths, &topics)?
    } else {
        HashMap::new()
    };

    // Plan first: decide every action (and fail any --updates-only mismatch)
    // before mutating anything on the server.
    let mut plan = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = read_markdown(&path)?;
        let raw = if options.render {
            crate::commands::render::render_content(config, discourse, &raw)?
        } else {
            raw
        };
        let (front, body) = strip_frontmatter(&raw);
        let body = match options.admonition_style {
            Some(style) => convert_mkdocs_admonitions(&body, style),
            None => body,
        };
        let (body, unresolved_links) = if options.rewrite_links {
            rewrite_relative_links(&body, &discourse.baseurl, &links)
        } else {
            (body, Vec::new())
        };
        for link in unresolved_links {
            eprintln!(
                "warning: {}: no category topic matches relative link {}",
                path.display(),
                link
            );
        }
        let title = front
            .get("title")
            .cloned()
            .or_else(|| extract_title(&body))
            .unwrap_or_else(|| path.file_stem().unwrap().to_string_lossy().to_string());

        let topic_id = route_topic_id(&front, &title, &path, &topics);

        match topic_id {
            Some(id) => {
                let detail = client.fetch_topic(id, true)?;
                let listed_in_category = topic_ids.contains(&id);
                validate_topic_category(id, detail.category_id, category_id, listed_in_category)?;
                let post = detail
                    .post_stream
                    .posts
                    .first()
                    .ok_or_else(|| anyhow!("topic {} has no posts", id))?;
                let remote = post.raw.as_deref().unwrap_or_default();
                if remote.trim_end() == body.trim_end() {
                    plan.push(PushAction::Unchanged { path, topic_id: id });
                } else {
                    plan.push(PushAction::Update {
                        path,
                        topic_id: id,
                        post_id: post.id,
                        body,
                    });
                }
            }
            None => {
                if options.updates_only {
                    return Err(anyhow!(
                        "no matching topic for {} (title: {:?})\n\
                         hint: remove --updates-only to allow new topic creation, \
                         or check the filename/topic_id matches an existing topic",
                        path.display(),
                        title
                    ));
                }
                plan.push(PushAction::Create { path, title, body });
            }
        }
    }

    print_push_plan(&plan, &discourse.name, category_id, dry_run);

    if !dry_run {
        for action in &plan {
            match action {
                PushAction::Update { post_id, body, .. } => {
                    client.update_post(*post_id, body, options.edit)?;
                }
                PushAction::Create { title, body, .. } => {
                    client.create_topic(category_id, title, body)?;
                }
                PushAction::Unchanged { .. } => {}
            }
        }
    }

    Ok(())
}

fn validate_topic_category(
    topic_id: u64,
    actual_category_id: Option<u64>,
    target_category_id: u64,
    listed_in_category: bool,
) -> Result<()> {
    match actual_category_id {
        Some(actual) if actual == target_category_id => Ok(()),
        Some(actual) => Err(anyhow!(
            "topic {} belongs to category {}, not target category {}; refusing cross-category update",
            topic_id,
            actual,
            target_category_id
        )),
        None if listed_in_category => Ok(()),
        None => Err(anyhow!(
            "cannot verify that topic {} belongs to target category {}; refusing update",
            topic_id,
            target_category_id
        )),
    }
}

/// Print the push plan using the same `~` (change) / `+` (create) /
/// `=` (unchanged) sigils as `setting push`, prefixed with `[dry-run]`
/// when nothing will actually be written.
fn print_push_plan(plan: &[PushAction], discourse: &str, category_id: u64, dry_run: bool) {
    let prefix = if dry_run { "[dry-run] " } else { "" };
    let updates = plan
        .iter()
        .filter(|a| matches!(a, PushAction::Update { .. }))
        .count();
    let creates = plan
        .iter()
        .filter(|a| matches!(a, PushAction::Create { .. }))
        .count();
    let unchanged = plan
        .iter()
        .filter(|a| matches!(a, PushAction::Unchanged { .. }))
        .count();

    println!(
        "{}Category push plan for {} (category {}): {} update{}, {} create{}, {} unchanged",
        prefix,
        discourse,
        category_id,
        updates,
        if updates == 1 { "" } else { "s" },
        creates,
        if creates == 1 { "" } else { "s" },
        unchanged,
    );
    for action in plan {
        match action {
            PushAction::Update {
                path,
                topic_id,
                body,
                ..
            } => {
                println!(
                    "  ~ {} → topic {} ({} bytes)",
                    file_label(path),
                    topic_id,
                    body.len()
                );
            }
            PushAction::Unchanged { path, topic_id } => {
                println!("  = {} (topic {}, unchanged)", file_label(path), topic_id);
            }
            PushAction::Create { path, title, body } => {
                println!(
                    "  + {} → new topic \"{}\" ({} bytes)",
                    file_label(path),
                    title,
                    body.len()
                );
            }
        }
    }
}

/// File name for plan output, falling back to the full path if there is none.
fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn resolve_category_id(client: &DiscourseClient, category: &str) -> Result<u64> {
    if let Ok(id) = category.parse::<u64>() {
        return Ok(id);
    }
    let slug = category.trim();
    if slug.is_empty() {
        return Err(anyhow!(
            "missing category identifier for category operation"
        ));
    }
    let categories = client.fetch_categories()?;
    let category = categories
        .into_iter()
        .find(|cat| cat.slug == slug)
        .ok_or_else(|| not_found("category", slug))?;
    category.id.ok_or_else(|| not_found("category", slug))
}

/// Resolve a category identifier and return both the ID and the full
/// `CategoryInfo` object when the catalogue was fetched. When the input
/// is a numeric ID, the catalogue is not fetched and `None` is returned
/// for the object (the caller must fetch it separately if needed).
fn resolve_category_with_object(
    client: &DiscourseClient,
    category: &str,
) -> Result<(u64, Option<CategoryInfo>)> {
    if let Ok(id) = category.parse::<u64>() {
        return Ok((id, None));
    }
    let slug = category.trim();
    if slug.is_empty() {
        return Err(anyhow!(
            "missing category identifier for category operation"
        ));
    }
    let categories = client.fetch_categories()?;
    let found = categories
        .into_iter()
        .find(|cat| cat.slug == slug)
        .ok_or_else(|| not_found("category", slug))?;
    let id = found.id.ok_or_else(|| not_found("category", slug))?;
    Ok((id, Some(found)))
}

fn flatten_categories(category: &CategoryInfo, out: &mut Vec<CategoryInfo>) {
    out.push(category.clone());
    for sub in &category.subcategory_list {
        flatten_categories(sub, out);
    }
}

fn unique_categories(flat: Vec<CategoryInfo>) -> Vec<CategoryInfo> {
    let mut seen = std::collections::HashSet::new();
    let mut unique = Vec::new();
    for category in flat {
        if let Some(id) = category.id
            && !seen.insert(id)
        {
            continue;
        }
        unique.push(category);
    }
    unique
}

fn print_category_tree(categories: &[CategoryInfo]) {
    let mut ordered_ids = Vec::new();
    let mut map = std::collections::HashMap::new();
    for category in categories {
        if let Some(id) = category.id {
            map.entry(id).or_insert_with(|| {
                ordered_ids.push(id);
                category.clone()
            });
        }
    }

    let mut children: std::collections::HashMap<u64, Vec<u64>> = std::collections::HashMap::new();
    for category in map.values() {
        if let (Some(id), Some(parent_id)) = (category.id, category.parent_category_id)
            && map.contains_key(&parent_id)
        {
            let entry = children.entry(parent_id).or_default();
            if !entry.contains(&id) {
                entry.push(id);
            }
        }
    }

    let mut roots = Vec::new();
    for id in &ordered_ids {
        if let Some(category) = map.get(id) {
            match category.parent_category_id {
                Some(parent_id) if map.contains_key(&parent_id) => {}
                _ => roots.push(*id),
            }
        }
    }

    let mut seen = std::collections::HashSet::new();
    let last_index = roots.len().saturating_sub(1);
    for (idx, id) in roots.into_iter().enumerate() {
        let is_last = idx == last_index;
        print_category_node(&map, &children, id, "", is_last, &mut seen);
    }
}

fn print_category_node(
    map: &std::collections::HashMap<u64, CategoryInfo>,
    children: &std::collections::HashMap<u64, Vec<u64>>,
    id: u64,
    prefix: &str,
    is_last: bool,
    seen: &mut std::collections::HashSet<u64>,
) {
    if !seen.insert(id) {
        return;
    }
    if let Some(category) = map.get(&id) {
        let branch = if is_last {
            "└── ".to_string()
        } else {
            "├── ".to_string()
        };
        println!("{}{}{} - {}", prefix, branch, id, category.name);
        if let Some(child_ids) = children.get(&id) {
            let new_prefix = if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}│   ", prefix)
            };
            let last_index = child_ids.len().saturating_sub(1);
            for (idx, child_id) in child_ids.iter().enumerate() {
                let child_last = idx == last_index;
                print_category_node(map, children, *child_id, &new_prefix, child_last, seen);
            }
        }
    }
}

fn extract_title(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(title) = line.strip_prefix("# ") {
            return Some(title.trim().to_string());
        }
        break;
    }
    None
}

/// Resolve which remote topic a local file targets. A `topic_id` from front
/// matter wins outright (the durable binding written by `category pull`);
/// otherwise fall back to slug/title matching for pre-front-matter snapshots.
/// `None` means "no remote match" — which `category push` turns into a create
/// (or, under `--updates-only`, an error).
fn route_topic_id(
    front: &std::collections::HashMap<String, String>,
    title: &str,
    path: &Path,
    topics: &[TopicSummary],
) -> Option<u64> {
    front
        .get("topic_id")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .or_else(|| find_topic_match(topics, title, path).map(|t| t.id))
}

fn find_topic_match<'a>(
    topics: &'a [TopicSummary],
    title: &str,
    path: &Path,
) -> Option<&'a TopicSummary> {
    let slug = slugify(title);
    topics.iter().find(|topic| {
        topic.slug == slug
            || topic.title.eq_ignore_ascii_case(title)
            || path
                .file_stem()
                .map(|s| s.to_string_lossy().eq_ignore_ascii_case(&topic.slug))
                .unwrap_or(false)
    })
}

#[derive(Clone)]
struct TopicLink {
    id: u64,
    slug: String,
}

/// Resolve each local filename stem to its current category topic. Front
/// matter supplies the durable binding; the fresh category listing supplies
/// the canonical slug for the forum URL.
fn local_topic_links(
    paths: &[PathBuf],
    topics: &[TopicSummary],
) -> Result<HashMap<String, TopicLink>> {
    let topic_by_id: std::collections::HashMap<u64, &TopicSummary> =
        topics.iter().map(|t| (t.id, t)).collect();
    let mut links = HashMap::new();
    for path in paths {
        let raw = read_markdown(path)?;
        let (front, body) = strip_frontmatter(&raw);
        let title = front
            .get("title")
            .cloned()
            .or_else(|| extract_title(&body))
            .unwrap_or_else(|| path.file_stem().unwrap().to_string_lossy().to_string());
        let Some(id) = route_topic_id(&front, &title, path, topics) else {
            continue;
        };
        let Some(topic) = topic_by_id.get(&id) else {
            continue;
        };
        let Some(stem) = path.file_stem() else {
            continue;
        };
        links.insert(
            link_key(&stem.to_string_lossy()),
            TopicLink {
                id,
                slug: topic.slug.clone(),
            },
        );
    }
    Ok(links)
}

/// Rewrite relative `.md` links in a local document to their matching forum
/// topic URLs. The category export is flat, so only a target's final filename
/// stem is used when resolving paths such as `../versioning.md`.
fn rewrite_relative_links(
    raw: &str,
    baseurl: &str,
    links: &HashMap<String, TopicLink>,
) -> (String, Vec<String>) {
    rewrite_markdown_links(raw, |destination| {
        let Some((stem, suffix)) = relative_markdown_stem(destination) else {
            return Ok(None);
        };
        links
            .get(&link_key(stem))
            .map(|topic| {
                Some(format!(
                    "{}/t/{}/{}{}",
                    normalize_baseurl(baseurl),
                    topic.slug,
                    topic.id,
                    suffix
                ))
            })
            .ok_or_else(|| destination.to_string())
    })
}

/// Rewrite same-forum topic links to the flat local Markdown paths created by
/// `category pull`. Links outside the pulled category remain full URLs.
fn rewrite_forum_links(raw: &str, baseurl: &str, local_paths: &HashMap<u64, String>) -> String {
    let (rewritten, _) = rewrite_markdown_links(raw, |destination| {
        let Some((topic_id, suffix)) = forum_topic_id(destination, baseurl) else {
            return Ok(None);
        };
        Ok(local_paths
            .get(&topic_id)
            .map(|path| format!("{path}{suffix}")))
    });
    rewritten
}

/// Rewrite Markdown inline-link destinations outside fenced code blocks. This
/// deliberately supports the simple `[text](destination)` form used by the
/// category workflow and leaves other Markdown byte-for-byte untouched.
fn rewrite_markdown_links(
    raw: &str,
    mut replacement: impl FnMut(&str) -> std::result::Result<Option<String>, String>,
) -> (String, Vec<String>) {
    let mut out = String::with_capacity(raw.len());
    let mut unresolved = Vec::new();
    let mut fence = None;

    for line in raw.split_inclusive('\n') {
        update_fence(&mut fence, line);
        if fence.is_some() {
            out.push_str(line);
            continue;
        }
        out.push_str(&rewrite_link_line(line, &mut replacement, &mut unresolved));
    }
    (out, unresolved)
}

fn rewrite_link_line(
    line: &str,
    replacement: &mut impl FnMut(&str) -> std::result::Result<Option<String>, String>,
    unresolved: &mut Vec<String>,
) -> String {
    let mut out = String::with_capacity(line.len());
    let mut remaining = line;
    while let Some(open) = remaining.find('[') {
        // Images have the same destination syntax but are not content links.
        if open > 0 && remaining.as_bytes()[open - 1] == b'!' {
            out.push_str(&remaining[..open + 1]);
            remaining = &remaining[open + 1..];
            continue;
        }
        let Some(close_label) = remaining[open..].find("](") else {
            break;
        };
        let destination_start = open + close_label + 2;
        let Some(close_destination) = remaining[destination_start..].find(')') else {
            break;
        };
        let destination_end = destination_start + close_destination;
        let destination = &remaining[destination_start..destination_end];

        out.push_str(&remaining[..destination_start]);
        match replacement(destination) {
            Ok(Some(value)) => out.push_str(&value),
            Ok(None) => out.push_str(destination),
            Err(value) => {
                unresolved.push(value);
                out.push_str(destination);
            }
        }
        remaining = &remaining[destination_end..];
    }
    out.push_str(remaining);
    out
}

fn relative_markdown_stem(destination: &str) -> Option<(&str, &str)> {
    let (path, suffix) = split_link_suffix(destination);
    if path.starts_with('/') || path.contains("://") {
        return None;
    }
    let stem = path.rsplit('/').next()?.strip_suffix(".md")?;
    (!stem.is_empty()).then_some((stem, suffix))
}

fn forum_topic_id<'a>(destination: &'a str, baseurl: &str) -> Option<(u64, &'a str)> {
    let (path, suffix) = split_link_suffix(destination);
    let path = path
        .strip_prefix(&normalize_baseurl(baseurl))?
        .strip_prefix("/t/")?;
    let id = path.rsplit('/').next()?.parse().ok()?;
    Some((id, suffix))
}

fn split_link_suffix(destination: &str) -> (&str, &str) {
    let index = destination.find(['?', '#']).unwrap_or(destination.len());
    destination.split_at(index)
}

fn link_key(stem: &str) -> String {
    stem.to_ascii_lowercase()
}

/// MkDocs/Zensical's three admonition markers, including its two foldable
/// forms. The mode is retained while parsing so Quote Callouts can preserve
/// collapsible behaviour; ordinary blockquotes intentionally cannot.
#[derive(Clone, Copy)]
enum MkdocsFold {
    None,
    Collapsed,
    Expanded,
}

struct MkdocsAdmonition {
    indent: usize,
    fold: MkdocsFold,
    kind: String,
    title: Option<String>,
}

/// Convert MkDocs/Zensical admonitions in a local Markdown body to the chosen
/// Discourse representation. This is deliberately a small line-oriented
/// parser rather than a general Markdown rewriter: it only recognises
/// admonition openers outside fenced code blocks and leaves all other content
/// byte-for-byte intact.
fn convert_mkdocs_admonitions(raw: &str, style: AdmonitionStyle) -> String {
    let lines = raw.split('\n').map(str::to_owned).collect::<Vec<_>>();
    convert_mkdocs_lines(&lines, style).join("\n")
}

fn convert_mkdocs_lines(lines: &[String], style: AdmonitionStyle) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut index = 0;
    let mut fence = None;

    while index < lines.len() {
        let line = &lines[index];
        update_fence(&mut fence, line);
        if fence.is_none()
            && let Some(admonition) = parse_mkdocs_admonition(line)
        {
            let (body_end, next_index) = mkdocs_block_bounds(lines, index, admonition.indent);
            let body = lines[index + 1..body_end]
                .iter()
                .map(|body_line| strip_leading_spaces(body_line, admonition.indent + 4))
                .collect::<Vec<_>>();
            let converted_body = convert_mkdocs_lines(&body, style);

            out.push(render_discourse_admonition_header(&admonition, style));
            for body_line in converted_body {
                if body_line.is_empty() {
                    out.push(">".to_string());
                } else {
                    out.push(format!("> {body_line}"));
                }
            }
            // Preserve blank lines separating this block from the following
            // paragraph, but do not make them part of the blockquote.
            out.extend(lines[body_end..next_index].iter().cloned());
            index = next_index;
            continue;
        }
        out.push(line.clone());
        index += 1;
    }
    out
}

fn parse_mkdocs_admonition(line: &str) -> Option<MkdocsAdmonition> {
    let indent = leading_spaces(line);
    let rest = &line[indent..];
    let (fold, after_marker) = if let Some(after) = rest.strip_prefix("???+") {
        (MkdocsFold::Expanded, after)
    } else if let Some(after) = rest.strip_prefix("???") {
        (MkdocsFold::Collapsed, after)
    } else {
        let after = rest.strip_prefix("!!!")?;
        (MkdocsFold::None, after)
    };

    if after_marker
        .chars()
        .next()
        .is_some_and(|character| !character.is_whitespace())
    {
        return None;
    }
    let after_marker = after_marker.trim_start();
    let kind_end = after_marker
        .find(char::is_whitespace)
        .unwrap_or(after_marker.len());
    let kind = &after_marker[..kind_end];
    if kind.is_empty()
        || !kind
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return None;
    }
    let title = parse_admonition_title(after_marker[kind_end..].trim());
    Some(MkdocsAdmonition {
        indent,
        fold,
        kind: kind.to_string(),
        title,
    })
}

fn parse_admonition_title(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let title = raw
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .map(|value| value.replace("\\\"", "\"").replace("\\\\", "\\"))
        .unwrap_or_else(|| raw.to_string());
    Some(title)
}

fn mkdocs_block_bounds(lines: &[String], start: usize, indent: usize) -> (usize, usize) {
    let mut next_index = start + 1;
    while next_index < lines.len()
        && (lines[next_index].trim().is_empty() || leading_spaces(&lines[next_index]) > indent)
    {
        next_index += 1;
    }
    let mut body_end = next_index;
    while body_end > start + 1 && lines[body_end - 1].trim().is_empty() {
        body_end -= 1;
    }
    (body_end, next_index)
}

fn render_discourse_admonition_header(
    admonition: &MkdocsAdmonition,
    style: AdmonitionStyle,
) -> String {
    match style {
        AdmonitionStyle::QuoteCallouts => {
            let fold = match admonition.fold {
                MkdocsFold::None => "",
                MkdocsFold::Collapsed => "-",
                MkdocsFold::Expanded => "+",
            };
            let title = admonition
                .title
                .as_ref()
                .map(|title| format!(" {title}"))
                .unwrap_or_default();
            format!("> [!{}]{}{}", admonition.kind, fold, title)
        }
        AdmonitionStyle::PlainBlockquote => {
            let (emoji, label) = plain_callout_heading(&admonition.kind);
            let title = admonition
                .title
                .as_ref()
                .map(|title| format!(" — {title}"))
                .unwrap_or_default();
            format!("> **{emoji} {label}{title}**")
        }
    }
}

/// Convert a selected `dsc`-generated Discourse callout representation back to
/// MkDocs/Zensical syntax. Ordinary blockquotes, including styles from another
/// mode, are intentionally left unchanged.
fn convert_discourse_admonitions(raw: &str, style: AdmonitionStyle) -> String {
    let lines = raw.split('\n').map(str::to_owned).collect::<Vec<_>>();
    convert_discourse_lines(&lines, style).join("\n")
}

fn convert_discourse_lines(lines: &[String], style: AdmonitionStyle) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut index = 0;
    let mut fence = None;

    while index < lines.len() {
        let line = &lines[index];
        update_fence(&mut fence, line);
        if fence.is_none()
            && let Some((fold, kind, title)) = parse_discourse_admonition(line, style)
        {
            let (body_end, next_index) = quote_block_bounds(lines, index);
            let body = lines[index + 1..body_end]
                .iter()
                .filter_map(|body_line| strip_quote_prefix(body_line).map(str::to_owned))
                .collect::<Vec<_>>();
            let converted_body = convert_discourse_lines(&body, style);

            out.push(render_mkdocs_admonition_header(
                fold,
                &kind,
                title.as_deref(),
            ));
            for body_line in converted_body {
                out.push(format!("    {body_line}"));
            }
            // A quoted blank line terminates the source blockquote. Preserve
            // its separation without retaining a stray quote marker locally.
            out.extend((body_end..next_index).map(|_| String::new()));
            index = next_index;
            continue;
        }
        out.push(line.clone());
        index += 1;
    }
    out
}

fn parse_discourse_admonition(
    line: &str,
    style: AdmonitionStyle,
) -> Option<(MkdocsFold, String, Option<String>)> {
    let content = strip_quote_prefix(line)?;
    match style {
        AdmonitionStyle::QuoteCallouts => parse_quote_callout_header(content),
        AdmonitionStyle::PlainBlockquote => parse_plain_blockquote_header(content),
    }
}

fn parse_quote_callout_header(content: &str) -> Option<(MkdocsFold, String, Option<String>)> {
    let rest = content.strip_prefix("[!")?;
    let close = rest.find(']')?;
    let kind = &rest[..close];
    if kind.is_empty()
        || !kind
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return None;
    }
    let after = &rest[close + 1..];
    let (fold, title) = if let Some(title) = after.strip_prefix('-') {
        if starts_with_non_whitespace(title) {
            return None;
        }
        (MkdocsFold::Collapsed, raw_callout_title(title))
    } else if let Some(title) = after.strip_prefix('+') {
        if starts_with_non_whitespace(title) {
            return None;
        }
        (MkdocsFold::Expanded, raw_callout_title(title))
    } else {
        if starts_with_non_whitespace(after) {
            return None;
        }
        (MkdocsFold::None, raw_callout_title(after))
    };
    Some((fold, kind.to_string(), title))
}

fn starts_with_non_whitespace(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|character| !character.is_whitespace())
}

fn raw_callout_title(raw: &str) -> Option<String> {
    let raw = raw.trim();
    (!raw.is_empty()).then(|| raw.to_string())
}

fn parse_plain_blockquote_header(content: &str) -> Option<(MkdocsFold, String, Option<String>)> {
    let bold = content.strip_prefix("**")?.strip_suffix("**")?;
    let (label, title) = bold
        .split_once(" — ")
        .map(|(label, title)| (label, Some(title.to_string())))
        .unwrap_or((bold, None));
    let kind = plain_callout_kind(label)?;
    Some((MkdocsFold::None, kind, title))
}

fn quote_block_bounds(lines: &[String], start: usize) -> (usize, usize) {
    let mut next_index = start + 1;
    while next_index < lines.len() && strip_quote_prefix(&lines[next_index]).is_some() {
        next_index += 1;
    }
    let mut body_end = next_index;
    while body_end > start + 1
        && strip_quote_prefix(&lines[body_end - 1]).is_some_and(|line| line.trim().is_empty())
    {
        body_end -= 1;
    }
    (body_end, next_index)
}

fn render_mkdocs_admonition_header(fold: MkdocsFold, kind: &str, title: Option<&str>) -> String {
    let marker = match fold {
        MkdocsFold::None => "!!!",
        MkdocsFold::Collapsed => "???",
        MkdocsFold::Expanded => "???+",
    };
    let title = title
        .map(|title| {
            let escaped = title.replace('\\', "\\\\").replace('"', "\\\"");
            format!(" \"{escaped}\"")
        })
        .unwrap_or_default();
    format!("{marker} {kind}{title}")
}

fn plain_callout_heading(kind: &str) -> (&'static str, String) {
    let kind = kind.to_ascii_lowercase();
    match kind.as_str() {
        "note" | "info" => ("📝", "Note".to_string()),
        "abstract" | "summary" | "tldr" => ("📄", "Abstract".to_string()),
        "todo" => ("☑️", "Todo".to_string()),
        "tip" | "hint" | "important" => ("💡", "Tip".to_string()),
        "success" | "check" | "done" => ("✅", "Success".to_string()),
        "question" | "help" | "faq" => ("❓", "Question".to_string()),
        "warning" | "caution" | "attention" => ("⚠️", "Warning".to_string()),
        "failure" | "fail" | "missing" => ("❌", "Failure".to_string()),
        "danger" | "error" => ("🚨", "Danger".to_string()),
        "bug" => ("🐛", "Bug".to_string()),
        "example" => ("🧪", "Example".to_string()),
        "quote" | "cite" => ("💬", "Quote".to_string()),
        _ => ("📌", display_callout_type(&kind)),
    }
}

fn plain_callout_kind(label: &str) -> Option<String> {
    let kind = match label {
        "📝 Note" => "note",
        "📄 Abstract" => "abstract",
        "☑️ Todo" => "todo",
        "💡 Tip" => "tip",
        "✅ Success" => "success",
        "❓ Question" => "question",
        "⚠️ Warning" => "warning",
        "❌ Failure" => "failure",
        "🚨 Danger" => "danger",
        "🐛 Bug" => "bug",
        "🧪 Example" => "example",
        "💬 Quote" => "quote",
        _ => {
            let custom = label.strip_prefix("📌 ")?;
            let kind = custom
                .split_whitespace()
                .map(|word| word.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("-");
            if kind.is_empty() {
                return None;
            }
            return Some(kind);
        }
    };
    Some(kind.to_string())
}

fn display_callout_type(kind: &str) -> String {
    kind.split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_quote_prefix(line: &str) -> Option<&str> {
    let indent = leading_spaces(line);
    let rest = line[indent..].strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn leading_spaces(line: &str) -> usize {
    line.as_bytes()
        .iter()
        .take_while(|&&byte| byte == b' ')
        .count()
}

fn strip_leading_spaces(line: &str, count: usize) -> String {
    let removable = leading_spaces(line).min(count);
    line[removable..].to_string()
}

fn update_fence(fence: &mut Option<(char, usize)>, line: &str) {
    let trimmed = line.trim_start();
    let Some(marker) = trimmed.chars().next() else {
        return;
    };
    let length = trimmed.chars().take_while(|&c| c == marker).count();
    if !matches!(marker, '`' | '~') || length < 3 {
        return;
    }
    match fence {
        Some((current, opening_length)) if *current == marker && length >= *opening_length => {
            *fence = None;
        }
        None => *fence = Some((marker, length)),
        Some(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn summary(id: u64, title: &str, slug: &str) -> TopicSummary {
        TopicSummary {
            id,
            title: title.to_string(),
            slug: slug.to_string(),
        }
    }

    fn front(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn category_binding_rejects_cross_category_topic() {
        let err = validate_topic_category(412, Some(9), 7, false).unwrap_err();
        assert!(err.to_string().contains("belongs to category 9"));
    }

    #[test]
    fn category_binding_allows_matching_or_listed_topic() {
        validate_topic_category(412, Some(7), 7, false).unwrap();
        validate_topic_category(412, None, 7, true).unwrap();
        assert!(validate_topic_category(412, None, 7, false).is_err());
    }

    #[test]
    fn category_pull_disambiguates_colliding_slugs_with_topic_ids() {
        let topics = vec![summary(41, "Cafe", "cafe"), summary(42, "Café", "cafe-2")];
        let paths = category_local_paths(&topics);
        assert_eq!(paths.get(&41).unwrap(), "cafe-41.md");
        assert_eq!(paths.get(&42).unwrap(), "cafe-42.md");
    }

    #[test]
    fn render_category_topic_emits_front_matter() {
        let topic = summary(412, "Dependency management", "dependency-management");
        let out = render_category_topic(&topic, "https://forum.rcpch.tech/", "Body here.\n");
        assert!(out.starts_with("---\n"));
        assert!(out.contains("title: Dependency management\n"));
        assert!(out.contains("topic_id: 412\n"));
        assert!(out.contains("url: https://forum.rcpch.tech/t/dependency-management/412\n"));
        assert!(out.contains("pulled_at: "));
        assert!(out.contains("\n---\n\nBody here.\n"));
    }

    #[test]
    fn render_then_strip_round_trips_the_body() {
        let topic = summary(7, "Intro: getting started", "intro-getting-started");
        let body = "First paragraph.\n\n---\n\nSecond, after a rule.\n";
        let rendered = render_category_topic(&topic, "https://x.test", body);
        let (front, recovered) = strip_frontmatter(&rendered);
        assert_eq!(front.get("topic_id").map(String::as_str), Some("7"));
        // Title carrying a colon is round-tripped through YAML quoting.
        assert_eq!(
            front.get("title").map(String::as_str),
            Some("Intro: getting started")
        );
        assert_eq!(recovered, body);
    }

    #[test]
    fn route_prefers_front_matter_topic_id_over_slug() {
        // The slug would match topic 1, but the front-matter id wins — this is
        // the whole point of Gap 1: a renamed/retitled file still routes home.
        let topics = vec![summary(1, "Old Title", "renamed-file")];
        let f = front(&[("topic_id", "412")]);
        let path = PathBuf::from("renamed-file.md");
        assert_eq!(route_topic_id(&f, "New Title", &path, &topics), Some(412));
    }

    #[test]
    fn route_falls_back_to_slug_match_without_front_matter() {
        let topics = vec![summary(
            55,
            "Dependency management",
            "dependency-management",
        )];
        let f = HashMap::new();
        let path = PathBuf::from("dependency-management.md");
        assert_eq!(
            route_topic_id(&f, "Dependency management", &path, &topics),
            Some(55)
        );
    }

    #[test]
    fn route_returns_none_when_nothing_matches() {
        let topics = vec![summary(
            55,
            "Dependency management",
            "dependency-management",
        )];
        let f = HashMap::new();
        let path = PathBuf::from("brand-new-file.md");
        assert_eq!(route_topic_id(&f, "Brand new file", &path, &topics), None);
    }

    #[test]
    fn route_ignores_unparseable_front_matter_topic_id() {
        // A garbage topic_id must not route; fall through to slug matching.
        let topics = vec![summary(
            55,
            "Dependency management",
            "dependency-management",
        )];
        let f = front(&[("topic_id", "not-a-number")]);
        let path = PathBuf::from("dependency-management.md");
        assert_eq!(
            route_topic_id(&f, "Dependency management", &path, &topics),
            Some(55)
        );
    }

    #[test]
    fn relative_links_rewrite_to_category_topic_urls_and_warn_when_unresolved() {
        let mut links = HashMap::new();
        links.insert(
            link_key("versioning"),
            TopicLink {
                id: 42,
                slug: "versioning".to_string(),
            },
        );
        let source = concat!(
            "[Known](../Versioning.md#details)\n",
            "[Missing](missing.md)\n",
            "[External](https://example.com/guide.md)\n",
            "![Image](diagram.md)\n",
            "```markdown\n",
            "[Literal](versioning.md)\n",
            "```\n"
        );

        let (rewritten, unresolved) =
            rewrite_relative_links(source, "https://forum.example.com/", &links);

        assert_eq!(
            rewritten,
            concat!(
                "[Known](https://forum.example.com/t/versioning/42#details)\n",
                "[Missing](missing.md)\n",
                "[External](https://example.com/guide.md)\n",
                "![Image](diagram.md)\n",
                "```markdown\n",
                "[Literal](versioning.md)\n",
                "```\n"
            )
        );
        assert_eq!(unresolved, ["missing.md"]);
    }

    #[test]
    fn forum_links_rewrite_to_flat_local_markdown_paths() {
        let local_paths = HashMap::from([(42, "versioning.md".to_string())]);
        let source = concat!(
            "[Known](https://forum.example.com/t/versioning/42#details)\n",
            "[Other topic](https://forum.example.com/t/elsewhere/99)\n",
            "[External](https://other.example/t/versioning/42)\n",
            "```markdown\n",
            "[Literal](https://forum.example.com/t/versioning/42)\n",
            "```\n"
        );

        assert_eq!(
            rewrite_forum_links(source, "https://forum.example.com/", &local_paths),
            concat!(
                "[Known](versioning.md#details)\n",
                "[Other topic](https://forum.example.com/t/elsewhere/99)\n",
                "[External](https://other.example/t/versioning/42)\n",
                "```markdown\n",
                "[Literal](https://forum.example.com/t/versioning/42)\n",
                "```\n"
            )
        );
    }

    #[test]
    fn quote_callouts_round_trip_nested_and_foldable_admonitions() {
        let source = concat!(
            "Before.\n\n",
            "!!! note \"Outer\"\n",
            "    First line.\n",
            "    \n",
            "    ??? warning \"Inner\"\n",
            "        Check this.\n\n",
            "???+ tip \"Open\"\n",
            "    Second.\n"
        );
        let expected_discourse = concat!(
            "Before.\n\n",
            "> [!note] Outer\n",
            "> First line.\n",
            ">\n",
            "> > [!warning]- Inner\n",
            "> > Check this.\n\n",
            "> [!tip]+ Open\n",
            "> Second.\n"
        );

        let discourse = convert_mkdocs_admonitions(source, AdmonitionStyle::QuoteCallouts);
        assert_eq!(discourse, expected_discourse);
        assert_eq!(
            convert_discourse_admonitions(&discourse, AdmonitionStyle::QuoteCallouts),
            source
        );
    }

    #[test]
    fn plain_blockquotes_round_trip_to_canonical_mkdocs_types() {
        let source = concat!(
            "!!! info \"Heads up\"\n",
            "    Read this.\n\n",
            "??? warning \"Review\"\n",
            "    Check this.\n"
        );
        let expected_discourse = concat!(
            "> **📝 Note — Heads up**\n",
            "> Read this.\n\n",
            "> **⚠️ Warning — Review**\n",
            "> Check this.\n"
        );
        let expected_mkdocs = concat!(
            "!!! note \"Heads up\"\n",
            "    Read this.\n\n",
            "!!! warning \"Review\"\n",
            "    Check this.\n"
        );

        let discourse = convert_mkdocs_admonitions(source, AdmonitionStyle::PlainBlockquote);
        assert_eq!(discourse, expected_discourse);
        assert_eq!(
            convert_discourse_admonitions(&discourse, AdmonitionStyle::PlainBlockquote),
            expected_mkdocs
        );
    }

    #[test]
    fn conversion_leaves_fenced_code_and_unrecognised_quotes_unchanged() {
        let mkdocs = concat!(
            "```markdown\n",
            "!!! note \"An example, not a callout\"\n",
            "    This must remain literal.\n",
            "```\n"
        );
        assert_eq!(
            convert_mkdocs_admonitions(mkdocs, AdmonitionStyle::QuoteCallouts),
            mkdocs
        );

        let discourse = "> An ordinary quote\n> remains an ordinary quote.\n";
        assert_eq!(
            convert_discourse_admonitions(discourse, AdmonitionStyle::QuoteCallouts),
            discourse
        );
    }
}

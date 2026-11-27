// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{
    Board, BoardAssignee, BoardCard, BoardDetail, DiscourseClient, board_create_payload,
};
use crate::cli::ListFormat;
use crate::commands::common::{emit_with, ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::{atomic_write, ensure_output_available, is_json_path, slugify};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A bundled, create-only board layout. Templates are deliberately code-owned
/// in this first phase: no local file is interpreted as a board mutation.
#[derive(Debug, Clone, Serialize)]
pub struct BoardTemplate {
    pub name: &'static str,
    pub default_name: &'static str,
    pub columns: &'static [&'static str],
}

const BOARD_TEMPLATES: &[BoardTemplate] = &[BoardTemplate {
    name: "to-do",
    default_name: "To-do",
    columns: &["Ideas", "Backlog", "In progress", "Completed", "Reference"],
}];

#[derive(Debug, Serialize)]
struct BoardScaffoldResult {
    discourse: String,
    template: String,
    name: String,
    slug: String,
    columns: Vec<String>,
    /// `planned`, `created`, or `unchanged`.
    action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    board_id: Option<i64>,
    /// The exact JSON body for the sole mutating request, when creating.
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<serde_json::Value>,
}

/// List bundled board templates without loading configuration or contacting a
/// forum.
pub fn board_templates(format: ListFormat) -> Result<()> {
    emit_with(format, &BOARD_TEMPLATES, || {
        for template in BOARD_TEMPLATES {
            println!(
                "{} (default board: {})",
                template.name, template.default_name
            );
            for column in template.columns {
                println!("  - {column}");
            }
        }
        Ok(())
    })
}

/// Create one empty board from a bundled template. Existing boards are never
/// adopted or updated: a matching layout is a no-op and every collision is an
/// error requiring operator review.
pub fn board_scaffold(
    config: &Config,
    discourse_name: &str,
    template_name: &str,
    supplied_name: Option<&str>,
    format: ListFormat,
    dry_run: bool,
) -> Result<()> {
    let template = find_template(template_name)?;
    let name = supplied_name.unwrap_or(template.default_name).trim();
    if name.is_empty() {
        bail!("board name must not be empty");
    }
    let slug = slugify(name);
    let columns = template
        .columns
        .iter()
        .map(|title| (*title).to_string())
        .collect::<Vec<_>>();

    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let boards = client.list_boards()?;
    let matching = boards
        .iter()
        .filter(|board| board.slug.as_deref() == Some(slug.as_str()))
        .collect::<Vec<_>>();
    if matching.len() > 1 {
        bail!(
            "multiple accessible boards use slug '{slug}'; review them manually before scaffolding"
        );
    }
    if let Some(board) = matching.first() {
        if board.name == name && board_column_titles(board) == columns {
            return emit_scaffold_result(
                format,
                BoardScaffoldResult {
                    discourse: discourse.name.clone(),
                    template: template.name.to_string(),
                    name: name.to_string(),
                    slug,
                    columns,
                    action: "unchanged".to_string(),
                    board_id: Some(board.id),
                    request: None,
                },
                dry_run,
            );
        }
        bail!(
            "board slug '{slug}' already belongs to '{}' with a different layout; choose --name or review it manually",
            board.name
        );
    }

    let planned = BoardScaffoldResult {
        discourse: discourse.name.clone(),
        template: template.name.to_string(),
        name: name.to_string(),
        slug: slug.clone(),
        columns: columns.clone(),
        action: "planned".to_string(),
        board_id: None,
        request: Some(board_create_payload(name, &slug, &columns)),
    };
    if dry_run {
        return emit_scaffold_result(format, planned, true);
    }

    let created = client.create_board(name, &slug, &columns)?;
    verify_created_board(&created, name, &slug, &columns, "create response")?;
    let verified = client.list_boards()?;
    let matching = verified
        .iter()
        .filter(|board| board.slug.as_deref() == Some(slug.as_str()))
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        bail!(
            "board scaffold verification failed: expected exactly one board with slug '{slug}' in the board index, found {}",
            matching.len()
        );
    }
    verify_created_board(matching[0], name, &slug, &columns, "board index")?;
    emit_scaffold_result(
        format,
        BoardScaffoldResult {
            action: "created".to_string(),
            board_id: Some(created.id),
            ..planned
        },
        false,
    )
}

fn find_template(name: &str) -> Result<&'static BoardTemplate> {
    BOARD_TEMPLATES
        .iter()
        .find(|template| template.name == name)
        .ok_or_else(|| {
            anyhow!(
                "unknown board template '{name}'; available templates: {}",
                BOARD_TEMPLATES
                    .iter()
                    .map(|template| template.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn board_column_titles(board: &Board) -> Vec<String> {
    board
        .columns
        .iter()
        .map(|column| column.title.clone())
        .collect()
}

fn verify_created_board(
    board: &Board,
    name: &str,
    slug: &str,
    columns: &[String],
    source: &str,
) -> Result<()> {
    if board.name != name
        || board.slug.as_deref() != Some(slug)
        || board_column_titles(board) != columns
    {
        bail!(
            "board scaffold verification failed in {source}: expected '{}' ({slug}) with columns [{}]",
            name,
            columns.join(", ")
        );
    }
    Ok(())
}

fn emit_scaffold_result(
    format: ListFormat,
    result: BoardScaffoldResult,
    dry_run: bool,
) -> Result<()> {
    emit_with(format, &result, || {
        if dry_run && result.action == "planned" {
            println!(
                "[dry-run] scaffold board '{}' on {}",
                result.name, result.discourse
            );
            println!("template: {}", result.template);
            println!("slug: {}", result.slug);
            println!("access: restricted (creator and mandatory administrators only)");
            println!("columns: {}", result.columns.join(" -> "));
            let request = serde_json::to_string(
                result
                    .request
                    .as_ref()
                    .expect("planned scaffold must include its request"),
            )
            .context("serializing board scaffold plan")?;
            println!("+ POST /boards/api/boards.json {request}");
        } else if result.action == "unchanged" {
            println!(
                "Board '{}' already matches the {} template; no changes made.",
                result.name, result.template
            );
        } else {
            println!(
                "Created board '{}' from the {} template (id {}).",
                result.name,
                result.template,
                result.board_id.unwrap_or_default()
            );
        }
        Ok(())
    })
}

/// List every board accessible to the configured API user.
pub fn board_list(config: &Config, discourse_name: &str, format: ListFormat) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let mut boards = client.list_boards()?;
    boards.sort_by(|a, b| a.name.cmp(&b.name));

    emit_with(format, &boards, || {
        print_board_list(&boards);
        Ok(())
    })
}

/// Show one board's full definition: metadata, columns, and cards.
pub fn board_show(
    config: &Config,
    discourse_name: &str,
    board_id: i64,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let detail = client.show_board(board_id)?;
    emit_with(format, &detail, || {
        print_board_detail(&detail);
        Ok(())
    })
}

// ─── Pull (snapshot) ────────────────────────────────────────────────────────

/// On-disk board snapshot (schema version 1). API response models are kept out
/// of this schema so response-only and newly introduced fields cannot silently
/// become future `push` input.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct BoardSnapshot {
    pub version: u32,
    /// Informational origin of the snapshot. It is deliberately not a guard:
    /// later board sync may intentionally copy reviewed snapshots cross-site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<crate::utils::SnapshotProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_id: Option<i64>,
    pub board_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discourse_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulled_at: Option<String>,
    pub board: BoardSnapshotDefinition,
    #[serde(default)]
    pub columns: Vec<BoardSnapshotColumn>,
}

/// Mutable board fields retained in a snapshot. ACL-derived and runtime-only
/// fields are intentionally excluded until ACL management has a defined schema.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct BoardSnapshotDefinition {
    pub name: String,
    pub slug: Option<String>,
    #[serde(default)]
    pub category_ids: Vec<i64>,
    #[serde(default)]
    pub tag_names: Vec<String>,
    pub require_confirmation: bool,
    pub show_tags: bool,
    pub card_style: Option<String>,
    pub show_topic_thumbnail: bool,
}

/// One column in snapshot list order. The server's opaque `position` is not
/// persisted; a future push must express reordering through relative moves.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct BoardSnapshotColumn {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub title: String,
    pub icon: Option<String>,
    pub default_sort: Option<String>,
    pub tag_name: Option<String>,
    pub move_to_category_id: Option<i64>,
    pub move_to_assigned: Option<String>,
    pub move_to_status: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub cards: Vec<BoardSnapshotCard>,
}

/// One card in snapshot list order. Topic response metadata and timestamps are
/// deliberately excluded; topic cards are represented by `topic_id` only.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct BoardSnapshotCard {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub card_type: String,
    pub title: Option<String>,
    pub notes: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub topic_id: Option<i64>,
    pub assigned_to: Option<BoardSnapshotAssignee>,
}

/// Assignment target retained without conflating users and groups that happen
/// to have the same name.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum BoardSnapshotAssignee {
    User { username: String },
    Group { name: String },
}

/// Snapshot a board to a local YAML/JSON file.
pub fn board_pull(
    config: &Config,
    discourse_name: &str,
    board_id: i64,
    local_path: &Path,
    force: bool,
) -> Result<()> {
    // Avoid invoking Discourse's side-effectful detail GET when the local
    // operation is already known to be invalid.
    ensure_output_available(local_path, force)?;

    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let detail = client.show_board(board_id)?;
    let discourse_version = client.fetch_version().ok().flatten();
    let pulled_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let mut category_ids = detail.board.category_ids.clone();
    category_ids.sort_unstable();
    let mut tag_names = detail.board.tag_names.clone();
    tag_names.sort();

    let board = BoardSnapshotDefinition {
        name: detail.board.name.clone(),
        slug: detail.board.slug.clone(),
        category_ids,
        tag_names,
        require_confirmation: detail.board.require_confirmation,
        show_tags: detail.board.show_tags,
        card_style: detail.board.card_style.clone(),
        show_topic_thumbnail: detail.board.show_topic_thumbnail,
    };
    let columns = detail
        .columns
        .into_iter()
        .map(|column| {
            let mut cards = column.cards;
            cards.sort_by_key(|card| (card.position.unwrap_or(i64::MAX), card.id));
            BoardSnapshotColumn {
                id: Some(column.id),
                title: column.title,
                icon: column.icon,
                default_sort: column.default_sort,
                tag_name: column.tag_name,
                move_to_category_id: column.move_to_category_id,
                move_to_assigned: column.move_to_assigned,
                move_to_status: column.move_to_status,
                color: column.color,
                cards: cards
                    .into_iter()
                    .map(|card| {
                        let mut tags = card
                            .tags
                            .into_iter()
                            .map(|tag| tag.name)
                            .collect::<Vec<_>>();
                        tags.sort();
                        BoardSnapshotCard {
                            id: Some(card.id),
                            card_type: card.card_type,
                            title: card.title,
                            notes: card.notes,
                            tags,
                            topic_id: card.topic_id,
                            assigned_to: card.assigned_to.map(|assignee| match assignee {
                                BoardAssignee::User { username, .. } => {
                                    BoardSnapshotAssignee::User { username }
                                }
                                BoardAssignee::Group { name, .. } => {
                                    BoardSnapshotAssignee::Group { name }
                                }
                            }),
                        }
                    })
                    .collect(),
            }
        })
        .collect();

    let snapshot = BoardSnapshot {
        version: 1,
        provenance: Some(crate::utils::snapshot_provenance(&discourse.baseurl)),
        board_id: Some(detail.board.id),
        board_name: detail.board.name,
        discourse_version,
        pulled_at: Some(pulled_at),
        board,
        columns,
    };

    let content = if is_json_path(local_path) {
        serde_json::to_string_pretty(&snapshot).context("serializing board snapshot as JSON")?
    } else {
        serde_yaml::to_string(&snapshot).context("serializing board snapshot as YAML")?
    };

    atomic_write(local_path, &content, force)?;

    let card_count: usize = snapshot.columns.iter().map(|c| c.cards.len()).sum();
    println!(
        "Wrote board '{}' ({} column{}, {} card{}) to {}",
        snapshot.board_name,
        snapshot.columns.len(),
        if snapshot.columns.len() == 1 { "" } else { "s" },
        card_count,
        if card_count == 1 { "" } else { "s" },
        local_path.display()
    );
    Ok(())
}

// ─── Text rendering ─────────────────────────────────────────────────────────

fn print_board_list(boards: &[Board]) {
    if boards.is_empty() {
        println!("No boards found.");
        return;
    }
    for board in boards {
        let slug = board.slug.as_deref().unwrap_or("-");
        let acl = acl_summary(board);
        let constraints = constraint_summary(board);
        println!(
            "{:>5}  {}  slug:{}  {}  {}",
            board.id, board.name, slug, acl, constraints
        );
    }
}

fn print_board_detail(detail: &BoardDetail) {
    let board = &detail.board;
    println!("id:          {}", board.id);
    println!("name:        {}", board.name);
    println!("slug:        {}", board.slug.as_deref().unwrap_or("-"));
    println!(
        "card style:  {}",
        board.card_style.as_deref().unwrap_or("-")
    );
    println!("acl:         {}", acl_summary(board));
    println!("constraints: {}", constraint_summary(board));
    println!(
        "created by:  {}",
        board
            .created_by
            .as_ref()
            .and_then(|u| u.username.as_deref())
            .unwrap_or("-")
    );
    println!();
    for column in &detail.columns {
        println!(
            "column {} - {} ({} card{})",
            column.id,
            column.title,
            column.cards.len(),
            if column.cards.len() == 1 { "" } else { "s" }
        );
        for card in &column.cards {
            println!("  {}", card_summary(card));
        }
    }
}

fn acl_summary(board: &Board) -> String {
    let mut bits = Vec::new();
    if board.can_manage {
        bits.push("manage");
    } else if board.can_write {
        bits.push("write");
    } else {
        bits.push("read");
    }
    if board.anonymous_can_read {
        bits.push("anon-read");
    }
    format!("acl:{}", bits.join(","))
}

fn constraint_summary(board: &Board) -> String {
    if board.tag_names.is_empty() && board.category_ids.is_empty() {
        return "unconstrained".to_string();
    }
    let mut parts = Vec::new();
    if !board.tag_names.is_empty() {
        parts.push(format!("tags:{}", board.tag_names.join(",")));
    }
    if !board.category_ids.is_empty() {
        parts.push(format!(
            "categories:{}",
            board
                .category_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    parts.join(" ")
}

fn card_summary(card: &BoardCard) -> String {
    match card.card_type.as_str() {
        "topic" => {
            let title = card
                .topic
                .as_ref()
                .and_then(|t| t.title.as_deref())
                .unwrap_or("(topic)");
            format!(
                "[topic] {} (topic_id:{})",
                title,
                card.topic_id.unwrap_or(0)
            )
        }
        _ => {
            let title = card.title.as_deref().unwrap_or("(untitled)");
            let notes = card
                .notes
                .as_deref()
                .filter(|n| !n.is_empty())
                .map(|n| format!(" - {n}"))
                .unwrap_or_default();
            format!("[{}] {}{}", card.card_type, title, notes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn json_path_detected_by_extension_case_insensitively() {
        assert!(is_json_path(&PathBuf::from("board.json")));
        assert!(is_json_path(&PathBuf::from("board.JSON")));
        assert!(!is_json_path(&PathBuf::from("board.yaml")));
        assert!(!is_json_path(&PathBuf::from("board")));
    }

    #[test]
    fn acl_summary_prefers_manage_then_write_then_read() {
        let mut board = Board {
            id: 1,
            name: "b".to_string(),
            slug: None,
            category_ids: vec![],
            tag_ids: vec![],
            tag_names: vec![],
            anonymous_can_read: false,
            require_confirmation: false,
            show_tags: false,
            card_style: None,
            show_topic_thumbnail: false,
            can_write: false,
            can_manage: false,
            created_by: None,
            columns: vec![],
            extra: Default::default(),
        };
        assert_eq!(acl_summary(&board), "acl:read");
        board.can_write = true;
        assert_eq!(acl_summary(&board), "acl:write");
        board.can_manage = true;
        assert_eq!(acl_summary(&board), "acl:manage");
    }

    #[test]
    fn constraint_summary_reports_unconstrained_when_empty() {
        let board = Board {
            id: 1,
            name: "b".to_string(),
            slug: None,
            category_ids: vec![],
            tag_ids: vec![],
            tag_names: vec!["discourse".to_string()],
            anonymous_can_read: false,
            require_confirmation: false,
            show_tags: false,
            card_style: None,
            show_topic_thumbnail: false,
            can_write: false,
            can_manage: false,
            created_by: None,
            columns: vec![],
            extra: Default::default(),
        };
        assert_eq!(constraint_summary(&board), "tags:discourse");
    }

    #[test]
    fn snapshot_schema_allows_new_objects_and_preserves_group_assignees() {
        let snapshot: BoardSnapshot = serde_yaml::from_str(
            r#"
version: 1
board_name: New board
board:
  name: New board
  slug: null
  category_ids: []
  tag_names: []
  require_confirmation: false
  show_tags: false
  card_style: detailed
  show_topic_thumbnail: false
columns:
  - title: Backlog
    icon: null
    default_sort: priority
    tag_name: null
    move_to_category_id: null
    move_to_assigned: null
    move_to_status: null
    color: null
    cards:
      - card_type: floater
        title: New work
        notes: null
        tags: []
        topic_id: null
        assigned_to:
          type: Group
          name: staff
"#,
        )
        .expect("valid board snapshot");

        assert!(snapshot.board_id.is_none());
        assert!(snapshot.provenance.is_none());
        assert!(snapshot.columns[0].id.is_none());
        assert!(snapshot.columns[0].cards[0].id.is_none());
        assert!(matches!(
            snapshot.columns[0].cards[0].assigned_to,
            Some(BoardSnapshotAssignee::Group { ref name }) if name == "staff"
        ));
    }
}

// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{Board, BoardAssignee, BoardCard, BoardDetail, DiscourseClient};
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::{atomic_write, ensure_output_available};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// List every board accessible to the configured API user.
pub fn board_list(config: &Config, discourse_name: &str, format: ListFormat) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let mut boards = client.list_boards()?;
    boards.sort_by(|a, b| a.name.cmp(&b.name));

    match format {
        ListFormat::Text => print_board_list(&boards),
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&boards)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&boards)?),
    }
    Ok(())
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
    match format {
        ListFormat::Text => print_board_detail(&detail),
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&detail)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&detail)?),
    }
    Ok(())
}

// ─── Pull (snapshot) ────────────────────────────────────────────────────────

/// On-disk board snapshot (schema version 1). API response models are kept out
/// of this schema so response-only and newly introduced fields cannot silently
/// become future `push` input.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct BoardSnapshot {
    pub version: u32,
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

fn is_json_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
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
        assert!(snapshot.columns[0].id.is_none());
        assert!(snapshot.columns[0].cards[0].id.is_none());
        assert!(matches!(
            snapshot.columns[0].cards[0].assigned_to,
            Some(BoardSnapshotAssignee::Group { ref name }) if name == "staff"
        ));
    }
}

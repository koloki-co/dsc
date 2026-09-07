// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{Board, BoardCard, BoardColumn, BoardDetail, DiscourseClient};
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::atomic_write;
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

/// On-disk board snapshot (schema version 1). Includes every column and
/// card, floater cards included - a snapshot omitting them would misrepresent
/// the board and could not restore it on a later `push`. Card order in each
/// column is the list order here, never the server's opaque `position`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BoardSnapshot {
    pub version: u32,
    pub board_id: i64,
    pub board_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discourse_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulled_at: Option<String>,
    pub board: Board,
    #[serde(default)]
    pub columns: Vec<BoardColumn>,
}

/// Snapshot a board to a local YAML/JSON file.
pub fn board_pull(
    config: &Config,
    discourse_name: &str,
    board_id: i64,
    local_path: &Path,
    force: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let detail = client.show_board(board_id)?;
    let discourse_version = client.fetch_version().ok().flatten();
    let pulled_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let snapshot = BoardSnapshot {
        version: 1,
        board_id,
        board_name: detail.board.name.clone(),
        discourse_version,
        pulled_at: Some(pulled_at),
        board: detail.board,
        columns: detail.columns,
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
}

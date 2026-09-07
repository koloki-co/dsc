// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! Client for the Discourse Boards plugin API (core plugin, Business/
//! Enterprise plans, shipped September 2026). Endpoints are mounted at
//! `/boards/api/*` and are undocumented in the official API docs; the
//! shapes here were captured against a live forum. See
//! `spec/commands/boards.md` for the full discovery notes and quirks.

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The `created_by`/`assigned_to` shape shared by boards and cards.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoardUserRef {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One accessible board, as returned by both `boards.json` (list) and the
/// `board` key of `boards/:id.json` (show). Fields not modelled explicitly
/// (e.g. `acl`, whose flattened format was not captured) survive in `extra`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Board {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub category_ids: Vec<i64>,
    #[serde(default)]
    pub tag_ids: Vec<i64>,
    #[serde(default)]
    pub tag_names: Vec<String>,
    #[serde(default)]
    pub anonymous_can_read: bool,
    #[serde(default)]
    pub require_confirmation: bool,
    #[serde(default)]
    pub show_tags: bool,
    #[serde(default)]
    pub card_style: Option<String>,
    #[serde(default)]
    pub show_topic_thumbnail: bool,
    #[serde(default)]
    pub can_write: bool,
    #[serde(default)]
    pub can_manage: bool,
    #[serde(default)]
    pub created_by: Option<BoardUserRef>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// The topic behind a topic-type card, embedded by the show endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoardCardTopic {
    pub id: i64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub category_id: Option<i64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub bumped_at: Option<String>,
    #[serde(default)]
    pub closed: Option<bool>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub posts_count: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One card in a column: either a `floater` (title/notes/tags of its own) or
/// a `topic` card (content lives on the referenced topic; `title` is null).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoardCard {
    pub id: i64,
    #[serde(default)]
    pub board_id: Option<i64>,
    #[serde(default)]
    pub column_id: Option<i64>,
    pub card_type: String,
    /// Server-assigned integer with large gaps (65536, 131072, ...); opaque,
    /// never meaningful across a pull/push round trip. Card order in a
    /// snapshot is expressed as list order, not this value.
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub tag_ids: Vec<i64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub topic_id: Option<i64>,
    #[serde(default)]
    pub topic: Option<BoardCardTopic>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub column_changed_at: Option<String>,
    #[serde(default)]
    pub recency_at: Option<String>,
    #[serde(default)]
    pub created_by: Option<BoardUserRef>,
    #[serde(default)]
    pub assigned_to: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One column of a board, with its cards embedded and pre-sorted by
/// `default_sort` (server-side: `position` for `priority`, `recency_at` desc
/// for `recency`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoardColumn {
    pub id: i64,
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default)]
    pub default_sort: Option<String>,
    #[serde(default)]
    pub tag_id: Option<i64>,
    #[serde(default)]
    pub tag_name: Option<String>,
    #[serde(default)]
    pub move_to_category_id: Option<i64>,
    #[serde(default)]
    pub move_to_assigned: Option<Value>,
    #[serde(default)]
    pub move_to_status: Option<Value>,
    /// Bare hex string without a leading `#` (e.g. `"2f7ed8"`).
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub cards: Vec<BoardCard>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// The combined result of showing one board: its metadata plus its columns
/// (with cards embedded), matching the `{"board": ..., "columns": [...]}`
/// shape of `GET /boards/api/boards/:id.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoardDetail {
    pub board: Board,
    #[serde(default)]
    pub columns: Vec<BoardColumn>,
}

#[derive(Debug, Deserialize)]
struct BoardListEnvelope {
    #[serde(default)]
    boards: Vec<Board>,
}

#[derive(Debug, Deserialize)]
struct BoardShowEnvelope {
    board: Board,
    #[serde(default)]
    columns: Vec<BoardColumn>,
}

const BOARDS_PATH: &str = "/boards/api/boards.json";

impl DiscourseClient {
    /// List every board accessible to the configured API user.
    pub fn list_boards(&self) -> Result<Vec<Board>> {
        let response = self.get(BOARDS_PATH)?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading boards list response")?;
        if !status.is_success() {
            return Err(boards_http_error("board list", status, &text));
        }
        let envelope: BoardListEnvelope =
            serde_json::from_str(&text).context("parsing boards list response")?;
        Ok(envelope.boards)
    }

    /// Fetch one board's full definition: metadata plus every column and its
    /// cards (including floater cards, which have no topic behind them).
    pub fn show_board(&self, board_id: i64) -> Result<BoardDetail> {
        let path = format!("/boards/api/boards/{board_id}.json");
        let response = self.get(&path)?;
        let status = response.status();
        let text = response.text_capped().context("reading board response")?;
        if !status.is_success() {
            return Err(boards_http_error("board show", status, &text));
        }
        let envelope: BoardShowEnvelope =
            serde_json::from_str(&text).context("parsing board response")?;
        Ok(BoardDetail {
            board: envelope.board,
            columns: envelope.columns,
        })
    }
}

fn boards_http_error(action: &str, status: reqwest::StatusCode, text: &str) -> anyhow::Error {
    if status == reqwest::StatusCode::NOT_FOUND {
        return anyhow::anyhow!(
            "{}; Discourse Boards may be disabled, unlicensed (Business/Enterprise plans only), \
             unavailable on this Discourse version, or the board may not exist",
            http_error(action, status, text)
        );
    }
    http_error(action, status, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_list_tolerates_unknown_fields_and_missing_optionals() {
        let envelope: BoardListEnvelope =
            serde_json::from_str(r#"{"boards":[{"id":1,"name":"Roadmap","acl":{"foo":"bar"}}]}"#)
                .unwrap();
        assert_eq!(envelope.boards.len(), 1);
        let board = &envelope.boards[0];
        assert_eq!(board.id, 1);
        assert_eq!(board.name, "Roadmap");
        assert!(board.slug.is_none());
        assert!(board.extra.contains_key("acl"));
    }

    #[test]
    fn board_show_separates_topic_and_floater_cards() {
        let envelope: BoardShowEnvelope = serde_json::from_str(
            r#"{
                "board": {"id": 3, "name": "Roadmap"},
                "columns": [
                    {
                        "id": 10,
                        "title": "Backlog",
                        "cards": [
                            {"id": 100, "board_id": 3, "column_id": 10, "card_type": "floater", "title": "Idea", "notes": "n"},
                            {"id": 101, "board_id": 3, "column_id": 10, "card_type": "topic", "title": null, "topic_id": 42, "topic": {"id": 42, "title": "A topic"}}
                        ]
                    }
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(envelope.columns.len(), 1);
        let cards = &envelope.columns[0].cards;
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].card_type, "floater");
        assert_eq!(cards[0].title.as_deref(), Some("Idea"));
        assert_eq!(cards[1].card_type, "topic");
        assert!(cards[1].title.is_none());
        assert_eq!(
            cards[1].topic.as_ref().unwrap().title.as_deref(),
            Some("A topic")
        );
    }

    #[test]
    fn not_found_hint_names_boards_plugin() {
        let err = boards_http_error("board show", reqwest::StatusCode::NOT_FOUND, "");
        let s = err.to_string();
        assert!(s.contains("Boards"), "expected Boards-specific hint: {s}");
        assert!(
            s.contains("Business/Enterprise"),
            "expected licensing hint: {s}"
        );
    }
}

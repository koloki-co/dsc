// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use super::client::{DiscourseClient, ResponseBody};
use super::error::http_error;
use super::search::urlencode_form;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One row from `/notifications.json`, matching Discourse's
/// `NotificationSerializer`.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Notification {
    pub id: u64,
    /// Numeric `Notification.types` value, e.g. `5` = liked, `6` = private_message.
    pub notification_type: u32,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub high_priority: bool,
    pub created_at: String,
    #[serde(default)]
    pub topic_id: Option<u64>,
    #[serde(default)]
    pub post_number: Option<u32>,
    #[serde(default)]
    pub fancy_title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub acting_user_name: Option<String>,
    #[serde(default)]
    pub is_warning: bool,
    /// A JSON hash whose shape depends on `notification_type`.
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct NotificationsResponse {
    #[serde(default)]
    notifications: Vec<Notification>,
    #[serde(default)]
    total_rows_notifications: Option<u64>,
}

/// One non-`recent` notifications-history page and its total row count before
/// access filtering, as returned by `NotificationsController#index`.
#[derive(Debug)]
pub struct NotificationPage {
    pub notifications: Vec<Notification>,
    pub total_rows: u64,
}

/// Query filters for `/notifications.json`, mirroring
/// `NotificationsController#index`'s non-`recent` pagination mode.
#[derive(Debug, Default, Clone)]
pub struct NotificationFilter<'a> {
    /// `"read"` or `"unread"`. The CLI validates this before sending.
    pub filter: Option<&'a str>,
    /// Rows to fetch. The CLI validates the server's `1..=60` range.
    pub limit: u16,
    /// Zero-based row offset in the newest-first notification history.
    pub offset: u64,
}

impl DiscourseClient {
    /// Fetch a non-`recent`, newest-first page of the API user's notifications.
    pub fn fetch_notifications_page(
        &self,
        filter: &NotificationFilter,
    ) -> Result<NotificationPage> {
        let path = format!("/notifications.json?{}", build_query(filter));
        let response = self.get(&path)?;
        let status = response.status();
        let text = response
            .text_capped()
            .context("reading notifications response")?;
        if !status.is_success() {
            return Err(http_error("notifications request", status, &text));
        }
        let body: NotificationsResponse =
            serde_json::from_str(&text).context("parsing notifications response")?;
        let total_rows = body
            .total_rows_notifications
            .unwrap_or(body.notifications.len() as u64);
        Ok(NotificationPage {
            notifications: body.notifications,
            total_rows,
        })
    }

    /// Mark a single notification as read by ID.
    pub fn mark_notification_read(&self, id: u64) -> Result<()> {
        self.mark_read(&format!("id={id}"))
    }

    /// Mark every unread notification of the given comma-separated
    /// `Notification.types` symbolic names as read.
    pub fn mark_notifications_read_by_type(&self, types: &str) -> Result<()> {
        self.mark_read(&format!("dismiss_types={}", urlencode_form(types)))
    }

    /// Mark every unread notification as read.
    pub fn mark_all_notifications_read(&self) -> Result<()> {
        self.mark_read("")
    }

    fn mark_read(&self, query: &str) -> Result<()> {
        let path = if query.is_empty() {
            "/notifications/mark-read.json".to_string()
        } else {
            format!("/notifications/mark-read.json?{query}")
        };
        let response = self.send_retrying(|| self.put(&path))?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text_capped()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            return Err(http_error("mark notifications read request", status, &text));
        }
        Ok(())
    }
}

/// Build the `application/x-www-form-urlencoded` query string for
/// `/notifications.json` from a filter.
fn build_query(filter: &NotificationFilter) -> String {
    let mut params = vec![format!("limit={}", filter.limit)];
    if let Some(v) = filter.filter {
        params.push(format!("filter={}", urlencode_form(v)));
    }
    if filter.offset > 0 {
        params.push(format!("offset={}", filter.offset));
    }
    params.join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter_only_sends_limit() {
        let filter = NotificationFilter {
            limit: 30,
            ..Default::default()
        };
        assert_eq!(build_query(&filter), "limit=30");
    }

    #[test]
    fn history_filter_and_offset_are_included() {
        let filter = NotificationFilter {
            filter: Some("unread"),
            limit: 60,
            offset: 120,
        };
        assert_eq!(build_query(&filter), "limit=60&filter=unread&offset=120");
    }

    #[test]
    fn deserializes_entry_with_null_optional_fields() {
        let raw = r#"{
            "notifications": [
                {
                    "id": 1,
                    "notification_type": 5,
                    "read": false,
                    "created_at": "2026-07-01T00:00:00.000Z",
                    "topic_id": 42,
                    "post_number": 3,
                    "fancy_title": "Hello world",
                    "slug": "hello-world",
                    "acting_user_name": "alice",
                    "data": {"topic_title": "Hello world"}
                }
            ],
            "total_rows_notifications": 42
        }"#;
        let body: NotificationsResponse = serde_json::from_str(raw).expect("parse");
        assert_eq!(body.notifications.len(), 1);
        let entry = &body.notifications[0];
        assert_eq!(entry.notification_type, 5);
        assert!(!entry.read);
        assert_eq!(entry.acting_user_name.as_deref(), Some("alice"));
        assert_eq!(entry.data.as_ref().unwrap()["topic_title"], "Hello world");
        assert_eq!(body.total_rows_notifications, Some(42));
    }

    #[test]
    fn page_uses_returned_total_or_falls_back_to_response_length() {
        let with_total: NotificationsResponse =
            serde_json::from_str(r#"{"notifications": [], "total_rows_notifications": 12}"#)
                .expect("parse response with total");
        assert_eq!(with_total.total_rows_notifications, Some(12));

        let without_total: NotificationsResponse =
            serde_json::from_str(r#"{"notifications": []}"#).expect("parse response without total");
        assert_eq!(without_total.total_rows_notifications, None);
    }

    #[test]
    fn deserializes_entry_missing_optional_fields() {
        let raw = r#"{
            "notifications": [
                {
                    "id": 2,
                    "notification_type": 6,
                    "read": true,
                    "created_at": "2026-07-01T00:00:00.000Z"
                }
            ]
        }"#;
        let body: NotificationsResponse = serde_json::from_str(raw).expect("parse");
        let entry = body.notifications.first().expect("one entry");
        assert!(entry.read);
        assert!(entry.topic_id.is_none());
        assert!(entry.fancy_title.is_none());
        assert!(entry.data.is_none());
    }
}

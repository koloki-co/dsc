// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! `dsc notification list|read` — inspect and mark read the API user's own
//! Discourse notifications (`/notifications.json`).

use crate::api::{DiscourseClient, Notification, NotificationFilter};
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use anyhow::{Result, anyhow};

/// Inputs controlling one `dsc notification list` request.
#[derive(Clone, Copy)]
pub struct NotificationListOptions<'a> {
    /// `"read"` or `"unread"`. Validated before the request is sent.
    pub filter: Option<&'a str>,
    /// Comma-separated `Notification.types` symbolic names, e.g.
    /// `liked,mentioned`. Validated before the history pages are filtered
    /// locally, because Discourse only honours this filter in `recent` mode.
    pub types: Option<&'a str>,
    /// Max newest-first rows to fetch, from 1 through 60.
    pub limit: u16,
    /// Rendering format.
    pub format: ListFormat,
}

/// Symbolic `Notification.types` names to numeric IDs, captured from
/// Discourse's `Notification.types` enum on 2026-07-19.
const NOTIFICATION_TYPES: &[(&str, u32)] = &[
    ("mentioned", 1),
    ("replied", 2),
    ("quoted", 3),
    ("edited", 4),
    ("liked", 5),
    ("private_message", 6),
    ("invited_to_private_message", 7),
    ("invitee_accepted", 8),
    ("posted", 9),
    ("moved_post", 10),
    ("linked", 11),
    ("granted_badge", 12),
    ("invited_to_topic", 13),
    ("custom", 14),
    ("group_mentioned", 15),
    ("group_message_summary", 16),
    ("watching_first_post", 17),
    ("topic_reminder", 18),
    ("liked_consolidated", 19),
    ("post_approved", 20),
    ("code_review_commit_approved", 21),
    ("membership_request_accepted", 22),
    ("membership_request_consolidated", 23),
    ("bookmark_reminder", 24),
    ("reaction", 25),
    ("votes_released", 26),
    ("event_reminder", 27),
    ("event_invitation", 28),
    ("chat_mention", 29),
    ("chat_message", 30),
    ("chat_invitation", 31),
    ("chat_group_mention", 32),
    ("chat_quoted", 33),
    ("assigned", 34),
    ("question_answer_user_commented", 35),
    ("watching_category_or_tag", 36),
    ("new_features", 37),
    ("admin_problems", 38),
    ("linked_consolidated", 39),
    ("chat_watched_thread", 40),
    ("upcoming_change_available", 41),
    ("upcoming_change_automatically_promoted", 42),
    ("boost", 43),
    ("suggested_edit_created", 44),
    ("suggested_edit_accepted", 45),
    ("following", 800),
    ("following_created_topic", 801),
    ("following_replied", 802),
    ("circles_activity", 900),
];

fn type_name(notification_type: u32) -> &'static str {
    NOTIFICATION_TYPES
        .iter()
        .find(|(_, id)| *id == notification_type)
        .map(|(name, _)| *name)
        .unwrap_or("unknown")
}

/// Validate a `--filter` value against the two values Discourse recognises.
fn validate_filter(filter: Option<&str>) -> Result<Option<&str>> {
    match filter {
        None | Some("read") | Some("unread") => Ok(filter),
        Some(other) => Err(anyhow!(
            "invalid --filter value: {other}\nhint: `--filter` accepts `read` or `unread` only"
        )),
    }
}

/// Validate a comma-separated `--type` list against known
/// `Notification.types` symbolic names.
fn validate_types(types: Option<&str>) -> Result<Option<&str>> {
    let Some(types) = types else {
        return Ok(None);
    };
    for name in types.split(',') {
        if !NOTIFICATION_TYPES.iter().any(|(n, _)| *n == name) {
            return Err(anyhow!(
                "unknown notification type: {name}\n\
                 hint: `--type` accepts comma-separated built-in Discourse notification type \
                 names (e.g. `liked,mentioned`); custom/plugin type names are not supported yet."
            ));
        }
    }
    Ok(Some(types))
}

fn requested_type_ids(types: &str) -> Vec<u32> {
    types
        .split(',')
        .filter_map(|name| {
            NOTIFICATION_TYPES
                .iter()
                .find(|(known_name, _)| *known_name == name)
                .map(|(_, id)| *id)
        })
        .collect()
}

fn retain_requested_types(
    notifications: Vec<Notification>,
    requested_type_ids: &[u32],
    limit: usize,
) -> Vec<Notification> {
    notifications
        .into_iter()
        .filter(|notification| requested_type_ids.contains(&notification.notification_type))
        .take(limit)
        .collect()
}

/// Fetch normal, read-only history pages until the requested number of type
/// matches has been collected or the server reports that history is exhausted.
/// Discourse's server-side `filter_by_types` is restricted to the `recent`
/// endpoint mode, which has different ordering and seen-state semantics.
fn fetch_history_notifications(
    client: &DiscourseClient,
    filter: Option<&str>,
    types: Option<&str>,
    requested_limit: u16,
) -> Result<Vec<Notification>> {
    const HISTORY_PAGE_LIMIT: u16 = 60;

    let requested_type_ids = types.map(requested_type_ids);
    let page_limit = if requested_type_ids.is_some() {
        HISTORY_PAGE_LIMIT
    } else {
        requested_limit
    };
    let mut notifications = Vec::with_capacity(usize::from(requested_limit));
    let mut offset = 0;

    loop {
        let page = client.fetch_notifications_page(&NotificationFilter {
            filter,
            limit: page_limit,
            offset,
        })?;
        let remaining = usize::from(requested_limit).saturating_sub(notifications.len());
        if let Some(type_ids) = requested_type_ids.as_deref() {
            notifications.extend(retain_requested_types(
                page.notifications,
                type_ids,
                remaining,
            ));
        } else {
            notifications.extend(page.notifications.into_iter().take(remaining));
        }

        if notifications.len() == usize::from(requested_limit) || requested_type_ids.is_none() {
            break;
        }
        let next_offset = offset.saturating_add(u64::from(page_limit));
        if next_offset >= page.total_rows {
            break;
        }
        offset = next_offset;
    }

    Ok(notifications)
}

/// List the API user's notifications.
///
/// Without `--type`, this reads one newest-first history page. With `--type`,
/// it scans the same read-only history pages locally until enough matching
/// notifications are found or the history is exhausted.
pub fn notification_list(
    config: &Config,
    discourse_name: &str,
    options: NotificationListOptions,
) -> Result<()> {
    let filter = validate_filter(options.filter)?;
    let types = validate_types(options.types)?;
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let notifications = fetch_history_notifications(&client, filter, types, options.limit)?;

    if notifications.len() == usize::from(options.limit) {
        eprintln!(
            "Showing up to {} newest matching notifications; older entries may exist.",
            options.limit
        );
    }

    match options.format {
        ListFormat::Text => {
            if notifications.is_empty() {
                println!("No notifications found.");
                return Ok(());
            }
            for n in &notifications {
                let state = if n.read { "read" } else { "unread" };
                let title = n.fancy_title.as_deref().unwrap_or("");
                let actor = n.acting_user_name.as_deref().unwrap_or("-");
                println!(
                    "{}  id={:<8}  {:<7}  {:<28}  actor={:<16}  {}",
                    n.created_at,
                    n.id,
                    state,
                    type_name(n.notification_type),
                    actor,
                    title
                );
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&notifications)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&notifications)?),
    }
    Ok(())
}

/// What a `dsc notification read` invocation targets.
enum ReadTarget<'a> {
    Id(u64),
    Types(&'a str),
    All,
}

impl<'a> ReadTarget<'a> {
    fn from_args(id: Option<u64>, types: Option<&'a str>, all: bool) -> Result<Self> {
        match (id, types, all) {
            (Some(id), None, false) => Ok(ReadTarget::Id(id)),
            (None, Some(types), false) => Ok(ReadTarget::Types(types)),
            (None, None, true) => Ok(ReadTarget::All),
            (None, None, false) => Err(anyhow!("specify exactly one of --id, --type, or --all")),
            _ => Err(anyhow!("--id, --type, and --all are mutually exclusive")),
        }
    }

    fn describe(&self, verb: &str) -> String {
        match self {
            ReadTarget::Id(id) => format!("{verb} notification {id} read"),
            ReadTarget::Types(types) => {
                format!("{verb} all unread `{types}` notifications read")
            }
            ReadTarget::All => format!("{verb} all unread notifications read"),
        }
    }
}

/// Mark notification(s) read: a single `id`, every unread notification of the
/// given comma-separated `types`, or (with `all`) every unread notification.
/// Honours `--dry-run`.
pub fn notification_read(
    config: &Config,
    discourse_name: &str,
    id: Option<u64>,
    types: Option<&str>,
    all: bool,
    dry_run: bool,
) -> Result<()> {
    let types = validate_types(types)?;
    let target = ReadTarget::from_args(id, types, all)?;
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!(
            "[dry-run] {}: would {}",
            discourse.name,
            target.describe("mark")
        );
        return Ok(());
    }

    match &target {
        ReadTarget::Id(id) => client.mark_notification_read(*id)?,
        ReadTarget::Types(types) => client.mark_notifications_read_by_type(types)?,
        ReadTarget::All => client.mark_all_notifications_read()?,
    }
    println!("{}: {}", discourse.name, target.describe("marked"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_filter_values_pass_through() {
        assert_eq!(validate_filter(Some("read")).unwrap(), Some("read"));
        assert_eq!(validate_filter(Some("unread")).unwrap(), Some("unread"));
        assert_eq!(validate_filter(None).unwrap(), None);
    }

    #[test]
    fn invalid_filter_value_is_rejected() {
        assert!(validate_filter(Some("archived")).is_err());
    }

    #[test]
    fn known_types_are_accepted() {
        assert_eq!(
            validate_types(Some("liked,mentioned")).unwrap(),
            Some("liked,mentioned")
        );
    }

    #[test]
    fn unknown_type_is_rejected() {
        let error = validate_types(Some("liked,bogus")).unwrap_err();
        assert!(error.to_string().contains("unknown notification type"));
    }

    #[test]
    fn requested_types_filter_mixed_notification_pages() {
        let notifications = vec![
            notification(1, 5),
            notification(2, 6),
            notification(3, 1),
            notification(4, 5),
        ];
        let kept = retain_requested_types(notifications, &[1, 5], 2);
        assert_eq!(
            kept.iter()
                .map(|notification| notification.id)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
    }

    fn notification(id: u64, notification_type: u32) -> Notification {
        Notification {
            id,
            notification_type,
            read: false,
            high_priority: false,
            created_at: "2026-07-01T00:00:00Z".to_string(),
            topic_id: None,
            post_number: None,
            fancy_title: None,
            slug: None,
            acting_user_name: None,
            is_warning: false,
            data: None,
        }
    }

    #[test]
    fn type_name_looks_up_known_ids_and_falls_back() {
        assert_eq!(type_name(5), "liked");
        assert_eq!(type_name(6), "private_message");
        assert_eq!(type_name(999), "unknown");
    }

    #[test]
    fn read_target_requires_exactly_one_selector() {
        assert!(ReadTarget::from_args(None, None, false).is_err());
        assert!(ReadTarget::from_args(Some(1), Some("liked"), false).is_err());
        assert!(ReadTarget::from_args(Some(1), None, true).is_err());
        assert!(ReadTarget::from_args(None, Some("liked"), true).is_err());
        assert!(matches!(
            ReadTarget::from_args(Some(1), None, false),
            Ok(ReadTarget::Id(1))
        ));
        assert!(matches!(
            ReadTarget::from_args(None, None, true),
            Ok(ReadTarget::All)
        ));
        assert!(matches!(
            ReadTarget::from_args(None, Some("liked"), false),
            Ok(ReadTarget::Types("liked"))
        ));
    }

    #[test]
    fn describe_mentions_the_target() {
        assert_eq!(
            ReadTarget::Id(42).describe("mark"),
            "mark notification 42 read"
        );
        assert_eq!(
            ReadTarget::Types("liked").describe("marked"),
            "marked all unread `liked` notifications read"
        );
        assert_eq!(
            ReadTarget::All.describe("mark"),
            "mark all unread notifications read"
        );
    }
}

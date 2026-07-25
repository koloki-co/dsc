// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

//! `dsc log staff` — read-only access to the staff action log (the admin
//! audit trail behind `/admin/logs/staff-action-logs` in the web UI).

use crate::api::{DiscourseClient, StaffActionLogFilter};
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::Config;
use crate::utils::parse_since_cutoff;
use anyhow::{Result, anyhow};
use chrono::SecondsFormat;

/// Inputs controlling one `dsc log staff` request.
#[derive(Clone, Copy)]
pub struct StaffLogOptions<'a> {
    /// Supported core `UserHistory` action name.
    pub action: Option<&'a str>,
    /// Staff username that performed the action.
    pub acting_user: Option<&'a str>,
    /// Username targeted by the action.
    pub target_user: Option<&'a str>,
    /// Exact value of the log entry's subject field.
    pub subject: Option<&'a str>,
    /// Relative duration or ISO-8601 cutoff.
    pub since: Option<&'a str>,
    /// Maximum newest-first rows to return, from 1 through 200.
    pub limit: u16,
    /// Rendering format.
    pub format: ListFormat,
}

/// Core action names that can safely be sent as `action_name` to Discourse.
///
/// Captured from Discourse `UserHistory.actions` / `staff_actions` on
/// 2026-07-17. Plugin and `custom_staff` type names deliberately do not appear:
/// passing one as `action_name` can make Discourse omit its action predicate.
const CORE_STAFF_ACTIONS: &[&str] = &[
    "delete_user",
    "change_trust_level",
    "change_site_setting",
    "change_theme",
    "delete_theme",
    "change_site_text",
    "suspend_user",
    "unsuspend_user",
    "removed_suspend_user",
    "removed_unsuspend_user",
    "grant_badge",
    "revoke_badge",
    "check_email",
    "delete_post",
    "delete_topic",
    "impersonate",
    "roll_up",
    "change_username",
    "anonymize_user",
    "reviewed_post",
    "change_category_settings",
    "delete_category",
    "create_category",
    "silence_user",
    "unsilence_user",
    "removed_silence_user",
    "removed_unsilence_user",
    "grant_admin",
    "revoke_admin",
    "grant_moderation",
    "revoke_moderation",
    "backup_create",
    "revoke_email",
    "deactivate_user",
    "lock_trust_level",
    "unlock_trust_level",
    "activate_user",
    "change_readonly_mode",
    "backup_download",
    "backup_destroy",
    "post_locked",
    "post_unlocked",
    "check_personal_message",
    "disabled_second_factor",
    "post_edit",
    "topic_published",
    "recover_topic",
    "recover_post",
    "post_approved",
    "create_badge",
    "change_badge",
    "delete_badge",
    "post_rejected",
    "merge_user",
    "entity_export",
    "change_name",
    "topic_timestamps_changed",
    "approve_user",
    "web_hook_create",
    "web_hook_update",
    "web_hook_destroy",
    "web_hook_deactivate",
    "embeddable_host_create",
    "embeddable_host_update",
    "embeddable_host_destroy",
    "change_theme_setting",
    "disable_theme_component",
    "enable_theme_component",
    "revoke_title",
    "change_title",
    "api_key_create",
    "api_key_update",
    "api_key_destroy",
    "override_upload_secure_status",
    "page_published",
    "page_unpublished",
    "add_email",
    "update_email",
    "destroy_email",
    "topic_closed",
    "topic_opened",
    "topic_archived",
    "topic_unarchived",
    "post_staff_note_create",
    "post_staff_note_destroy",
    "watched_word_create",
    "watched_word_destroy",
    "delete_group",
    "permanently_delete_post_revisions",
    "create_public_sidebar_section",
    "update_public_sidebar_section",
    "destroy_public_sidebar_section",
    "reset_bounce_score",
    "create_watched_word_group",
    "update_watched_word_group",
    "delete_watched_word_group",
    "topic_slow_mode_set",
    "topic_slow_mode_removed",
    "custom_emoji_create",
    "custom_emoji_destroy",
    "delete_post_permanently",
    "delete_topic_permanently",
    "tag_group_create",
    "tag_group_destroy",
    "tag_group_change",
    "delete_associated_accounts",
    "change_theme_site_setting",
    "stop_impersonating",
    "upcoming_change_toggled",
    "change_site_setting_groups",
    "upcoming_change_available",
    "change_access_control_list_permissions",
];

/// List staff action log entries from one forum.
///
/// The endpoint returns one newest-first page. `limit` is deliberately bounded
/// to Discourse's documented maximum of 200 so callers are never silently
/// given fewer rows than they requested.
pub fn log_staff(config: &Config, discourse_name: &str, options: StaffLogOptions) -> Result<()> {
    let action = validate_core_action(options.action)?;
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let start_date = since_to_start_date(options.since)?;
    let filter = StaffActionLogFilter {
        action_name: action,
        acting_user: options.acting_user,
        target_user: options.target_user,
        subject: options.subject,
        start_date: start_date.as_deref(),
        limit: options.limit,
    };
    let entries = client.fetch_staff_action_logs(&filter)?;

    if entries.len() == usize::from(options.limit) {
        eprintln!(
            "Showing up to {} newest matching entries; older entries may exist.",
            options.limit
        );
    }

    match options.format {
        ListFormat::Text => {
            if entries.is_empty() {
                println!("No staff action log entries found.");
                return Ok(());
            }
            for entry in &entries {
                let actor = entry
                    .acting_user
                    .as_ref()
                    .map(|user| user.username.as_str())
                    .unwrap_or("-");
                let target = entry
                    .target_user
                    .as_ref()
                    .map(|user| user.username.as_str())
                    .unwrap_or("-");
                let subject = entry.subject.as_deref().unwrap_or("");
                println!(
                    "{}  {:<28}  actor={:<16}  target={:<16}  {}",
                    entry.created_at, entry.action_name, actor, target, subject
                );
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&entries)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&entries)?),
    }
    Ok(())
}

fn validate_core_action(action: Option<&str>) -> Result<Option<&str>> {
    let Some(action) = action else {
        return Ok(None);
    };
    if CORE_STAFF_ACTIONS.contains(&action) {
        return Ok(Some(action));
    }
    Err(anyhow!(
        "unknown or unsupported core staff action: {action}\n\
         hint: `--action` accepts built-in Discourse actions only; custom/plugin \n\
         action types are not supported yet. Omit `--action` to browse recent \n\
         entries and find their action names."
    ))
}

/// Convert a `--since` value (relative duration or ISO-8601) into the RFC 3339
/// timestamp Discourse's `start_date` filter parses with `to_time`.
fn since_to_start_date(since: Option<&str>) -> Result<Option<String>> {
    since
        .map(parse_since_cutoff)
        .transpose()
        .map(|cutoff| cutoff.map(|time| time.to_rfc3339_opts(SecondsFormat::Secs, true)))
}

#[cfg(test)]
mod tests {
    use super::{since_to_start_date, validate_core_action};
    use chrono::{DateTime, Utc};

    #[test]
    fn none_since_yields_none_start_date() {
        assert_eq!(since_to_start_date(None).unwrap(), None);
    }

    #[test]
    fn relative_duration_yields_an_rfc3339_timestamp() {
        let timestamp = since_to_start_date(Some("7d")).unwrap().unwrap();
        let parsed = DateTime::parse_from_rfc3339(&timestamp).expect("RFC 3339 timestamp");
        assert!(parsed.with_timezone(&Utc) < Utc::now());
    }

    #[test]
    fn iso_date_becomes_midnight_utc() {
        let got = since_to_start_date(Some("2026-04-01")).unwrap();
        assert_eq!(got, Some("2026-04-01T00:00:00Z".to_string()));
    }

    #[test]
    fn iso_timestamp_preserves_the_instant() {
        let got = since_to_start_date(Some("2026-04-01T13:30:00+01:00")).unwrap();
        assert_eq!(got, Some("2026-04-01T12:30:00Z".to_string()));
    }

    #[test]
    fn invalid_since_is_an_error() {
        assert!(since_to_start_date(Some("not-a-date")).is_err());
    }

    #[test]
    fn known_core_action_is_accepted() {
        assert_eq!(
            validate_core_action(Some("change_site_setting")).unwrap(),
            Some("change_site_setting")
        );
    }

    #[test]
    fn unknown_or_custom_action_is_rejected_before_requesting_logs() {
        let error = validate_core_action(Some("my_plugin_action")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unknown or unsupported core staff action")
        );
        assert!(validate_core_action(Some("custom_staff")).is_err());
    }
}

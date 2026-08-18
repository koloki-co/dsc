// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{DiscourseClient, UserAction, UserSummary};
use crate::cli::ListFormat;
use crate::commands::common::{ensure_api_credentials, select_discourse};
use crate::config::{Config, DiscourseConfig};
use crate::utils::{normalize_baseurl, parse_since_cutoff};
use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use std::io::{self, Read};

pub fn user_list(
    config: &Config,
    discourse_name: &str,
    listing: &str,
    page: u32,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let users = client.admin_list_users(listing, page)?;

    match format {
        ListFormat::Text => {
            if users.is_empty() {
                println!("No users found in listing '{}'.", listing);
                return Ok(());
            }
            let name_width = users
                .iter()
                .map(|u| u.username.len())
                .max()
                .unwrap_or(0)
                .max(8);
            for u in &users {
                let flag = if u.admin.unwrap_or(false) {
                    "admin"
                } else if u.moderator.unwrap_or(false) {
                    "mod"
                } else if u.suspended.unwrap_or(false) {
                    "suspended"
                } else if u.silenced.unwrap_or(false) {
                    "silenced"
                } else {
                    "-"
                };
                let tl = u
                    .trust_level
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "?".to_string());
                println!(
                    "{:<width$}  id:{}  tl:{}  {}",
                    u.username,
                    u.id,
                    tl,
                    flag,
                    width = name_width
                );
            }
        }
        ListFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&users)?);
        }
        ListFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&users)?);
        }
    }

    Ok(())
}

pub fn user_info(
    config: &Config,
    discourse_name: &str,
    username: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let detail = client.fetch_user_detail(username)?;

    match format {
        ListFormat::Text => {
            println!("id:          {}", detail.id);
            println!("username:    {}", detail.username);
            if let Some(name) = &detail.name {
                println!("name:        {}", name);
            }
            if let Some(email) = &detail.email {
                println!("email:       {}", email);
            }
            if let Some(tl) = detail.trust_level {
                println!("trust_level: {}", tl);
            }
            if detail.admin.unwrap_or(false) {
                println!("role:        admin");
            } else if detail.moderator.unwrap_or(false) {
                println!("role:        moderator");
            }
            if let Some(until) = &detail.suspended_till {
                println!("suspended:   until {}", until);
            }
            if let Some(until) = &detail.silenced_till {
                println!("silenced:    until {}", until);
            }
            if let Some(last) = &detail.last_seen_at {
                println!("last_seen:   {}", last);
            }
            if let Some(created) = &detail.created_at {
                println!("created:     {}", created);
            }
            if let Some(posts) = detail.post_count {
                println!("posts:       {}", posts);
            }
            if !detail.groups.is_empty() {
                println!("groups:      {}", detail.groups.len());
            }
        }
        ListFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&detail)?);
        }
        ListFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&detail)?);
        }
    }
    Ok(())
}

/// GDPR "which forum has this person" lookup: search every configured forum
/// for an account whose email matches, and print forum-tagged matches.
/// Continues past per-forum failures (missing credentials, unreachable
/// forum) so one bad entry doesn't block the rest of the fleet; fails at the
/// end if any forum could not be searched.
pub fn user_find(config: &Config, email: &str, format: ListFormat) -> Result<()> {
    if !email.contains('@') {
        return Err(anyhow!("invalid email: {:?}", email));
    }
    if config.discourse.is_empty() {
        return Err(anyhow!("no discourses configured"));
    }

    let mut matches: Vec<ForumMatch> = Vec::new();
    let mut failed = 0usize;
    for discourse in &config.discourse {
        match find_one(discourse, email) {
            Ok(users) => matches.extend(users.into_iter().map(|user| ForumMatch {
                forum: discourse.name.clone(),
                id: user.id,
                username: user.username,
            })),
            Err(e) => {
                failed += 1;
                eprintln!("{}: user search failed - {e}", discourse.name);
            }
        }
    }

    match format {
        ListFormat::Text => {
            if matches.is_empty() {
                println!("No account found on any configured forum.");
            } else {
                let forum_width = matches.iter().map(|m| m.forum.len()).max().unwrap_or(4);
                for m in &matches {
                    println!(
                        "{:<forum_width$}  {}  id:{}",
                        m.forum,
                        m.username,
                        m.id,
                        forum_width = forum_width
                    );
                }
            }
        }
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&matches)?),
        ListFormat::Yaml => println!("{}", serde_yaml::to_string(&matches)?),
    }

    if failed > 0 {
        return Err(anyhow!(
            "user search failed on {failed} of {} forum(s)",
            config.discourse.len()
        ));
    }
    Ok(())
}

fn find_one(discourse: &DiscourseConfig, email: &str) -> Result<Vec<UserSummary>> {
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let hits = client.admin_search_users(email)?;
    // admin_search_users matches username/name/email loosely; narrow to an
    // exact (case-insensitive) email match so a GDPR lookup doesn't report
    // unrelated accounts that merely share a substring.
    Ok(hits
        .into_iter()
        .filter(|u| {
            u.email
                .as_deref()
                .is_some_and(|e| e.eq_ignore_ascii_case(email))
        })
        .collect())
}

#[derive(Serialize)]
struct ForumMatch {
    forum: String,
    id: i64,
    username: String,
}

pub fn user_suspend(
    config: &Config,
    discourse_name: &str,
    username: &str,
    until: &str,
    reason: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!(
            "[dry-run] {}: would suspend {} until {} (reason: {})",
            discourse.name,
            username,
            until,
            if reason.is_empty() { "<none>" } else { reason }
        );
        return Ok(());
    }

    let detail = client.fetch_user_detail(username)?;
    client.suspend_user(detail.id, until, reason)?;
    println!(
        "Suspended {} (id:{}) until {}",
        detail.username, detail.id, until
    );
    Ok(())
}

pub fn user_unsuspend(
    config: &Config,
    discourse_name: &str,
    username: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!("[dry-run] {}: would unsuspend {}", discourse.name, username);
        return Ok(());
    }

    let detail = client.fetch_user_detail(username)?;
    client.unsuspend_user(detail.id)?;
    println!("Unsuspended {} (id:{})", detail.username, detail.id);
    Ok(())
}

pub fn user_silence(
    config: &Config,
    discourse_name: &str,
    username: &str,
    until: &str,
    reason: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!(
            "[dry-run] {}: would silence {}{}{}",
            discourse.name,
            username,
            if until.is_empty() {
                String::new()
            } else {
                format!(" until {}", until)
            },
            if reason.is_empty() {
                String::new()
            } else {
                format!(" (reason: {})", reason)
            },
        );
        return Ok(());
    }

    let detail = client.fetch_user_detail(username)?;
    client.silence_user(detail.id, until, reason)?;
    println!("Silenced {} (id:{})", detail.username, detail.id);
    Ok(())
}

pub fn user_unsilence(
    config: &Config,
    discourse_name: &str,
    username: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!("[dry-run] {}: would unsilence {}", discourse.name, username);
        return Ok(());
    }

    let detail = client.fetch_user_detail(username)?;
    client.unsilence_user(detail.id)?;
    println!("Unsilenced {} (id:{})", detail.username, detail.id);
    Ok(())
}

#[derive(Clone, Copy)]
pub enum Role {
    Admin,
    Moderator,
}

pub fn user_promote(
    config: &Config,
    discourse_name: &str,
    username: &str,
    role: Role,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let role_label = match role {
        Role::Admin => "admin",
        Role::Moderator => "moderator",
    };

    if dry_run {
        println!(
            "[dry-run] {}: would grant {} to {}",
            discourse.name, role_label, username
        );
        return Ok(());
    }

    let detail = client.fetch_user_detail(username)?;
    match role {
        Role::Admin => client.grant_admin(detail.id)?,
        Role::Moderator => client.grant_moderation(detail.id)?,
    }
    println!(
        "Granted {} to {} (id:{})",
        role_label, detail.username, detail.id
    );
    Ok(())
}

pub fn user_demote(
    config: &Config,
    discourse_name: &str,
    username: &str,
    role: Role,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let role_label = match role {
        Role::Admin => "admin",
        Role::Moderator => "moderator",
    };

    if dry_run {
        println!(
            "[dry-run] {}: would revoke {} from {}",
            discourse.name, role_label, username
        );
        return Ok(());
    }

    let detail = client.fetch_user_detail(username)?;
    match role {
        Role::Admin => client.revoke_admin(detail.id)?,
        Role::Moderator => client.revoke_moderation(detail.id)?,
    }
    println!(
        "Revoked {} from {} (id:{})",
        role_label, detail.username, detail.id
    );
    Ok(())
}

pub fn user_groups_list(
    config: &Config,
    discourse_name: &str,
    username: &str,
    format: ListFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    let mut groups = client.fetch_user_groups(username)?;
    groups.sort_by(|a, b| a.name.cmp(&b.name));

    match format {
        ListFormat::Text => {
            if groups.is_empty() {
                println!("{} is not in any groups.", username);
                return Ok(());
            }
            let name_width = groups
                .iter()
                .map(|g| g.name.len())
                .max()
                .unwrap_or(0)
                .max(4);
            for g in &groups {
                println!("{:<width$}  id:{}", g.name, g.id, width = name_width);
            }
        }
        ListFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&groups)?);
        }
        ListFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&groups)?);
        }
    }

    Ok(())
}

pub fn user_groups_add(
    config: &Config,
    discourse_name: &str,
    username: &str,
    group_id: u64,
    notify: bool,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!(
            "[dry-run] {}: would add {} to group {} (notify={})",
            discourse.name, username, group_id, notify
        );
        return Ok(());
    }

    let usernames = vec![username.to_string()];
    let outcome = client.add_group_members_by_username(group_id, &usernames, notify)?;
    if outcome.added_usernames.is_empty() {
        println!(
            "{} was already a member of group {} (or Discourse reported no change)",
            username, group_id
        );
    } else {
        println!("Added {} to group {}", username, group_id);
    }
    if !outcome.errors.is_empty() {
        eprintln!("Server notes:");
        for msg in &outcome.errors {
            eprintln!("  - {}", msg);
        }
    }
    Ok(())
}

pub fn user_groups_remove(
    config: &Config,
    discourse_name: &str,
    username: &str,
    group_id: u64,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!(
            "[dry-run] {}: would remove {} from group {}",
            discourse.name, username, group_id
        );
        return Ok(());
    }

    let usernames = vec![username.to_string()];
    client.remove_group_members_by_username(group_id, &usernames)?;
    println!("Removed {} from group {}", username, group_id);
    Ok(())
}

/// Output format variants for `dsc user activity`. Superset of ListFormat —
/// adds `markdown` and `csv`.
#[derive(Clone, Copy)]
pub enum ActivityFormat {
    Text,
    Json,
    Yaml,
    Markdown,
    Csv,
}

/// Fetch a user's recent activity and render it.
pub fn user_activity(
    config: &Config,
    discourse_name: &str,
    username: &str,
    type_names: &[String],
    since: Option<&str>,
    limit: Option<u32>,
    format: ActivityFormat,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    // Activity is read-only, and Discourse's user_actions.json is public for
    // forums that allow anonymous read. Skip the apikey/api_username guard so
    // this command works on forums where the caller only has a config entry
    // with baseurl (no admin access). If the forum is login-walled, the API
    // call will surface a 403/401 with the normal credentials hint.
    let client = DiscourseClient::new(discourse)?;

    let filter_types = resolve_activity_types(type_names)?;
    let cutoff = match since {
        Some(raw) => Some(parse_since_cutoff(raw)?),
        None => None,
    };

    let mut collected: Vec<UserAction> = Vec::new();
    let mut offset: u32 = 0;
    let page_hint: u32 = 30; // Discourse returns ~10-30 depending on version
    let max = limit.unwrap_or(u32::MAX);
    loop {
        let page = client.fetch_user_actions(username, &filter_types, offset)?;
        if page.is_empty() {
            break;
        }
        let page_len = page.len() as u32;

        let mut past_cutoff = false;
        for action in page {
            if let Some(cutoff) = cutoff
                && let Ok(created) = chrono::DateTime::parse_from_rfc3339(&action.created_at)
                && created.with_timezone(&chrono::Utc) < cutoff
            {
                past_cutoff = true;
                continue;
            }
            collected.push(action);
            if collected.len() as u32 >= max {
                break;
            }
        }

        if past_cutoff || collected.len() as u32 >= max {
            break;
        }
        offset = offset.saturating_add(page_len.max(page_hint));
    }

    render_activity(&collected, &normalize_baseurl(&discourse.baseurl), format)
}

fn render_activity(actions: &[UserAction], baseurl: &str, format: ActivityFormat) -> Result<()> {
    match format {
        ActivityFormat::Text => {
            if actions.is_empty() {
                println!("No activity in that window.");
                return Ok(());
            }
            for a in actions {
                let date = a.created_at.split('T').next().unwrap_or(&a.created_at);
                let title = a.title.as_deref().unwrap_or("(untitled)");
                let kind = action_type_label(a.action_type);
                println!(
                    "{}  [{:<6}]  {}  {}",
                    date,
                    kind,
                    title,
                    activity_url(baseurl, a)
                );
            }
        }
        ActivityFormat::Markdown => {
            for a in actions {
                let date = a.created_at.split('T').next().unwrap_or(&a.created_at);
                let title = a.title.as_deref().unwrap_or("(untitled)");
                println!("- [{}]({}) — {}", title, activity_url(baseurl, a), date);
            }
        }
        ActivityFormat::Csv => {
            println!("date,type,title,url");
            for a in actions {
                let date = a.created_at.split('T').next().unwrap_or(&a.created_at);
                let title = a.title.as_deref().unwrap_or("").replace('"', "\"\"");
                println!(
                    "{},{},\"{}\",{}",
                    date,
                    action_type_label(a.action_type),
                    title,
                    activity_url(baseurl, a)
                );
            }
        }
        ActivityFormat::Json => println!("{}", serde_json::to_string_pretty(&actions)?),
        ActivityFormat::Yaml => println!("{}", serde_yaml::to_string(&actions)?),
    }
    Ok(())
}

/// Construct the public URL for a user-action row.
pub(crate) fn activity_url(baseurl: &str, a: &UserAction) -> String {
    let slug = a.slug.as_deref().unwrap_or("-");
    match a.post_number {
        Some(n) if n > 1 => format!("{}/t/{}/{}/{}", baseurl, slug, a.topic_id, n),
        _ => format!("{}/t/{}/{}", baseurl, slug, a.topic_id),
    }
}

/// Discourse's UserAction::Types numeric constants.
const ACTION_LIKE: u32 = 1;
const ACTION_NEW_TOPIC: u32 = 4;
const ACTION_REPLY: u32 = 5;
const ACTION_RESPONSE: u32 = 6;
const ACTION_MENTION: u32 = 7;
const ACTION_QUOTE: u32 = 9;
const ACTION_EDIT: u32 = 11;

fn action_type_label(n: u32) -> &'static str {
    match n {
        ACTION_LIKE => "like",
        ACTION_NEW_TOPIC => "topic",
        ACTION_REPLY => "reply",
        ACTION_RESPONSE => "resp",
        ACTION_MENTION => "@",
        ACTION_QUOTE => "quote",
        ACTION_EDIT => "edit",
        _ => "?",
    }
}

/// Map friendly type names on the command line to Discourse's numeric filters.
pub(crate) fn resolve_activity_types(names: &[String]) -> Result<Vec<u32>> {
    if names.is_empty() {
        return Ok(vec![ACTION_NEW_TOPIC, ACTION_REPLY]);
    }
    let mut out = Vec::new();
    for raw in names {
        for piece in raw.split(',') {
            let piece = piece.trim().to_ascii_lowercase();
            if piece.is_empty() {
                continue;
            }
            let n = match piece.as_str() {
                "topic" | "topics" | "new_topic" => ACTION_NEW_TOPIC,
                "reply" | "replies" => ACTION_REPLY,
                "response" | "responses" => ACTION_RESPONSE,
                "mention" | "mentions" => ACTION_MENTION,
                "quote" | "quotes" => ACTION_QUOTE,
                "like" | "likes" => ACTION_LIKE,
                "edit" | "edits" => ACTION_EDIT,
                other => {
                    return Err(anyhow!(
                        "unknown activity type: {:?} (known: topics, replies, mentions, quotes, likes, edits, responses)",
                        other
                    ));
                }
            };
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::UserAction;

    #[test]
    fn default_activity_types_are_topics_and_replies() {
        assert_eq!(
            resolve_activity_types(&[]).unwrap(),
            vec![ACTION_NEW_TOPIC, ACTION_REPLY]
        );
    }

    #[test]
    fn activity_types_accept_plural_and_csv() {
        let got = resolve_activity_types(&["topics,mentions".to_string()]).unwrap();
        assert_eq!(got, vec![ACTION_NEW_TOPIC, ACTION_MENTION]);
    }

    #[test]
    fn activity_types_dedupe() {
        let got = resolve_activity_types(&["reply,reply,replies".to_string()]).unwrap();
        assert_eq!(got, vec![ACTION_REPLY]);
    }

    #[test]
    fn activity_types_reject_unknown() {
        assert!(resolve_activity_types(&["nonsense".to_string()]).is_err());
    }

    #[test]
    fn activity_url_for_reply_includes_post_number() {
        let a = UserAction {
            action_type: ACTION_REPLY,
            created_at: "2026-04-15T12:00:00Z".to_string(),
            title: Some("Hi".to_string()),
            slug: Some("hi-there".to_string()),
            topic_id: 42,
            post_id: Some(999),
            post_number: Some(3),
            username: Some("alice".to_string()),
            excerpt: None,
        };
        assert_eq!(
            activity_url("https://f.example", &a),
            "https://f.example/t/hi-there/42/3"
        );
    }

    #[test]
    fn activity_url_for_op_omits_post_number() {
        let a = UserAction {
            action_type: ACTION_NEW_TOPIC,
            created_at: "2026-04-15T12:00:00Z".to_string(),
            title: Some("Hi".to_string()),
            slug: Some("hi-there".to_string()),
            topic_id: 42,
            post_id: Some(999),
            post_number: Some(1),
            username: Some("alice".to_string()),
            excerpt: None,
        };
        assert_eq!(
            activity_url("https://f.example", &a),
            "https://f.example/t/hi-there/42"
        );
    }
}

// A command handler whose parameters map 1:1 to CLI inputs; bundling them into
// a struct would just move the list without adding clarity.
#[allow(clippy::too_many_arguments)]
pub fn user_create(
    config: &Config,
    discourse_name: &str,
    email: &str,
    username: &str,
    name: Option<&str>,
    password_from_stdin: bool,
    approve: bool,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if !email.contains('@') {
        return Err(anyhow!("invalid email: {:?}", email));
    }
    if username.trim().is_empty() {
        return Err(anyhow!("username cannot be empty"));
    }

    let password = if password_from_stdin {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .context("reading password from stdin")?;
        let trimmed = buf.trim_end_matches(['\r', '\n']).to_string();
        if trimmed.is_empty() {
            return Err(anyhow!("--password-stdin set but stdin was empty"));
        }
        Some(trimmed)
    } else {
        None
    };

    if dry_run {
        println!(
            "[dry-run] {}: would create user {} ({}){}{}{}",
            discourse.name,
            username,
            email,
            name.filter(|n| !n.is_empty())
                .map(|n| format!(", name=\"{}\"", n))
                .unwrap_or_default(),
            if password.is_some() {
                ", with password from stdin"
            } else {
                ", no password (triggers email reset flow)"
            },
            if approve { ", approved=true" } else { "" }
        );
        return Ok(());
    }

    let id = client.create_user(email, username, password.as_deref(), name, approve)?;
    println!("Created user {} (id:{})", username, id);
    if password.is_none() {
        println!(
            "  no password set — send them a reset email with:\n    dsc user password-reset {} {}",
            discourse.name, username
        );
    }
    Ok(())
}

pub fn user_password_reset(
    config: &Config,
    discourse_name: &str,
    username: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if dry_run {
        println!(
            "[dry-run] {}: would trigger password-reset email for {}",
            discourse.name, username
        );
        return Ok(());
    }

    client.trigger_password_reset(username)?;
    // Discourse deliberately returns a generic success even for unknown
    // users to prevent enumeration — surface that so the caller doesn't
    // over-trust the result.
    println!(
        "Password reset request sent for {} (if that user exists).",
        username
    );
    Ok(())
}

pub fn user_email_set(
    config: &Config,
    discourse_name: &str,
    username: &str,
    email: &str,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;

    if !email.contains('@') {
        return Err(anyhow!("invalid email: {:?}", email));
    }

    if dry_run {
        println!(
            "[dry-run] {}: would set {}'s email to {}",
            discourse.name, username, email
        );
        return Ok(());
    }

    client.set_user_email(username, email)?;
    println!("Set {}'s email to {}", username, email);
    Ok(())
}

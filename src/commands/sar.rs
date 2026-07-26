// SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd
//
// SPDX-License-Identifier: GPL-2.0-or-later

use crate::api::{DiscourseClient, PmTopicSummary, TopicResponse, UserAction};
use crate::commands::common::{ensure_api_credentials, not_found, select_discourse};
use crate::config::Config;
use crate::utils::{
    atomic_write_private, current_utc_iso8601, ensure_private_dir, normalize_baseurl, slugify,
    write_markdown_private,
};
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;

/// The person a SAR bundle is about, resolved from a username or email.
struct Subject {
    user_id: i64,
    username: String,
    email: Option<String>,
}

/// Item counts for the manifest and the closing summary.
struct SectionCounts {
    posts: usize,
    likes: usize,
    groups: usize,
    messages: usize,
}

/// Discourse user-action type ids we care about for a SAR.
const ACTION_NEW_TOPIC: u32 = 4;
const ACTION_REPLY: u32 = 5;
const ACTION_LIKE: u32 = 1;

/// Produce a one-shot Subject Access Request bundle for `user` on one forum.
/// Collects the admin PII view, authored posts (full raw), likes, and group
/// memberships into a reviewable directory; private messages are included only
/// when `include_messages` is set (they carry third-party data). See
/// spec/subject-access-request.md - this automates the data-gathering, not the
/// controller's legal judgement.
pub fn sar(
    config: &Config,
    discourse_name: &str,
    user: &str,
    output: Option<&Path>,
    include_messages: bool,
    dry_run: bool,
) -> Result<()> {
    let discourse = select_discourse(config, Some(discourse_name))?;
    ensure_api_credentials(discourse)?;
    let client = DiscourseClient::new(discourse)?;
    let base = normalize_baseurl(&discourse.baseurl);

    let subject = resolve_subject(&client, user)?;
    let generated_at = current_utc_iso8601();
    let dir = match output {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir()?.join(format!(
            "sar-{}-{}",
            slugify(&subject.username),
            date_part(&generated_at)
        )),
    };

    if dry_run {
        println!(
            "[dry-run] would write SAR bundle for {} (user {}) on {} to {}",
            subject.username,
            subject.user_id,
            discourse.name,
            dir.display()
        );
        println!(
            "  sections: profile, posts, activity, groups{}",
            if include_messages { ", messages" } else { "" }
        );
        if !include_messages {
            println!("  (private messages excluded; pass --messages to include them)");
        }
        return Ok(());
    }

    ensure_private_dir(&dir)?;

    // Profile / PII (admin view) and the group memberships embedded in it.
    let admin_detail = client.fetch_admin_user_detail(subject.user_id)?;
    let profile = admin_detail
        .get("user")
        .cloned()
        .unwrap_or_else(|| admin_detail.clone());
    write_json(&dir.join("profile.json"), &profile)?;
    let groups = profile.get("groups").cloned().unwrap_or_else(|| json!([]));
    write_json(&dir.join("groups.json"), &groups)?;

    // Authored posts, with full raw content fetched per post.
    let post_actions = collect_all_actions(
        &client,
        &subject.username,
        &[ACTION_NEW_TOPIC, ACTION_REPLY],
    )?;
    let posts_dir = dir.join("posts");
    ensure_private_dir(&posts_dir)?;
    let mut posts_json: Vec<Value> = Vec::with_capacity(post_actions.len());
    for action in &post_actions {
        let raw = action
            .post_id
            .and_then(|pid| client.fetch_post_raw(pid).ok().flatten());
        if let (Some(pid), Some(body)) = (action.post_id, raw.as_deref()) {
            let stem = action
                .slug
                .as_deref()
                .map(slugify)
                .unwrap_or_else(|| "topic".to_string());
            let md = render_post_md(action, body, &base);
            write_markdown_private(&posts_dir.join(format!("{}-{}.md", stem, pid)), &md, true)?;
        }
        posts_json.push(json!({
            "post_id": action.post_id,
            "topic_id": action.topic_id,
            "title": action.title,
            "url": post_url(&base, action),
            "created_at": action.created_at,
            "raw": raw,
        }));
    }
    write_json(&dir.join("posts.json"), &Value::Array(posts_json))?;

    // Likes given.
    let likes = collect_all_actions(&client, &subject.username, &[ACTION_LIKE])?;
    let likes_json: Vec<Value> = likes.iter().map(action_to_json).collect();
    write_json(
        &dir.join("activity.json"),
        &json!({ "likes_given": likes_json }),
    )?;

    // Private messages (opt-in; third-party data).
    let message_count = if include_messages {
        collect_messages(&client, &subject.username, &dir)?
    } else {
        0
    };

    let counts = SectionCounts {
        posts: post_actions.len(),
        likes: likes.len(),
        groups: groups.as_array().map(|a| a.len()).unwrap_or(0),
        messages: message_count,
    };
    let has_ip =
        profile.get("ip_address").is_some() || profile.get("registration_ip_address").is_some();
    let manifest = build_manifest(
        &subject,
        &discourse.name,
        &generated_at,
        &counts,
        include_messages,
        has_ip,
    );
    write_json(&dir.join("manifest.json"), &manifest)?;
    write_markdown_private(
        &dir.join("README.md"),
        &render_readme(&subject, &discourse.name, &generated_at, include_messages),
        true,
    )?;

    println!("SAR bundle written to {}", dir.display());
    println!(
        "  {} posts, {} likes, {} group(s){}",
        counts.posts,
        counts.likes,
        counts.groups,
        if include_messages {
            format!(", {} message thread(s)", counts.messages)
        } else {
            String::new()
        }
    );
    println!(
        "This bundle contains personal data. Review it (see README.md), transmit \
         it securely, and delete it once the request is fulfilled."
    );
    Ok(())
}

fn resolve_subject(client: &DiscourseClient, user: &str) -> Result<Subject> {
    if user.contains('@') {
        let matches = client.admin_search_users(user)?;
        let found = matches
            .into_iter()
            .find(|u| {
                u.email
                    .as_deref()
                    .map(|e| e.eq_ignore_ascii_case(user))
                    .unwrap_or(false)
            })
            .ok_or_else(|| not_found("user with email", user))?;
        Ok(Subject {
            user_id: found.id,
            username: found.username,
            email: found.email,
        })
    } else {
        let detail = client.fetch_user_detail(user)?;
        Ok(Subject {
            user_id: detail.id,
            username: detail.username,
            email: detail.email,
        })
    }
}

/// Page through `fetch_user_actions` until a short/empty page is returned.
/// Capped so a misbehaving endpoint cannot loop forever.
fn collect_all_actions(
    client: &DiscourseClient,
    username: &str,
    filters: &[u32],
) -> Result<Vec<UserAction>> {
    const PAGE_HINT: usize = 10; // Discourse returns ~10 per page.
    const MAX_ITEMS: usize = 100_000;
    let mut all = Vec::new();
    let mut offset = 0u32;
    loop {
        let page = client.fetch_user_actions(username, filters, offset)?;
        let n = page.len();
        if n == 0 {
            break;
        }
        all.extend(page);
        offset += n as u32;
        if n < PAGE_HINT || all.len() >= MAX_ITEMS {
            break;
        }
    }
    Ok(all)
}

fn collect_messages(client: &DiscourseClient, username: &str, dir: &Path) -> Result<usize> {
    let msg_dir = dir.join("messages");
    ensure_private_dir(&msg_dir)?;
    let mut threads = client.list_private_messages(username, "inbox")?;
    threads.extend(client.list_private_messages(username, "sent")?);

    let mut seen = HashSet::new();
    let mut count = 0;
    for pm in threads {
        if !seen.insert(pm.id) {
            continue;
        }
        let topic = client.fetch_topic(pm.id, true)?;
        let stem = pm
            .slug
            .as_deref()
            .map(slugify)
            .unwrap_or_else(|| "message".to_string());
        write_markdown_private(
            &msg_dir.join(format!("{}-{}.md", stem, pm.id)),
            &render_pm_md(&pm, &topic),
            true,
        )?;
        count += 1;
    }
    Ok(count)
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    atomic_write_private(path, text, true)
}

fn date_part(iso: &str) -> String {
    iso.split('T').next().unwrap_or(iso).to_string()
}

fn post_url(base: &str, action: &UserAction) -> String {
    let slug = action.slug.as_deref().unwrap_or("topic");
    match action.post_number {
        Some(n) if n > 1 => format!("{}/t/{}/{}/{}", base, slug, action.topic_id, n),
        _ => format!("{}/t/{}/{}", base, slug, action.topic_id),
    }
}

fn action_to_json(action: &UserAction) -> Value {
    json!({
        "topic_id": action.topic_id,
        "post_id": action.post_id,
        "title": action.title,
        "created_at": action.created_at,
        "excerpt": action.excerpt,
    })
}

fn render_post_md(action: &UserAction, raw: &str, base: &str) -> String {
    format!(
        "# {}\n\n- URL: {}\n- Posted: {}\n\n---\n\n{}\n",
        action.title.as_deref().unwrap_or("(untitled)"),
        post_url(base, action),
        action.created_at,
        raw.trim_end()
    )
}

fn render_pm_md(pm: &PmTopicSummary, topic: &TopicResponse) -> String {
    let mut out = String::new();
    out.push_str(
        "> REVIEW REQUIRED: this private-message thread contains other people's \
         personal data. Review for third-party information and redact before \
         disclosure.\n\n",
    );
    out.push_str(&format!(
        "# {}\n\n",
        pm.title.as_deref().unwrap_or("(no subject)")
    ));
    for post in &topic.post_stream.posts {
        let who = post.username.as_deref().unwrap_or("(unknown)");
        let when = post.created_at.as_deref().unwrap_or("(no date)");
        let body = post.raw.as_deref().unwrap_or("").trim_end();
        out.push_str(&format!("## {} · {}\n\n{}\n\n---\n\n", who, when, body));
    }
    out
}

fn build_manifest(
    subject: &Subject,
    forum: &str,
    generated_at: &str,
    counts: &SectionCounts,
    include_messages: bool,
    has_ip: bool,
) -> Value {
    let mut review_required: Vec<String> = Vec::new();
    if has_ip {
        review_required
            .push("profile.json includes IP addresses; confirm these should be released".into());
    }
    if include_messages {
        review_required.push(
            "messages/ contains third-party personal data; review and redact before disclosure"
                .into(),
        );
    }
    json!({
        "subject": {
            "username": subject.username,
            "user_id": subject.user_id,
            "email": subject.email,
        },
        "forum": forum,
        "generated_at": generated_at,
        "messages_included": include_messages,
        "sections": {
            "posts": counts.posts,
            "likes_given": counts.likes,
            "groups": counts.groups,
            "messages": counts.messages,
        },
        "review_required": review_required,
    })
}

/// The human-facing cover sheet. Explains what the bundle is, lists the
/// controller's remaining steps, and scaffolds the Article 15 supplementary
/// information for them to complete.
fn render_readme(
    subject: &Subject,
    forum: &str,
    generated_at: &str,
    include_messages: bool,
) -> String {
    let email = subject.email.as_deref().unwrap_or("(not recorded)");
    let messages_line = if include_messages {
        "- `messages/` - private messages (**contains third-party data - review and redact**)\n"
    } else {
        "- (private messages were NOT collected; re-run with `--messages` if the request requires them)\n"
    };
    let messages_checklist = if include_messages {
        "- [ ] Review `messages/` for third-party personal data and redact.\n"
    } else {
        ""
    };
    format!(
        "# Subject Access Request - {username}\n\
\n\
Personal data held about **{username}** ({email}) on the **{forum}** Discourse \
forum, generated by `dsc sar` at {generated_at}.\n\
\n\
This package was assembled automatically from the Discourse admin API. It is a \
**data-gathering aid, not a finished SAR response** - the steps below are the \
data controller's responsibility and have not been done for you.\n\
\n\
## What's included\n\
\n\
- `profile.json` - account and profile data (PII), including IP addresses and emails.\n\
- `posts/` and `posts.json` - every post the subject authored, full text.\n\
- `activity.json` - likes the subject gave.\n\
- `groups.json` - group memberships.\n\
{messages_line}\
- `manifest.json` - machine-readable index, counts, and items flagged for review.\n\
\n\
## Controller checklist (before sending)\n\
\n\
- [ ] Verify the requester is the data subject (or is properly authorised).\n\
- [ ] Confirm IP addresses and technical data in `profile.json` should be released.\n\
{messages_checklist}\
- [ ] Complete the Article 15 supplementary information below.\n\
- [ ] Apply any exemptions (others' rights, legal privilege, etc.).\n\
- [ ] Send via a secure channel within **one calendar month** of the request.\n\
\n\
## Article 15 supplementary information (to complete)\n\
\n\
Under UK/EU GDPR Article 15 the response must also state, in addition to the \
data itself, the following - none of which lives in Discourse, so fill them in \
from your processing records:\n\
\n\
- **Purposes of processing:** [controller to complete]\n\
- **Categories of personal data:** account profile, posts, activity{messages_cat}.\n\
- **Recipients / categories of recipient:** [controller to complete]\n\
- **Retention period (or the criteria for it):** [controller to complete]\n\
- **Source of the data** (if not collected from the subject): [controller to complete]\n\
- **Existence of automated decision-making / profiling:** [controller to complete]\n\
- **The subject's rights** (rectification, erasure, restriction, objection, complaint to the supervisory authority): [controller to complete]\n\
\n\
---\n\
\n\
This bundle is personal data. Store and transmit it securely, and delete it \
once the request has been fulfilled.\n",
        username = subject.username,
        email = email,
        forum = forum,
        generated_at = generated_at,
        messages_line = messages_line,
        messages_checklist = messages_checklist,
        messages_cat = if include_messages {
            ", private messages"
        } else {
            ""
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject() -> Subject {
        Subject {
            user_id: 412,
            username: "jane-doe".to_string(),
            email: Some("jane@example.com".to_string()),
        }
    }

    #[test]
    fn date_part_takes_the_date() {
        assert_eq!(date_part("2026-06-23T09:00:00Z"), "2026-06-23");
        assert_eq!(date_part("2026-06-23"), "2026-06-23");
    }

    #[test]
    fn manifest_flags_ip_and_messages_when_present() {
        let counts = SectionCounts {
            posts: 84,
            likes: 12,
            groups: 3,
            messages: 7,
        };
        let m = build_manifest(
            &subject(),
            "rcpch",
            "2026-06-23T09:00:00Z",
            &counts,
            true,
            true,
        );
        assert_eq!(m["subject"]["user_id"], 412);
        assert_eq!(m["sections"]["posts"], 84);
        assert_eq!(m["messages_included"], true);
        let review = m["review_required"].as_array().unwrap();
        assert_eq!(review.len(), 2, "expected IP + messages flags");
    }

    #[test]
    fn manifest_has_no_review_flags_when_clean() {
        let counts = SectionCounts {
            posts: 1,
            likes: 0,
            groups: 0,
            messages: 0,
        };
        let m = build_manifest(&subject(), "rcpch", "t", &counts, false, false);
        assert!(m["review_required"].as_array().unwrap().is_empty());
        assert_eq!(m["messages_included"], false);
    }

    #[test]
    fn readme_includes_checklist_and_article_15() {
        let out = render_readme(&subject(), "rcpch", "2026-06-23T09:00:00Z", false);
        assert!(out.contains("Subject Access Request - jane-doe"));
        assert!(out.contains("Verify the requester is the data subject"));
        assert!(out.contains("Article 15 supplementary information"));
        assert!(out.contains("one calendar month"));
        // Messages excluded -> note the opt-in, no message-review checklist line.
        assert!(out.contains("--messages"));
        assert!(!out.contains("Review `messages/`"));
    }

    #[test]
    fn readme_adds_message_review_when_included() {
        let out = render_readme(&subject(), "rcpch", "t", true);
        assert!(out.contains("Review `messages/`"));
        assert!(out.contains("third-party data"));
    }

    #[test]
    fn post_url_includes_post_number_after_first() {
        let action = UserAction {
            action_type: 5,
            created_at: "2026-01-01".into(),
            title: Some("Hi".into()),
            slug: Some("hi-there".into()),
            topic_id: 50,
            post_id: Some(99),
            post_number: Some(3),
            username: Some("jane-doe".into()),
            excerpt: None,
        };
        assert_eq!(
            post_url("https://forum.example.com", &action),
            "https://forum.example.com/t/hi-there/50/3"
        );
    }
}

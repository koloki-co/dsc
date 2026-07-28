# `dsc notification forensics` - inspect topic/category/tag notification levels

Spec for read-only inspection of `TopicUser`, `CategoryUser`, `TagUser`, and `SkippedEmailLog` records. Goal: answer "who is watching this topic and why did they get an email" without server or Data Explorer access. Driver: a real notification cascade incident on a production forum where the admin could not diagnose why specific users received emails for a dormant topic that was suddenly replied to.

## Motivation

A welcome topic created in 2024 mentioned a 17-member group. In 2026, someone replied to the dormant topic, triggering a cascade of email notifications. Members who received unexpected emails replied to complain, and each reply auto-watched the replier and notified all existing watchers, compounding the cascade. The admin needed to answer: who was watching this topic, why, and how did the cascade start?

Today `dsc` can read the staff action log (`dsc log staff`) and the API user's own notifications (`dsc notification list`), but it cannot inspect per-user topic/category/tag notification levels or email send/skip logs. There is no Discourse admin API endpoint for any of these tables. The only access paths are Data Explorer queries or the Rails console, both of which require server access.

## Current state (as of 2026-07-28)

`dsc` 0.12.1 has:

- `dsc log staff` - staff action log (UserHistory)
- `dsc notification list` - the API user's own notifications only
- `dsc user info` - includes the user's group memberships but not their topic/category/tag notification levels
- `dsc group info` - does not surface `group_category_notification_defaults` or `group_tag_notification_defaults` even though the Discourse API returns them in the group show serializer

`dsc` cannot:

- List which users are watching/tracking/muting a specific topic
- List which users are watching a specific category
- List which users are watching a specific tag
- Show whether a group has category/tag notification defaults set
- Show which emails were sent vs skipped and why (SkippedEmailLog)

## Proposed CLI surface

```text
dsc notify who <discourse> <topic-id>                    # list TopicUser records for a topic
dsc notify who <discourse> --category <category-id>     # list CategoryUser records for a category
dsc notify who <discourse> --tag <tag-name>             # list TagUser records for a tag
dsc notify who <discourse> --user <username>            # list all notification-level records for a user
dsc notify skipped <discourse> [--topic <topic-id>]     # list SkippedEmailLog entries
                     [--user <username>] [--since <dur>] [--limit <n>]
                     [--format text|json|yaml]
dsc group info <discourse> <group-id> --with-defaults    # include group_category/tag_notification_defaults
```

### `dsc notify who`

Lists notification-level records (`TopicUser`, `CategoryUser`, `TagUser`) in a unified output:

- `dsc notify who <discourse> <topic-id>` - all `TopicUser` rows for the topic, showing `user_id`, `username`, `notification_level` (0= muted, 1= regular, 2= tracking, 3= watching, 4= watching_first_post if applicable), `notifications_reason_id` (translated to symbol), `last_read_post_number`, `first_visited_at`, `last_visited_at`
- `dsc notify who <discourse> --category <category-id>` - all `CategoryUser` rows, showing `user_id`, `username`, `notification_level`, reason if available
- `dsc notify who <discourse> --tag <tag-name>` - all `TagUser` rows for the tag, showing `user_id`, `username`, `notification_level`
- `dsc notify who <discourse> --user <username>` - all three tables combined for one user, showing topic_id/category_id/tag_name, level, reason

Output includes the human-readable `notification_level` name and, for `TopicUser`, the `notifications_reason_id` translated to its symbol (created_topic, user_changed, user_interacted, created_post, auto_watch, auto_watch_category, auto_mute_category, auto_track_category, plugin_changed, auto_watch_tag, auto_mute_tag, auto_track_tag).

### `dsc notify skipped`

Lists `SkippedEmailLog` entries, showing `user_id`, `username`, `email_type`, `post_id`, `reason_type` (translated to symbol), `created_at`. Supports filtering by topic (via post_id join), user, and time range. This is the "why didn't this person get an email" or "which emails were sent vs skipped" forensic view.

### `dsc group info --with-defaults`

Adds `group_category_notifications` and `group_tag_notifications` to the existing `dsc group info` output. These fields are already returned by the Discourse API (`GroupShowSerializer#group_category_notifications`) but `dsc` currently does not surface them. The `--with-defaults` flag is additive so existing consumers are unaffected.

## Reference: API calls observed in the field

Discourse has **no admin API endpoint** for `TopicUser`, `CategoryUser`, `TagUser`, or `SkippedEmailLog`. These are internal Rails models with no admin controller route. The only access paths are:

1. **Data Explorer plugin queries** - direct SQL against the tables, requires the plugin to be installed and a staff account
2. **Rails console** - `TopicUser.where(topic_id: 9898, notification_level: 3).joins(:user).pluck(:username)` etc., requires server SSH access
3. **The group show serializer** - `GET /groups/<id>.json` does return `group_category_notifications` and `group_tag_notifications` hashes in the `group` object, but `dsc group info` does not currently surface them

Tested against Discourse version confirmed by `dsc version rcgp` (not run here, but the forum is on a recent stable release as of July 2026).

## Phases

### Phase 1 - blocking

- [ ] `dsc group info --with-defaults` - surfaces already-available API data, no new endpoint needed. Lowest effort, immediately useful for the incident investigation.
- [ ] `dsc notify who <discourse> <topic-id>` - the most urgently needed forensic query. Requires either a Data Explorer query wrapper or documenting that this needs server access. If Data Explorer is the only path, `dsc` could detect whether the plugin is installed and run a pre-packaged query via the Data Explorer admin API (`POST /admin/plugins/explorer/queries/<id>/run`).

### Phase 2 - iteration ergonomics

- [ ] `dsc notify who --category <id>` and `dsc notify who --tag <name>` - same mechanism as topic
- [ ] `dsc notify who --user <username>` - unified view across all three tables for one user
- [ ] `dsc notify skipped` - SkippedEmailLog inspection

### Phase 3 - nice to have

- [ ] `dsc notify why <discourse> <topic-id> <username>` - synthesise a human-readable explanation of why a user received (or did not receive) a notification for a specific topic, correlating TopicUser, CategoryUser, TagUser, group memberships, group_category_notification_defaults, user_option email_level, and SkippedEmailLog. This is the "give me a straight answer" command that Discourse itself does not provide.

## Backward compatibility

- `dsc group info` output gains two optional fields when `--with-defaults` is passed; without the flag, output is unchanged.
- `dsc notify` is a new top-level command; no existing commands are affected.
- If the Data Explorer plugin is not installed, `dsc notify who` should fail with a clear message explaining the prerequisite, not silently return empty results.

## Out of scope

- Modifying any notification level (that would be a separate `dsc notify set` command if demanded)
- Real-time notification monitoring or streaming
- Per-user notification history beyond what `dsc notification list` already provides for the API user
- Chat notifications (separate subsystem)
- Plugin-specific notification types
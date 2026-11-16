# dsc log

View admin audit trails.

## dsc log staff

```text
dsc log staff <discourse> [--action <name>] [--acting-user <username>] [--target-user <username>] [--user <username>] [--subject <text>] [--since <dur>] [--until <timestamp>] [--limit <n>] [--include-context] [--format text|json|yaml]
```

Lists entries from the staff action log — the same audit trail behind `/admin/logs/staff-action-logs` in the web UI: setting changes, suspensions, deletions, admin grants, and the rest of Discourse's `UserHistory` records.

All filters are optional and combine (AND):

- `--action` — built-in Discourse action name, e.g. `change_site_setting`, `suspend_user`, `delete_post`, `grant_admin`. Unknown and custom/plugin names are rejected rather than risking an unfiltered query.
- `--acting-user` — the staff member who performed the action.
- `--target-user` — the user the action was performed on.
- `--user` — the user was the actor **or** the target. `dsc` makes two requests (one `acting_user`, one `target_user`, each with every other filter and the same `--limit`), removes duplicates by entry ID (self-actions appear in both), and prints the newest `--limit` rows in newest-first order (ties broken by entry ID). It cannot be combined with `--acting-user` or `--target-user`, which keep their AND meaning. The older-entries warning appears if either stream filled its limit. This still only covers events Discourse records with this user as actor or target; it is not a complete account history.
- `--subject` — exact match on the subject field (often a setting name).
- `--since` — relative duration (`7d`, `24h`) or an ISO-8601 date/timestamp; sent to Discourse as a precise UTC `start_date` timestamp.
- `--until` — inclusive upper bound, as a full RFC 3339 timestamp (e.g. `2026-10-31T23:59:59Z`); sent as `end_date`. Bare dates and relative durations are rejected because they are ambiguous as an end point, and a window whose `--since` is later than `--until` is refused before any request is made.
- `--limit` — newest-first rows to fetch, default 50; must be from 1 through 200. If exactly this many are returned, `dsc` warns on stderr that older matching entries may exist.

`--include-context` is an output option, not a filter. It adds the source context and linkage metadata Discourse records with each entry: `context`, `topic_id`, `post_id`, `category_id`, `reviewable_id`, the numeric `action` code, `custom_type` and `expires_at`. In JSON/YAML every one of these keys is present on every row, as `null` when Discourse supplied nothing — Discourse also returns `null` when it redacts `context`/`expires_at` for the viewer, and the two cases cannot be told apart. In text output, available IDs are appended as `[topic=… post=…]` and any context follows on indented `context:` lines. Email addresses and IP addresses are never emitted, with or without this flag. Without it, output is exactly as before.

```bash
# Recent activity, most recent first
dsc log staff myforum

# Who changed a setting, and to what
dsc log staff myforum --action change_site_setting --subject login_required

# One admin's actions in the last week, as JSON
dsc log staff myforum --acting-user alice --since 7d -f json

# A bounded historical window (both ends inclusive)
dsc log staff myforum --target-user bob --since 2026-10-01 --until 2026-10-31T23:59:59Z

# Everything bob did or had done to them in the last 30 days
dsc log staff myforum --user bob --since 30d -f json

# Post deletions with the topic/post IDs and recorded context
dsc log staff myforum --action delete_post --include-context
```

## Notes

- Requires an admin API key — this is an admin-only endpoint.
- Read-only: `dsc log staff` never writes anything (`--user` makes two GET requests), and ignores `--dry-run`.
- This first version reads one page only; it does not yet paginate through the full audit history or support custom/plugin action types.

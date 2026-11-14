# dsc log

View admin audit trails.

## dsc log staff

```text
dsc log staff <discourse> [--action <name>] [--acting-user <username>] [--target-user <username>] [--subject <text>] [--since <dur>] [--until <timestamp>] [--limit <n>] [--format text|json|yaml]
```

Lists entries from the staff action log — the same audit trail behind `/admin/logs/staff-action-logs` in the web UI: setting changes, suspensions, deletions, admin grants, and the rest of Discourse's `UserHistory` records.

All filters are optional and combine (AND):

- `--action` — built-in Discourse action name, e.g. `change_site_setting`, `suspend_user`, `delete_post`, `grant_admin`. Unknown and custom/plugin names are rejected rather than risking an unfiltered query.
- `--acting-user` — the staff member who performed the action.
- `--target-user` — the user the action was performed on.
- `--subject` — exact match on the subject field (often a setting name).
- `--since` — relative duration (`7d`, `24h`) or an ISO-8601 date/timestamp; sent to Discourse as a precise UTC `start_date` timestamp.
- `--until` — inclusive upper bound, as a full RFC 3339 timestamp (e.g. `2026-10-31T23:59:59Z`); sent as `end_date`. Bare dates and relative durations are rejected because they are ambiguous as an end point, and a window whose `--since` is later than `--until` is refused before any request is made.
- `--limit` — newest-first rows to fetch, default 50; must be from 1 through 200. If exactly this many are returned, `dsc` warns on stderr that older matching entries may exist.

```bash
# Recent activity, most recent first
dsc log staff myforum

# Who changed a setting, and to what
dsc log staff myforum --action change_site_setting --subject login_required

# One admin's actions in the last week, as JSON
dsc log staff myforum --acting-user alice --since 7d -f json

# A bounded historical window (both ends inclusive)
dsc log staff myforum --target-user bob --since 2026-10-01 --until 2026-10-31T23:59:59Z
```

## Notes

- Requires an admin API key — this is an admin-only endpoint.
- Read-only: `dsc log staff` never writes anything, and ignores `--dry-run`.
- This first version reads one page only; it does not yet paginate through the full audit history or support custom/plugin action types.

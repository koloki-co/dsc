# `dsc log staff` - staff action log access

> **Status: implemented.** Read-only access to Discourse's staff action log
> (`UserHistory`) - the audit trail behind `/admin/logs/staff-action-logs` in
> the web UI. Roadmap item R15.

Driver: "who changed this setting, and when" is a recurring question across a
multi-forum fleet, and today the only answer is clicking through the admin UI
on each forum individually. `dsc log staff` makes that a one-liner, and lets
the answer be piped, diffed, or scripted like the rest of `dsc`.

## Command surface

```
dsc log staff <discourse> [--action <name>] [--acting-user <username>]
                           [--target-user <username>] [--subject <text>]
                           [--since <dur>] [--until <timestamp>] [--limit <n>] [--format text|json|yaml]
```

All filters are optional and AND together, mirroring Discourse's own
`UserHistory.staff_filters`:

- `--action` - supported built-in action name, e.g. `change_site_setting`,
  `suspend_user`, `delete_post`, `grant_admin`. `dsc` validates this against
  the core staff-action names it knows before requesting logs. This is a safety
  guard: Discourse silently drops the action predicate for an unrecognised
  `action_name`, which would otherwise return unrelated newest records.
  Custom/plugin action types are deliberately not supported in this phase.
- `--acting-user` - the staff member who performed the action.
- `--target-user` - the user the action was performed on (absent for
  non-user-targeted actions, e.g. a setting change).
- `--subject` - exact match on the subject field (often a setting name). The
  Discourse endpoint uses `subject = ?`; a substring-search option needs a
  paginated client-side design and is not claimed here.
- `--since` - relative duration (`7d`, `24h`) or an ISO-8601 date/timestamp,
  parsed with the same `parse_since_cutoff` used by `analytics` and
  `update log`, then sent as a precise UTC RFC 3339 `start_date` timestamp.
- `--until` - inclusive upper bound (R83 slice, implemented 2026-11-14). Only
  a full RFC 3339 timestamp is accepted; it is normalised to UTC and sent as
  `end_date`. Discourse applies `created_at <= end_date`
  (`UserHistory.staff_action_records`, and the controller slices
  `:start_date, :end_date` from params - confirmed against `main` source on
  2026-11-14). A bare date is rejected because it would mean midnight at the
  start of that day and silently exclude the rest of it; a relative duration
  is rejected as ambiguous. `--since` is resolved once, and a reversed window
  (`--since` later than `--until`) is refused before any request. Equal
  bounds are allowed.
- `--limit` - newest-first rows to fetch, default 50, validated to `1..=200`
  rather than silently clamped. When the response reaches the requested limit,
  `dsc` writes a stderr warning that older matching entries may exist.

This first cut makes one request only; it does not paginate through the full
history. A future pagination spec must capture the live endpoint's paging
contract before adding `--all` or client-side substring filtering. Fleet-wide
aggregation is `R20`'s job, not this command's.

## Output

- **text** (default): one line per entry - timestamp, action name, acting
  user, target user (`-` when absent), subject.
- **json** / **yaml**: the full entry list, one object per row: `id`,
  `action_name`, `acting_user` (`{id, username}` or absent), `target_user`
  (ditto), `subject`, `details`, `previous_value`, `new_value`, `created_at`.
  `previous_value` and `new_value` preserve either scalar strings or structured
  JSON values as returned by Discourse. Field set matches Discourse's
  `UserHistorySerializer` minus a few
  presentation-only fields (`ip_address`, `email`, `context`, `custom_type`)
  not needed for the audit-trail use case.

## Notes

- Requires an admin API key - `/admin/logs/*` is an admin-only route.
- Read-only: ignores the global `--dry-run` flag like other inspection
  commands (`user activity`, `search`, `analytics`).
- Not yet covered: custom/plugin action types, `--action-id` (the numeric
  code, vs. the validated symbolic `--action` name), pagination, substring
  subject search, and cross-forum aggregation - add on demand.

## Investigation follow-up (R83)

> **Status: proposed, not implemented.** Field driver: a November 2026 account-history investigation using `dsc 0.24.0` against Discourse `2026.11.0-latest`. The existing actor/target filters successfully established role changes and account/post deletions, but required separate requests and local date filtering. The current serializer intentionally drops metadata that is useful for incident evidence, not merely presentation.

Tentative additive surface:

```text
dsc log staff <forum> --user <username> --since <timestamp> --until <timestamp>
    [--include-context] [--include-email] [--include-ip] --format json
```

- `--user` means actor **or** target. Fetch both existing filtered streams, deduplicate by log-entry ID and order deterministically. Reject combination with `--acting-user`/`--target-user` rather than silently changing their established AND semantics. Self-initiated email events may have only an actor; a target-only search must not masquerade as complete account history.
- [x] `--until` supplies a bounded historical end date. **Implemented 2026-11-14** (see Command surface above): relative `--since` resolved once, RFC 3339-only `--until`, reversed windows rejected with zero requests, inclusivity documented. Covered by unit tests and a mock-server test asserting the exact query. Still unverified against a live forum. Note from source: the controller's `load_more_staff_action_logs` URL permits only `staff_filters`, so it drops `start_date`/`end_date` - confirming the pagination rule below.
- Design bounded pagination with an explicit completeness state. Preserve the existing `--limit` contract; verify the live paging/total-count contract before introducing a larger export option. Preserve date/action/user filters explicitly on every page: do not blindly follow a `load_more` URL that might omit them. R65 owns any durable bundle envelope.
- `--include-context` preserves available `context`, post/topic/category/reviewable IDs, action IDs and custom-type metadata in an opt-in expanded result. Default output remains backward compatible. In this investigation the omitted context could have helped distinguish a user-deletion cascade from individual post deletion; timestamps and source inspection were used instead. Never manufacture a causal link when the source supplies none.
- Email and IP fields require separate explicit opt-ins and ordinary permission checks. These are personal-data fields, not presentation-only fields. No token or credential output. Preserve server redaction and distinguish absent/withheld/unknown data where the API permits.

### Evidence and validation

Observed through existing CLI requests, with account parameters redacted:

```text
GET /admin/logs/staff_action_logs.json?acting_user={username}&limit=200
GET /admin/logs/staff_action_logs.json?target_user={username}&limit=200
GET /admin/logs/staff_action_logs.json?action_name=grant_moderation&start_date={timestamp}&limit=200
```

The results were below the requested limit; pagination did not block this particular investigation. End-date/paging support and additional serialized fields are source-derived contracts requiring live test-account verification. Current `StaffActionLog` preserves `details` and old/new values but omits `context`, linkage IDs and response completeness metadata.

- [ ] Capture sanitized live evidence for upper date bounds, paging, total counts, optional fields and redaction without creating production moderation events.
- [ ] Test actor-or-target union, self-actions with null targets, duplicate IDs, stable ordering, date-window boundaries (query construction done; live boundary behaviour pending), page/filter retention, budget exhaustion and no false completeness claims.
- [ ] Test default output compatibility, opt-in metadata, withheld sensitive fields and absence of mutations.

No automation attribution is inferred from log timing. Account deletion can emit many associated post-deletion rows automatically, and an actor username alone cannot distinguish a browser click, API client or account takeover.

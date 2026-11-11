# Targeted account inspection

> **Status: proposed, R82.** No new commands have been implemented. Authentication-history access is a discovery question, not a promised API capability.

## Motivation

A November 2026 investigation on a managed, legacy-imported forum needed to establish whether an account's email address had changed recently, when it gained moderator privileges, and whether its recent activity established account compromise or automation. `dsc` could export the discussion and staff actions, but `user info` returned only a reduced public profile. Running a whole SAR solely to inspect account metadata would collect unrelated authored content and activity. The decisive historical email observation came from a narrowly filtered, read-only inspection of a retained local backup, not a live API response. Client identities, addresses and private discussion remain in the client's administrative workspace, not this public spec.

## Current state, 11 November 2026

- Installed tool: `dsc 0.24.0`; investigated Discourse: `2026.11.0-latest`.
- `dsc user info <forum> <username> --format json` uses `GET /u/{username}.json` and deserializes into the reduced `UserDetail` model. In the observed result, `email` was null; admin account metadata and authentication/session history were not output. Null is not evidence that the account has no email address.
- The client already implements `fetch_admin_user_detail` for `GET /admin/users/{id}.json`, used by `dsc sar`. Expose a targeted inspection path instead of a second client or a whole SAR workaround.
- Source inspection shows admin detail can supply reading/visit counters, MFA state and API-key count as well as ordinary account metadata. Some sensitive fields are permission- or option-gated; a bare admin-detail fetch must not be advertised as retrieving every email.
- First-login time, historical IP/browser changes, completed email changes and password-reset history were not retrieved. Establish supported API access and retention before proposing commands for them. Do not assume that current sessions constitute a complete login history.
- [Staff-log investigation improvements](staff-action-log.md#investigation-follow-up-r83) are R83, not a second account-history implementation.

## Proposed CLI surface

Tentative additive options:

```text
dsc user info <forum> <username> --admin [--include-email] [--include-ip] --format json
```

- Existing `user info` behavior and output remain unchanged without `--admin`.
- Resolve one exact account and reuse the existing admin-detail client. Return an allowlisted inspection view: account flags, timestamps actually supplied by the server, reading/visit counters, MFA state and API-key count where available. Preserve omitted/unknown states rather than coercing them to false or zero.
- Email addresses and IP addresses require separate explicit opt-ins. Discover whether email reads need a distinct endpoint and produce a `check_email` audit event; document those side effects and the `--dry-run` policy before implementation. Do not assume local output filtering makes the remote request side-effect-free.
- Never output session cookies, authentication tokens, API secrets, recovery codes or password material. Do not pass through an opaque raw admin payload.
- No posts, likes, messages, Chat content, member lists or backup downloads are collected by this inspection command.
- Keep source endpoint, retrieval timestamp and server-version provenance available for later R65 evidence collection without inventing a separate evidence-bundle format.

## Reference: API calls observed in the field

Paths are parameterized for privacy. These operations were performed through existing `dsc` commands:

```text
GET /u/{username}.json
GET /admin/logs/staff_action_logs.json?target_user={username}&limit=200
GET /admin/logs/staff_action_logs.json?acting_user={username}&limit=200
```

The public-profile CLI result included account identity, current roles, creation/last-seen timestamps and groups, with `email: null`. Actor- and target-filtered staff-log requests returned fewer than 200 rows. They established role grants/revocation and deletions, not the operator's identity or a complete email/login history.

`GET /admin/users/{id}.json` is an existing client capability, but its live response was not captured during this investigation. Verify its current response, optional fields, permission failures and any audited reads before implementation. No raw admin API, database or SSH workaround was used.

## Phases and completion criteria

### Phase 1 - targeted admin view

- [ ] Verify the minimal admin-detail and explicit email/IP read contracts on an authorized test account, retaining only sanitized evidence.
- [ ] Add the targeted opt-in admin view, preserving the old command contract.
- [ ] Test public/admin selection, exact account resolution, missing permissions, omitted/null fields, explicit sensitive-field inclusion, and exclusion of all credential material using mock responses.
- [ ] Document actual read side effects and ensure `--dry-run` cannot accidentally trigger an audited sensitive read.

### Phase 2 - authentication and email-history discovery

- [ ] Determine whether supported admin APIs expose first successful login, retained authentication-event metadata, completed email changes and password-reset events for another account. Record unavailable surfaces explicitly; do not infer historical events from current profile timestamps.
- [ ] If a needed observation is available only through Data Explorer, prefer a separately reviewed, administrator-installed saved query with an exact account parameter and minimal columns, using the existing `explorer show/run` contract. Do not add arbitrary SQL execution or silently install queries.
- [ ] Only propose a dedicated session/history command after verifying source coverage, permissions and retention. Metadata must exclude live token values and distinguish browser sessions, API activity and unknown origin only when the source supports that distinction.

## Historical evidence and limits

An imported email record's creation time dates the Discourse record, not necessarily the user's original address choice. A dated backup establishes presence at that snapshot, not uninterrupted ownership. Email-change requests may be cleaned up after one month. An absent event is not proof that an event never happened; an acting account does not identify its human or software operator.

Local SQL-dump parsing remains outside this implementation. A reusable backup-inspection command needs a separate admission/design decision covering supported formats, bounded streaming, secret exclusion, path/archive safety and provenance. Do not restore a database or download backups implicitly to fill an API gap.

## Backward compatibility and out of scope

This is an additive inspection capability, not a replacement for SAR exports. No account changes, credential testing, impersonation, automatic sanctions, bot scores or identity verdicts. It does not promise recovery of pruned logs, original source-forum role records or any datum Discourse does not expose. Evidence-bundle work stays under R65.

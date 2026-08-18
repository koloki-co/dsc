# `dsc sar` - one-shot Subject Access Request export

> **Status: Phase 1 implemented (unreleased).** `dsc sar <discourse> <user>`
> writes the bundle (profile/PII, authored posts, likes, groups, README cover
> sheet + manifest), with private messages opt-in via `--messages` and a
> working `--dry-run`. Phase 2 (`--zip`, combined document, staff notes) is
> on-demand; multi-forum fan-out is out of scope by decision.

Spec for a new `dsc sar` command that gathers everything a Discourse holds
about one person into a single, organised, portable bundle suitable for
answering a **Subject Access Request** (SAR / DSAR under UK GDPR Art. 15 and
the equivalent EU GDPR right). Goal: turn the laborious "collect all of a
user's personal data by hand" task into one command. Driver: the author runs
forums in NHS/medical-adjacent contexts (RCPCH, restorativejustculture.org)
where SARs are a real statutory obligation and Discourse has no single
"export everything about this person" action.

## Compliance scope - read this first

A SAR response is a **legal** deliverable, and most of what makes it
*compliant* is human/legal judgement, not data plumbing. `dsc sar` automates
the part that is pure labour - finding and packaging every piece of personal
data the Discourse **admin API** exposes about the subject - and scaffolds the
rest. It deliberately does **not** try to make the whole response automatically
"legally compliant", because these steps are the controller's responsibility
and cannot be safely automated:

- **Identity verification** of the requester (is this really the data subject?).
- **Third-party data**: a private message between A and B contains B's personal
  data too; deciding what to redact is a judgement call.
- **Exemptions** (legal privilege, crime prevention, others' rights, etc.).
- **Article 15 supplementary information** (purposes of processing, retention,
  recipients, source, automated decision-making) - this is organisational
  policy, not data in Discourse.
- **Timeliness**: the statutory deadline is one calendar month from receipt.

So the honest framing is: **`dsc sar` produces a comprehensive, structured data
package for a SAR and a checklist of the human steps that remain.** The cover
sheet and `manifest.json` make those steps explicit and flag what needs review
(see below). The tool must never imply the bundle is ready to send unreviewed.

## Current state (as of 2026-06-23)

`dsc` already exposes most of the underlying data piecemeal:

- `dsc user info <discourse> <username>` -> `fetch_user_detail` (public
  `/u/{username}.json`; **limited** PII - no IP history, partial email).
- `dsc user ls -f json` -> `admin_list_users` (`/admin/users/list/*.json`,
  carries emails).
- `dsc user activity` / `fetch_user_actions` -> posts, likes, actions.
- `dsc pm list <discourse> <username>` -> their private-message threads.
- `dsc user groups list` -> group memberships.

What is missing: (1) the **admin** user-detail endpoint with the full PII
surface (`/admin/users/{id}.json` - registration IP, last IP, all emails,
associated accounts, custom fields); (2) a command that **orchestrates** all of
the above into one coherent, reviewable bundle. Today a SAR is assembled by
hand from several `dsc` calls plus raw `curl`.

## Proposed CLI surface

```text
dsc sar <DISCOURSE> <USER> [OPTIONS]
```

- `<USER>` is a username **or** an email address (resolved to the account).
- Options:
  - `--output <DIR>` - destination directory (default
    `sar-<username>-<YYYY-MM-DD>/` in the cwd).
  - `--messages` - **opt-in**: also collect the subject's private messages.
    Off by default because PMs necessarily contain other people's personal
    data; a full SAR may require them, but only after the controller makes the
    third-party disclose/redact judgement. When set, `messages/` is written
    with a prominent REVIEW REQUIRED banner and flagged in the manifest.
  - global `-n` / `--dry-run` - resolve the subject and report what would be
    collected and written, making no files.

Everything else (profile/PII, authored posts, activity, group memberships) is
collected by default; only private messages are gated behind `--messages`.

### Output bundle

A directory the controller reviews, then sends (after redaction):

```text
sar-jane-doe-2026-06-23/
  README.md            # cover sheet: subject, forum, generated-at, the
                       #   controller checklist, and an Article 15
                       #   supplementary-information template to fill in
  manifest.json        # machine-readable index: sections, item counts, and a
                       #   `review_required` list (e.g. private messages)
  profile.json         # account data / PII (admin detail + secondary emails)
  groups.json          # group memberships
  posts/               # every post the subject authored, as Markdown
    <topic-slug>-<post-id>.md
  posts.json           # same, structured (ids, timestamps, urls, raw)
  messages/            # ONLY when --messages; REVIEW REQUIRED banner in README
    <thread-slug>-<topic-id>.md
  activity.json        # likes given/received and other user actions
```

`manifest.json` carries counts and a `review_required` array so the human step
is auditable, e.g.:

```json
{
  "subject": { "username": "jane-doe", "user_id": 412, "email": "jane@example.com" },
  "forum": "rcpch",
  "generated_at": "2026-06-23T09:00:00Z",
  "sections": { "posts": 84, "messages": 7, "groups": 3 },
  "review_required": [
    "messages/ contains third-party personal data; review before disclosure",
    "profile.json includes IP addresses; confirm these should be released"
  ]
}
```

The `README.md` cover sheet includes a checklist:

```text
- [ ] Verify the requester is the data subject (or authorised).
- [ ] Review messages/ for third-party personal data and redact.
- [ ] Confirm IP addresses / technical data should be released.
- [ ] Complete the Article 15 supplementary information below.
- [ ] Send via a secure channel within one month of the request date.
```

## Reference: API calls

(To be confirmed against the running Discourse during implementation - the
admin API is not formally versioned. Endpoints `dsc` already uses are marked.)

- **Full PII** (new): `GET /admin/users/{id}.json` - name, all emails,
  `registration_ip_address`, `ip_address` (last), `created_at`, `last_seen_at`,
  `last_emailed_at`, trust level, custom/profile fields, associated accounts,
  staged/active flags. (`dsc user info` currently uses the thinner public
  `/u/{username}.json`.)
- **Resolve email -> account** (new-ish): `GET /admin/users/list/all.json?email=<email>`
  or the filter param, then take the id. (Mirrors the implemented `dsc user find`.)
- **Authored posts** (have): `fetch_user_actions` with the post filters; fetch
  each post's raw via the existing topic/post path.
- **Private messages** (have): the `dsc pm list` path
  (`/topics/private-messages/{username}.json` and the sent variant).
- **Group memberships** (have): from `/admin/users/{id}.json` or `dsc user
  groups list`.
- **Likes/actions** (have): `fetch_user_actions` (action type 1 = like).

## Phases

### Phase 1 - blocking (single-forum MVP)

- [ ] `fetch_admin_user_detail(user_id)` -> `/admin/users/{id}.json` (full PII).
- [ ] Resolve `<USER>` as username or email to a user id.
- [ ] Write the bundle: `profile.json`, `groups.json`, `posts/` + `posts.json`,
      `activity.json`. `messages/` only when `--messages` is set.
- [ ] `README.md` cover sheet (subject, forum, generated-at, controller
      checklist, Article 15 template) + `manifest.json` with counts and
      `review_required` flags (IP data always; `messages/` when included).
- [ ] `--output`, `--messages`, `--dry-run`.

### Phase 2 - iteration ergonomics (build on demand)

- [ ] `--zip` for secure transfer.
- [ ] A single combined human-readable `sar-<username>.md` (some controllers
      prefer one document).
- [ ] Staff/admin notes about the subject (if the Staff Notes plugin is
      present) - these are disclosable personal data.
- [ ] Bookmarks, drafts, preferences, screened-email/IP history where exposed.

## Backward compatibility

New command; nothing existing changes. Reuses existing client methods where
possible and adds `fetch_admin_user_detail`.

## Out of scope

- Identity verification, redaction, exemption decisions, and the legal
  sufficiency of the response - these are the controller's responsibility (see
  "Compliance scope").
- Erasure / "right to be forgotten" (`dsc user anonymize`/delete) - a different
  GDPR right, deliberately not bundled here.
- **Multi-forum fan-out** - deliberately not built. A subject who appears on
  several of one org's forums is rare enough that running `dsc sar` once per
  forum is fine; the per-forum command already removes the bulk of the labour.
  No `dsc sar all` / `--tags` aggregation.
- Holding or transmitting the bundle securely after generation - the output is
  sensitive personal data; transport and retention are the operator's job. The
  tool should print a reminder to that effect.

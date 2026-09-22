# dsc sar

Export the personal data `dsc` can retrieve about one person into a single, reviewable **Subject Access Request** (SAR / DSAR) bundle - the data-gathering half of answering a GDPR Article 15 request.

```
dsc sar <discourse> <user> [--output <dir>] [--messages] [--dry-run]
```

- `<user>` is a **username or an email address** (the email is resolved to the account via the admin user search).
- `--output` / `-o`: destination directory. Defaults to `sar-<username>-<date>/` in the current directory.
- `--messages`: also collect the subject's private messages. **Off by default** - PMs necessarily contain other people's personal data, so disclosing them is a judgement call for the controller. When included they are written with a prominent REVIEW REQUIRED banner and flagged in the manifest.
- `-n` / `--dry-run`: resolve the subject and report what would be collected, writing nothing.

## What it is (and isn't)

`dsc sar` automates the laborious part - finding and packaging the supported personal-data surfaces exposed through the Discourse admin API - and **scaffolds the rest**. It does **not** make the response automatically "legally compliant" or claim that Discourse exposes every datum through those APIs; that remains the data controller's responsibility. The bundle's `README.md` cover sheet lists the steps that are still yours:

- verifying the requester is the data subject,
- reviewing private messages for third-party data and redacting,
- applying exemptions,
- completing the Article 15 supplementary information (purposes, retention, recipients, …),
- sending securely within one calendar month.

It never implies the bundle is ready to send unreviewed.

## The bundle

```
sar-jane-doe-2026-06-23/
  README.md         cover sheet: subject, forum, checklist, Article 15 template
  manifest.json     machine-readable index, counts, and review_required flags
  profile.json      account / profile data (PII): emails, IP addresses, etc.
  groups.json       group memberships
  posts/            every post the subject authored, as Markdown
  posts.json        the same, structured (ids, urls, timestamps, full raw)
  activity.json     likes the subject gave
  messages/         private messages -- only with --messages; REVIEW REQUIRED
```

`manifest.json` carries a `review_required` array (IP data, and `messages/` when included) so the human steps are explicit and auditable.

Discourse Chat is not included yet. Chat can contain personal data relevant to a SAR, but a defensible export must account for channel visibility, DMs, threads, edits, deletions, uploads, retention expiry, and API pagination rather than copying whichever messages happen to be visible in an interactive client. Deterministic Chat archival and opt-in SAR inclusion are tracked as [R12](https://github.com/koloki-co/dsc/blob/main/spec/roadmap.md) and specified in [chat archival](https://github.com/koloki-co/dsc/blob/main/spec/commands/chat-archival.md).

## Requirements & handling

- Needs an **admin** API key (it reads `/admin/users/{id}.json`, which carries the full PII surface the public profile endpoint omits).
- The output is **sensitive personal data**. Store and transmit it securely, and delete it once the request is fulfilled. `dsc` prints a reminder to that effect.

## Examples

```bash
# By username, default output directory
dsc sar rcpch jane-doe

# By email, to a chosen directory, including private messages
dsc sar rcpch jane@example.com -o ./sar-jane --messages

# Preview without writing anything
dsc -n sar rcpch jane-doe
```

For SARs spanning several forums, run `dsc sar` once per forum - multi-forum aggregation is deliberately out of scope.

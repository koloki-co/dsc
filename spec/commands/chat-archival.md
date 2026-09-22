# `dsc chat` - deterministic Chat archival for compliance exports

Spec for R12. Goal: export Discourse Chat data to bounded, reviewable local files and include a subject's relevant Chat data in a SAR when explicitly requested. Driver: Chat messages may be personal data required for a Subject Access Request, while an interactive MCP conversation is not a complete or reproducible archive.

## Motivation

Discourse's built-in MCP server is the maintained route for an AI assistant to read and write Chat on one forum. `dsc` should not duplicate that conversational surface. The remaining operator gap is deterministic archival: enumerate all data the API makes available, follow pagination under explicit budgets, preserve stable metadata and upload references, report omissions, and write files that can be reviewed, retained, checksummed, or included in a compliance bundle.

## Current state (as of 2026-09-21)

`dsc` has no Chat command. `dsc sar` exports profile data, posts, groups, activity, and optional topic-backed private messages, but it does not include Discourse Chat. The built-in MCP offers interactive Chat tools, subject to the connected user's visibility, but does not produce a deterministic subject archive or claim SAR completeness.

## Proposed CLI surface

The exact surface follows API discovery rather than preceding it. The intended operator capabilities are:

```text
dsc chat channels <discourse> [--format text|json|yaml]
dsc chat fetch <discourse> <channel> [--since <time>] [--output <path>] [--format json|yaml]
dsc sar <discourse> <user> --chat
```

- `chat channels` inventories the channels visible to the configured administrative identity and states whether the result is exhaustive.
- `chat fetch` writes a stable, bounded archive of one channel, including message and thread identifiers, authors, timestamps, edit state, reply relationships, and upload references where exposed. Human-readable rendering is optional; structured output is canonical.
- `sar --chat` is opt-in because Chat contains third-party personal data. It exports only data attributable or relevant to the subject according to the API behavior established during discovery, adds it to `manifest.json`, and marks it `REVIEW REQUIRED`.
- Every export reports pagination budgets, time bounds, inaccessible scopes, retention gaps, deleted-content behavior, and other reasons it may be incomplete. Silence must never imply completeness.

## Phase 1 - API and compliance discovery

- [ ] Capture exact requests and redacted responses from a current Discourse for channel inventory, channel messages, threads, DMs, and user-authored Chat activity.
- [ ] Establish what an administrator can retrieve versus what only a channel participant can retrieve.
- [ ] Determine how edits, deletions, uploads, reactions, threads, and retention-expired messages appear or disappear.
- [ ] Determine whether the API can enumerate a subject's messages directly or requires bounded channel traversal.
- [ ] Record pagination semantics, request budgets, ordering guarantees, and rate limits.
- [ ] Decide whether a defensible subject export is possible. If not, retain channel archival without claiming SAR coverage.

## Phase 2 - deterministic channel archive

- [ ] Implement bounded channel inventory and message pagination with truthful completeness metadata.
- [ ] Define a versioned JSON/YAML archive schema separate from raw API models.
- [ ] Preserve source identifiers and timestamps needed to correlate messages, threads, replies, edits, and uploads.
- [ ] Write atomically and refuse ambiguous or colliding output paths.
- [ ] Support `--dry-run` as a request and output plan that writes nothing.
- [ ] Add fixture-backed pagination, truncation, malformed-response, and request-budget tests.

## Phase 3 - SAR integration

- [ ] Add an explicit `dsc sar --chat` opt-in.
- [ ] Add Chat files, counts, provenance, and completeness warnings to the SAR manifest and cover sheet.
- [ ] Mark Chat as requiring review for third-party personal data, including other participants in DMs and threads.
- [ ] Keep Chat failure visible: a partial or inaccessible Chat export must fail the requested section or mark the bundle incomplete, never silently omit it.

## Backward compatibility

This is additive. Existing `dsc sar` output remains unchanged unless `--chat` is supplied. Schema versions make future archive evolution explicit.

## Out of scope

- Generic `chat send`, reaction, or moderation commands without a separate operator driver.
- An agent-oriented conversational Chat interface; use Discourse's built-in MCP server.
- Claiming access to deleted or retention-expired data that Discourse no longer exposes.
- Automated legal conclusions, disclosure decisions, or third-party redaction.
- Cross-forum SAR aggregation; the existing per-forum SAR decision remains unchanged.

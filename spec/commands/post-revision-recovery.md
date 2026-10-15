# `dsc post revision` - inspect and safely revert post revisions

Spec for inspecting a post's edit history and reverting a selected revision through Discourse's supported endpoint. Goal: make accidental content replacement recoverable through a reviewable, dry-run-first `dsc` workflow. Driver: an accidental whole-body replacement of a private forum's entity-information opening post required manual recovery through the Discourse UI.

## Motivation

`dsc topic push` updates an opening post by replacing its complete body. It deliberately preserves Discourse's edit-history revision by default, but `dsc` offers no way to inspect that history or recover the earlier content. Operators must leave their repeatable CLI workflow for the browser UI, where it is too easy to select the wrong version or lose the evidence of what will change. The tool already supports soft-delete recovery for whole topics; it needs the equivalent reviewed recovery path for a post that remains present but whose content or topic metadata was accidentally replaced.

## Current state (as of 2026-10-12)

`dsc 0.21.0` can pull, edit, delete, move, and inspect basic metadata for a post, but has no post-revision client or command. `topic push` calls the standard post-update endpoint and normally creates a revision; `--skip-revision` deliberately suppresses that history and is therefore not recoverable through this feature.

Current Discourse routes expose a bounded revision chain per post, but no collection endpoint. `GET /posts/:post_id/revisions/latest.json` returns the latest visible revision plus its `first_revision`, `previous_revision`, `current_revision`, `next_revision`, `last_revision`, and `version_count` navigation fields. `GET /posts/:post_id/revisions/:revision.json` returns one revision and its field-level diffs. The serializer considers at most the latest 99 stored revisions, so `dsc` must state that limit rather than claiming a complete unlimited history.

## Proposed CLI Surface

```text
dsc post revision list <DISCOURSE> <POST_ID> [--format text|json|yaml]
dsc post revision show <DISCOURSE> <POST_ID> <REVISION> [--format text|json|yaml]
dsc post revision revert <DISCOURSE> <POST_ID> <REVISION> --yes
```

- `list` starts at `revisions/latest`, follows the server-provided revision links, and prints each visible revision in newest-first order. Text output contains revision number, creation time, editor, edit reason, and changed fields without printing post bodies. JSON and YAML preserve the same structured summaries.
- `show` retrieves one revision. Text output presents its metadata and a raw-Markdown body diff where available, never cooked HTML. JSON and YAML expose the structured API fields so callers can review title, category, tags, reply target, and body changes without parsing text output.
- `revert` first retrieves the current post and selected revision. Under `--dry-run`, it prints the post, target revision, changed fields, and the exact staff-only `PUT` it would issue, then makes no mutation. A live revert requires `--yes`, sends the supported reversion request, re-fetches the post and latest revision, and reports the resulting revision and edit reason.
- `revert` must describe the server's actual semantics precisely: it asks Discourse to apply the selected revision's prior values. It must not label this as an opaque "restore revision N" operation or implement recovery by copying a rendered diff into a normal post update.
- All three commands remain post-level. An operator supplies the relevant post ID rather than treating a topic-level opening-post revert as a different resource.

## Reference: API calls observed in the field

Confirmed from the checked-out Discourse source on 2026-10-12. No live recovery request was sent while writing this spec.

```text
GET /posts/:post_id/revisions/latest.json
GET /posts/:post_id/revisions/:revision.json
PUT /posts/:post_id/revisions/:revision/revert.json
```

The routes are defined in `discourse/config/routes.rb`. `PostsController#revert` requires a staff user, rejects revision numbers below 2 with `400`, requires that the named post and revision exist and are visible/editable, and returns `422` when the selected revision has no applicable change. Reversion can restore raw content and, for an opening post, title, category, and tags; it can also restore a reply target. A successful response includes the resulting post and, for an opening post, topic data. The command must use `Api-Key` and `Api-Username` headers through the existing `DiscourseClient`, never expose them in output, and surface shared HTTP errors unchanged.

## Phases

### Phase 1 - blocking recovery

- [x] Add revision response models and bounded client methods for latest and named revisions.
- [x] Add `dsc post revision list` and `show` with text, JSON, and YAML output, including the 99-revision visibility limit.
- [x] Add `dsc post revision revert` with global `--dry-run`, mandatory `--yes` for a live mutation, current-state preflight, and post-write verification.
- [x] Add mock-server integration tests for linked revision traversal, inaccessible or missing revisions, revision 1 refusal, no-op reversion, dry-run zero writes, exact PUT path, and post-write verification.

### Phase 2 - recovery ergonomics

- [ ] Add an optional local evidence file for a reviewed recovery plan and result, written atomically without credentials.
- [ ] Assess whether `topic` needs a thin opening-post revision wrapper after real use demonstrates that repeatedly resolving post IDs is a material workflow cost.

## Backward compatibility

This is an additive `post` subcommand. Existing post and topic pull/push behaviour does not change. Existing revision history remains authoritative on the server; `dsc` does not create, hide, or rewrite history except when an explicitly confirmed revert causes Discourse to create its normal new revision.

## Out of scope

- Automatic rollback after any failed or surprising push.
- Timestamp-based or unattended recovery.
- Batch revision reverts across posts or forums.
- Hiding or permanently deleting revision history.
- Restoring deleted posts or topics, which use the existing recovery commands.
- Reconstructing history when a prior update used `--skip-revision` or the server has already removed the revision.

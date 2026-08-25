# `dsc topic change-owner` - reassign post authorship

Spec for reassigning the visible author of a topic's post(s) to a different user. Goal: let an operator who posted content on someone else's behalf (e.g. compiled from a staff member's source document) correct authorship without leaving the CLI. Driver: ACCM Discourse (`kitchen.culinarymedicine.org`) - five FAQ topics were created via `dsc topic new` under the operator's own admin account from a staff-authored source document, then needed reassigning to the actual author's account.

## Motivation

`dsc topic new` always creates a topic under the API-authenticated user. When the content actually belongs to someone else - a staff member's document, a migrated post, ghost-written material - there's currently no `dsc` command to fix authorship afterward. The only workaround today is the Discourse Admin UI: open the topic, click the wrench icon in the timeline, choose "Change owner," pick the user, and repeat per topic. That's fine for one-off, low-volume corrections (as it was here, five topics) but doesn't scale, isn't scriptable, and can't be batched across a fleet.

## Current state (as of 2026-08-23)

Neither `dsc topic` nor `dsc post` has an ownership-change subcommand. `dsc post info` (R44) can resolve a post ID to metadata but is read-only and explicitly does not return author data.

## Proposed CLI surface

```text
dsc topic change-owner <DISCOURSE> <TOPIC_ID> <USERNAME> [--post <POST_ID>]...
```

- With no `--post` flags: reassigns just the topic's first post (the OP), matching what most people mean by "change the topic's owner."
- One or more `--post <POST_ID>` flags: reassigns exactly those posts within the topic to `<USERNAME>` instead, for multi-post topics where only some replies need reassigning.
- `<USERNAME>` is validated against the target Discourse before the request is sent (reuse the existing user-lookup helper used elsewhere in `dsc`), so a typo fails fast with a clear error rather than a confusing 4xx from Discourse.
- Honours `--dry-run`: prints the resolved topic ID, post ID(s), current author(s), and target username without sending the request.
- Not a `pull`/`push` pair - this is a one-shot mutation, closer in shape to `dsc topic title` or `dsc topic tag`.

## Reference: API calls observed in the field

Verified against ACCM on 25 August 2026 using `dsc 0.15.0` and Discourse `2026.8.0-latest` (`2306592f8992255162cf7c7fc5055d5277b3d1a4`). `POST /t/{topic_id}/change-owner.json` with form fields `topic_id`, canonical-cased `username`, and one or more repeated `post_ids[]` returned success and reassigned ten topic opening posts. Discourse's endpoint performs a case-sensitive `User.find_by(username:)`: a lowercase username copied from a profile URL passed `dsc`'s user lookup but returned `422 {"failed":"FAILED"}` from the ownership endpoint. `dsc` must therefore send the canonical username returned by its validation lookup rather than the caller's original spelling.

## Phases

### Phase 1 - blocking

- [x] `dsc topic change-owner <discourse> <topic_id> <username>` - OP-only reassignment, the common case.
- [x] `--dry-run` support.
- [x] Username validation against the target Discourse before the mutating request.

### Phase 2 - iteration ergonomics

- [x] `--post <POST_ID>` repeatable flag for reassigning specific non-OP posts.
- [ ] Consider a corresponding `dsc post change-owner <discourse> <post_id> <username>` alias for the single-post case, mirroring the `topic`/`post` split used elsewhere (e.g. `topic pull --full` vs `post pull`).

### Phase 3 - nice to have

- [ ] Bulk mode: accept a file of `topic_id,username` pairs for batch reassignment after a large content migration.

## Backward compatibility

New subcommand; nothing existing changes.

## Out of scope

- Anonymizing or merging accounts (see `dsc user anonymize`, already out of scope for different reasons).
- Reassigning ownership of uploads/attachments independently of the post they're attached to.

# `dsc topic close|reopen` - close a topic without archiving it

Spec for closing and reopening one existing topic. Goal: a guarded, verifiable status change that touches nothing but the `closed` flag. Driver: an operator deprecating documentation topics (R90) - after a no-bump OP notice, a `DEPRECATED:` title prefix and an additive `deprecated` tag, the topics needed closing, and `dsc 0.24.0` had no way to do it.

> **Status: implemented on branch (2026-11-23), pending live verification.**

## Motivation

The deprecation workflow is otherwise fully covered by `topic push --no-bump`, `topic title` and `topic tag`. The last step - closing the topic to new replies - had to be done in the Discourse UI. Closing must not be confused with archiving (which also freezes the topic and removes it from lists) or with soft deletion.

## CLI surface

```text
dsc topic close  <DISCOURSE> <TOPIC_ID>
dsc topic reopen <DISCOURSE> <TOPIC_ID>
```

1. `GET /t/{id}.json` for fresh state.
2. Refuse if `deleted_at` is set, pointing at `dsc topic restore`.
3. If `closed` already matches the request, print `already closed` / `already open` and exit 0 with no write (prefixed `[dry-run]` under `--dry-run`).
4. Under `--dry-run`, print the exact request and the current archived status, then stop.
5. `PUT /t/{id}/status.json` with form body `status=closed&enabled=true|false`.
6. Re-fetch and fail unless `closed` now matches and `archived` is unchanged.

## Reference: Discourse source (upstream `main`, reviewed 2026-11-23)

Routes: `put "t/:topic_id/status" => "topics#status"` (and the `t/:slug/:topic_id/status` form).

`TopicsController#status` requires `status` and `enabled`, optionally permits `until` and `category_id`, checks `guardian.ensure_can_close_topic!` for `status=closed`, then calls `topic.update_status`. The response is `success_json` plus `topic_status_update` (the topic timer, often `null`).

`TopicStatusUpdater#update!`:

- Updates conditionally (`WHERE closed = !enabled`), so a repeated request is a server-side no-op and adds no post.
- When the state changed, adds a `small_action` moderator post with `action_code` `closed.enabled` / `closed.disabled`. `bump` is true only when opening, so **close does not bump; reopen does**. `silent` is not set by the controller.
- Closing deletes pending `close` / `silent_close` topic timers; opening deletes the `open` timer and calls `inherit_auto_close_from_category`.
- Closing removes the topic from `CategoryFeaturedTopic`.
- Fires `topic_status_updated` and `topic_closed` (`:manually`) events, so webhooks and plugins listening for those will see the change.

No live request has been sent yet.

## Phases

### Phase 1 - blocking

- [x] `DiscourseClient::set_topic_closed` and `dsc topic close|reopen`.
- [x] Fresh-state read, deleted-topic refusal, already-matching no-op, exact dry-run, post-write verification of `closed` and `archived`.
- [x] Mock-server tests (`tests/topic-close-test.rs`): exact form body, zero dry-run writes, archived preserved, reopen payload, no-op both ways, deleted refusal, unapplied-write detection; both commands in the `--dry-run` mutation guarantee table.
- [ ] Authorised isolated live test: record redacted response, Discourse version, small-action post, bump and notification behaviour.

### Phase 2 - nice to have

- [ ] Multiple topic IDs per invocation (mirroring `topic delete`) if a batch deprecation needs it.
- [ ] Timed close (`--until`) via the same endpoint, only on demand.

## Backward compatibility

Additive subcommands. No existing command changes.

## Out of scope

- Archiving/unarchiving, pinning, and listing/unlisting - same endpoint, different semantics; add separately on demand rather than generalising this command.
- Topic timers (`/t/{id}/timer`).
- Suppressing the small-action post - the controller offers no option for it.

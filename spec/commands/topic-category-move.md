# `dsc topic move` - move a topic between categories

Spec for moving one existing topic to a different category without recreating it. Goal: preserve the topic ID, posts, replies, revisions and links while making a reviewable category change. Driver: the private finance archive is consolidating entity-information threads into one restricted Business category. `dsc 0.24.0` can create, pull, push, rename and tag topics, but cannot move an existing topic to a category. Recreating the topic would break its identity and lose replies and revision history, so the operator must currently use the Discourse UI.

## Motivation

A category change is a normal part of forum reorganisation, especially when a private archive introduces a cross-entity category after years of entity-specific threads. The existing `category pull` and `push` commands are content-copying workflows, not safe ways to relocate one live topic. They create a replacement topic rather than retaining the original topic ID and history. A direct API call is possible, but bypasses `dsc`'s dry-run, validation and verification conventions.

## Current state (as of 2026-11-19)

`dsc 0.24.0` has no command that changes a topic's category. `dsc post move` moves a post into another topic, which is a different operation. `dsc topic title` and `dsc topic tags` already use `PUT /t/{id}.json` for topic metadata, but neither accepts a destination category.

Discourse core's `TopicsController#update` accepts `category_id`. It validates the change through `TopicCategoryChangeValidator` and applies it through `PostRevisor`, rather than recreating the topic. This was inspected in the upstream Discourse source on 2026-11-19. No live category-move request has been sent for this spec, so the endpoint contract and response shape need live verification before release.

## Proposed CLI surface

```text
dsc topic move <DISCOURSE> <TOPIC_ID> <CATEGORY> [--format text|json|yaml]
```

- `<CATEGORY>` accepts the same ID or slug forms as other category commands.
- The command reads the current topic and resolves the destination category before planning or writing.
- `--dry-run` prints the topic ID, title, current category and destination category, then sends no `PUT` request.
- A live invocation sends `PUT /t/{topic_id}.json` with `category_id=<destination-category-id>`. It must not send a replacement body, create a topic, move individual posts or change title, tags, status, ownership or pin state.
- After a successful response, the command re-fetches the topic and verifies its unchanged ID, destination category and post count. It reports the old and new category IDs and names. Structured output contains the same identifiers.
- Moving a topic to its current category is a verified no-op and sends no mutation.
- The command relies on Discourse to enforce category permissions, required tags and other category rules. It surfaces the server's validation error without attempting a workaround.

## Reference: API evidence

Source-derived from current upstream Discourse on 2026-11-19. `TopicsController#update` handles the existing topic route and applies `params[:category_id]` when it differs from the topic's current category:

```text
PUT /t/{topic_id}.json
Content-Type: application/x-www-form-urlencoded

category_id={destination-category-id}
```

The controller calls `TopicCategoryChangeValidator` before passing the category change to `PostRevisor`. This indicates an in-place move with normal revision/audit behaviour, but a real forum must confirm the exact accepted form shape, response fields, activity-bump behaviour and staff-action record. No API key, response body or production write was retained for this report.

## Phases

### Phase 1 - blocking move

- [ ] Add a client method for the in-place category update and a `dsc topic move` command.
- [ ] Implement complete dry-run output, same-category no-op handling and post-write verification.
- [ ] Add mock-server tests for ID and slug destination resolution, exact form payload, zero dry-run writes, category-rule rejection, same-category no-op, failed mutation and post-write mismatch.
- [ ] Verify the endpoint against a non-production forum and record the redacted request shape, response fields, Discourse version, resulting revision and activity-bump behaviour.

### Phase 2 - ergonomics

- [ ] Consider a file-driven batch mode only after a real migration needs it. It must plan every topic independently, stop safely on failure and never substitute category copy/push for an in-place move.

## Backward compatibility

This is an additive topic subcommand. Existing `topic pull`, `push`, `title`, `tag`, `tags` and `post move` behaviour does not change. The command changes a topic's category only when explicitly invoked.

## Out of scope

- Recreating, copying, deleting or merging topics as a substitute for an in-place category move.
- Changing topic content, replies, title, tags, ownership, status, pin state or access controls.
- Moving categories in the hierarchy, which remains `dsc category def` work.
- An implicit bulk move across categories or forums.

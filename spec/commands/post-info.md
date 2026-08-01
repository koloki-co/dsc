# `dsc post info` - inspect post metadata and canonical URL

Spec for a read-only `dsc post info` command. Goal: resolve a known post ID to its canonical topic URL and minimal moderation-relevant metadata without exporting its body or author data. Driver: while validating AI spam detection on `discourse.openehr.org`, a staff action-log entry identified a recently deleted spam post but exposed only its post ID and topic title. Discourse's AI spam-test UI accepts a deleted post only as a topic URL, while its direct post-ID path excludes soft-deleted posts. The Admin UI's completed Reviewables list does not expose a copyable post URL or post ID.

## Motivation

`dsc log staff <forum> --action delete_post` can identify a soft-deleted post ID, but its audit detail has no topic ID, category, or canonical URL. `dsc post pull` prints only the raw body, which is unnecessarily sensitive for a metadata lookup, and `dsc post move` internally resolves the source topic but is a mutating command. Administrators need a safe way to turn a post ID, including a soft-deleted post ID visible to their API key, into a stable URL for review, moderation diagnostics, and Discourse AI test workflows.

## Current state (as of 2026-07-31)

- `dsc post pull <forum> <post-id>` calls `GET /posts/:id.json?include_raw=1` and writes only the raw Markdown body.
- `dsc post move` already calls the same post endpoint internally to obtain the source `topic_id` and `post_number`, but does not expose that metadata as a read-only operation.
- `dsc log staff` returns a flattened, redacted audit record. A `delete_post` record can include a post ID and a topic title but not the topic ID or URL.
- `dsc topic list --deleted` lists deleted topics, not deleted posts within a topic.

## Proposed CLI surface

```text
dsc post info <discourse> <post-id> [--format text|json|yaml]
```

- Fetch the post by ID using the configured API key. The command is read-only and does not accept `--dry-run`-specific behaviour.
- Resolve the associated topic to return its title, slug, category ID, and deletion state. Do not request or print post raw content.
- Support soft-deleted posts and soft-deleted topics when the authenticated API user is permitted to moderate the topic. Return the API's permission/not-found error otherwise.
- Return a canonical absolute post URL. Prefer Discourse's `post_url` field; when it is absent, construct the URL from the configured base URL, topic slug/ID, and post number.
- Use the project's standard `--format` values. Text output is a concise labelled record; JSON and YAML expose the same stable field names.

## Output schema

```json
{
  "id": 61695,
  "topic": {
    "id": 12345,
    "title": "Example topic",
    "slug": "example-topic",
    "category_id": 5,
    "deleted_at": null
  },
  "post_number": 2,
  "url": "https://forum.example/t/example-topic/12345/2",
  "deleted_at": "2026-06-30T14:31:08Z"
}
```

`id`, `topic.id`, `post_number`, and `url` are required on successful output. Topic title, slug, category ID, and either deletion timestamp may be absent when the relevant Discourse version does not serialize them.

The command must not include `raw`, author username, email address, IP address, cooked HTML, revision history, reviewable scores, or API credentials. `dsc post pull` remains the explicit raw-content operation.

## Reference: API calls observed in the field

Tested against `discourse.openehr.org` on Discourse `2026.8.0-latest` using an administrator API key.

The existing `dsc` client already uses:

```text
GET /posts/61695.json?include_raw=1
```

Current Discourse resolves a staff-visible deleted post with `Post.with_deleted`, checks that the caller can moderate its topic, and serializes `id`, `topic_id`, `post_number`, `deleted_at`, and `post_url`. The command should omit `include_raw=1` because it does not need the post body.

The associated topic is resolved with:

```text
GET /t/<topic-id>.json
```

Its metadata supplies `id`, `title`, `slug`, `category_id`, and `deleted_at`. No raw body from either response is persisted or emitted.

## Phases

### Phase 1 - blocking

- [ ] Add `dsc post info <discourse> <post-id>` with text, JSON, and YAML output.
- [ ] Extend the post API model to deserialize only the metadata required for the output schema, including `post_url`.
- [ ] Fetch the associated topic without raw content and return its metadata.
- [ ] Cover visible, deleted-post, and deleted-topic responses; assert raw and author fields never appear in output.
- [ ] Add `docs/post.md` usage and output examples.

### Phase 2 - reviewables workflow

- [ ] Assess a read-only `dsc reviewable list` surface for recent handled spam, including post ID and canonical post URL, only if a concrete Discourse admin API response is captured and redacted.

## Backward compatibility

This adds one read-only subcommand. It does not change `post pull`, which continues to retrieve raw Markdown only when explicitly requested, or any existing output schema.

## Out of scope

- Listing, restoring, approving, rejecting, or otherwise changing Reviewables.
- Post-body, revision, or user-data export.
- Topic search or a general deleted-post listing.
- Circumventing Discourse category permissions or exposing deleted content to a non-staff API key.

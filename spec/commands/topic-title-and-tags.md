# `dsc topic` - title and tag editing

> **Status: Implemented (v0.10.18).** `dsc topic title` and `dsc topic tags`
> ship, both honouring `--dry-run`; the reserved-slug `403` is surfaced with a
> clear message. The "Future" front-matter-title enhancement is already
> satisfied - `dsc category push` prefers the `title` field in a file's YAML
> front matter when creating a topic.

Spec for two missing `dsc topic` subcommands: `title` (rename a topic) and
`tags` (set/replace a topic's tag list). Goal: allow the full topic
metadata surface to be managed from the CLI without needing raw `curl`.
Driver: `forum.rcpch.tech/c/playbook` migration - 15 newly-created topics
had slug-derived titles (e.g. `git-overview`, `twelve-factor-apps`) because
`dsc category push` derives titles from filenames; bulk-renaming and tagging
required direct API calls workaround.

## Motivation

When `dsc category push` creates new topics it slugifies the local filename
to produce the title. The result is lowercase slug strings as Discourse
topic titles (e.g. "git-overview", "python-virtual-environments"). There is
no `dsc` subcommand to correct these after the fact; the workaround is a
raw `curl PUT /t/{id}.json` call with `title=...`.

Similarly, topics created via `dsc category push` or `dsc topic new` have no
tags. The existing `dsc topic tag` / `dsc topic untag` subcommands add or
remove individual tags but there is no way to set the full tag list
atomically, which is what you want when initialising tags on a fresh batch
of newly-created topics.

Both gaps are felt immediately after any bulk `category push` that creates
new topics.

## Current state (as of 2026-06-22)

- `dsc topic push <discourse> <id> <file>` - updates the first post body
  only. Does not touch title or tags.
- `dsc topic tag <discourse> <id> <tag>` - adds one tag. Cannot replace
  the full tag list.
- `dsc topic untag <discourse> <id> <tag>` - removes one tag.
- No `dsc topic title` command exists.
- Title changes require: `curl -X PUT https://<host>/t/<id>.json -d "title=..."`
- Full tag replacement requires: `curl -X PUT https://<host>/t/<id>.json -d "tags[]=tag1&tags[]=tag2"`

One edge case: Discourse reserves certain slugs (`contact`, `about`, etc.)
as system routes. Attempts to rename a topic whose slug matches a reserved
word return `403 invalid_access`. `dsc topic title` should surface this
clearly rather than silently failing.

## Proposed CLI surface

```text
dsc topic title <DISCOURSE> <TOPIC_ID> <TITLE>
dsc topic tags  <DISCOURSE> <TOPIC_ID> [<TAG>...]
```

### `dsc topic title <discourse> <topic_id> <title>`

Renames the topic's title in-place.

- Calls `PUT /t/{id}.json` with `title=<title>`.
- Prints the previous title and the new title on success:
  `renamed topic 723: "git-overview" → "Get Git: an overview of our Git workflow"`
- On 422 (title too short/long, duplicate): prints the Discourse error
  message and exits non-zero.
- On 403: prints a clear message: `topic <id> title cannot be changed
  (reserved slug or insufficient permission)` and exits non-zero.
- Supports `--dry-run`: prints what would be sent without calling the API.
- Note: changing a title changes the Discourse slug, which means the
  URL of the topic changes. Warn the user: `note: topic URL will change
  from /t/<old-slug>/<id> to /t/<new-slug>/<id>`.

### `dsc topic tags <discourse> <topic_id> [<tag>...]`

Sets the complete tag list for a topic, replacing any existing tags.
Passing zero tags clears all tags.

- Calls `PUT /t/{id}.json` with `tags[]=tag1&tags[]=tag2&...` (or
  `tags[]=` with an empty value to clear).
- Prints the previous tag list and the new tag list on success:
  `tags set on topic 723: [] → [git, developer-guide]`
- On 422 (tag doesn't exist, tag limit exceeded): prints the Discourse
  error and exits non-zero.
- Supports `--dry-run`.
- If a tag name is provided that does not exist on the Discourse instance,
  Discourse may auto-create it (depending on site settings) or reject it.
  Surface whichever outcome occurred.

## Future: title field in YAML front matter for `category push`

`dsc category push` currently derives the topic title from the filename
stem when creating a new topic. A cleaner approach: if the YAML front
matter contains a `title:` field, use that as the topic title on creation.
This would avoid the slug-derived title problem entirely.

Example: a file `git-overview.md` with front matter:

```yaml
---
title: "Get Git: an overview of our Git workflow"
---
```

...would create the topic with that title rather than "git-overview".

This is a companion enhancement to `category push`, not a separate
subcommand, but it belongs in the same fix as `dsc topic title`.

## Reference: API calls observed in the field

Tested against Discourse stable (`forum.rcpch.tech`), 2026-06-22.

### Rename a topic title

```
PUT /t/{id}.json
Content-Type: application/x-www-form-urlencoded
Api-Key: <admin-key>
Api-Username: <admin-username>

title=Get+Git%3A+an+overview+of+our+Git+workflow
```

Response on success: `200 OK`, JSON body contains updated topic object.
Response on reserved slug: `403`, `{"errors":["You are not permitted to
view the requested resource."],"error_type":"invalid_access"}`.

Tested for 19 topics in a batch; all returned 200 except topic 720 whose
slug was `contact` (a reserved Discourse route).

### Set tags atomically

```
PUT /t/{id}.json
Content-Type: application/x-www-form-urlencoded
Api-Key: <admin-key>
Api-Username: <admin-username>

tags[]=git&tags[]=developer-guide
```

Response on success: `200 OK`, JSON body contains updated topic object
with `tags` array.

To clear all tags:

```
tags[]=
```

(empty value; Discourse interprets this as an empty tag list)

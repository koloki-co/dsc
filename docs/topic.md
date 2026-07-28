# dsc topic

Pull, push, sync, delete, restore, and tag individual topics.

## dsc topic pull

```
dsc topic pull <discourse> <topic-id> [<local-path>] [--full|-F]
```

Pulls the specified topic into a local Markdown file.

By default writes only the topic's first post (the OP), suitable for the `pull → edit → push` round-trip with `dsc topic push`.

If `<local-path>` is omitted, the topic is written to a new file in the current directory (named from the topic title). Directories are created as needed.

### `--full` (read-only thread snapshot)

```bash
dsc topic pull myforum 364 --full
dsc topic pull myforum 364 thread.md --full
```

Pulls every post in the thread (paginating internally as needed) into a single Markdown file with YAML frontmatter and per-post headings:

```markdown
---
title: Sitekit, eRedBook and Harris Health Alliance Acquisition
topic_id: 364
url: https://forum.example.com/t/sitekit-.../364
posts_count: 27
pulled_at: 2026-06-10T11:34:00Z
---

## Post 1 · alice · 2026-03-24

[raw markdown of post 1]

---

## Post 2 · bob · 2026-03-25

[raw markdown of post 2]
```

A full-thread file is a read-only snapshot. `dsc topic push` still operates on the OP only and does not consume the full-thread format.

Useful for archiving long discussions, feeding a complete conversation to an LLM, or producing a human-readable export.

## dsc topic push

```text
dsc topic push [OPTIONS] <discourse> <topic-id> <local-path>
```

Pushes the local Markdown file up to the specified topic, updating its first post with the file contents. Any leading YAML front matter is stripped before sending, so a file annotated with a `---` block (or one carried over from `category pull`) pushes a clean body.

Flags:

- `-n`, `--dry-run` — describe the edit without sending it.
- `--no-bump` — update the post without bumping the topic to the top of the activity feed (sends `post[no_bump]=true`). Use for silent maintenance edits.
- `--skip-revision` — update without recording an edit-history revision (sends `post[skip_revision]=true`). Suppresses the online audit trail; use sparingly.

## dsc topic sync

```
dsc topic sync <discourse> <topic-id> <local-path> [--yes]
```

Intelligently syncs the topic with the local Markdown file, using the most recently modified version as the source of truth.

Timestamps of both copies are shown before proceeding. Pass `--yes` (or `-y`) to skip the confirmation prompt.

## dsc topic reply

```text
dsc topic reply <discourse> <topic-id> [<local-path>] [--format text|json|yaml]
```

Posts a new reply at the end of the topic. Reads from `<local-path>` if given, otherwise from stdin (equivalent to passing `-`). `--format json` emits `{"topic_id": ..., "post_id": ...}` for scripting. Honours `-n` / `--dry-run`, which prints a `[dry-run] … would reply …` preview without posting.

Examples:

```bash
dsc topic reply myforum 1525 ./note.md
git log --since=yesterday --oneline | dsc topic reply myforum 1525
```

## dsc topic new

```text
dsc topic new <discourse> <category-id> --title <title> [<local-path>] [--format text|json|yaml]
```

Creates a new topic in the given category with the specified title. Reads the body from `<local-path>` if given, otherwise from stdin. `--format json` emits `{"topic_id": ..., "category_id": ...}` for scripting.

Examples:

```bash
dsc topic new myforum 42 --title "Release notes" ./notes.md
df -h | dsc topic new myforum 42 -t "Disk report $(date -I)"
```

## dsc topic delete

```text
dsc topic delete <discourse> <topic-id> [<topic-id>...] [--purge]
dsc topic rm     <discourse> <topic-id> [<topic-id>...] [--purge]
```

Deletes one or more topics by topic ID. By default this is a Discourse soft-delete, so staff can restore the topic from the trash. Supports global `-n` / `--dry-run`, which fetches each topic and prints the title/post count plus the planned delete without sending it.

`--purge` (alias `--permanent`) force-destroys a topic that is already in the trash. Discourse requires an administrator, the `can_permanently_delete` site setting, and either a different administrator from the one who soft-deleted the topic or a five-minute wait. Use only after confirming the topic has been archived or is genuinely disposable.

```bash
# Archive then remove from the forum
dsc topic pull myforum 1178 --full ./archive/topic-1178.md
dsc -n topic delete myforum 1178
dsc topic delete myforum 1178

# Batch delete
dsc topic rm myforum 1178 969

# Permanent deletion after the Discourse safety window
dsc topic delete myforum 1178 --purge
```

## dsc topic restore

```text
dsc topic restore <discourse> <topic-id>
```

Restores a soft-deleted topic via Discourse's topic recovery endpoint. Supports global `-n` / `--dry-run`.

```bash
dsc topic restore myforum 1178
```

## dsc topic list --deleted

```text
dsc topic list <discourse> --deleted [query] [--format text|json|yaml]
```

Lists soft-deleted topics visible to the configured staff/admin API key. This is the discovery path for `topic restore` when you do not remember the topic ID. Internally this paginates Discourse's deleted topic-list filter and verifies each result is actually deleted. Optional `query` terms are matched case-insensitively against title and slug; every term must match.

```bash
dsc topic list myforum --deleted
dsc topic list myforum --deleted "Postern Close"
dsc topic list myforum --deleted --format json
```

For general topic search, use `dsc search` directly.

## dsc topic tag

```text
dsc topic tag <discourse> <topic-id> <tag>
```

Adds the given tag to the specified topic, preserving any existing tags. No-op (and prints a message) if the topic already has the tag. Supports `--dry-run` (`-n`).

## dsc topic untag

```text
dsc topic untag <discourse> <topic-id> <tag>
```

Removes the given tag from the specified topic, leaving the others intact. No-op if the tag isn't present. Supports `--dry-run`.

```bash
# Tag every search hit with "triage"
dsc search myforum "status:open category:bugs" --format json \
  | jq -r '.[].id' \
  | xargs -I{} dsc topic tag myforum {} triage

# Preview removal without committing
dsc -n topic untag myforum 1525 archived
```

## dsc topic title

```text
dsc topic title <discourse> <topic-id> <title>
```

Renames a topic's title in place (`PUT /t/{id}.json`). Prints the old and new title, and - because the title drives the URL slug - a note when the topic URL changes. Supports `--dry-run`.

Useful after a `dsc category push` that created topics with slug-derived titles (e.g. `git-overview`):

```bash
dsc topic title myforum 723 "Get Git: an overview of our workflow"
# renamed topic 723: "git-overview" → "Get Git: an overview of our workflow"
# note: topic URL changed from /t/git-overview/723 to /t/get-git-an-overview-of-our-workflow/723
```

Discourse reserves some slugs (`contact`, `about`, …) as system routes; renaming a topic with a reserved slug returns a clear error instead of failing silently.

## dsc topic tags

```text
dsc topic tags <discourse> <topic-id> [<tag>...]
```

Sets a topic's **full** tag list atomically, replacing any existing tags (`PUT /t/{id}.json`). Pass no tags to clear all tags. This differs from `topic tag`/`topic untag`, which add or remove a single tag. Supports `--dry-run`.

```bash
# Initialise tags on a freshly-created topic
dsc topic tags myforum 723 git developer-guide

# Clear all tags
dsc topic tags myforum 723
```

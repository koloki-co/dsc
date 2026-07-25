# `dsc category` pull/push workflow — gaps + admonition/URL conversion + silent push

> **Status: Gaps 1–5 implemented (unreleased).**
> Surfaced from a real-world offline playbook sync workflow against
> `forum.rcpch.tech`. Gaps 1–3 affect the `category pull` / `category push`
> command pair. Gap 4 is content transformation for a single-source workflow.
> Gap 5 is notification/bump suppression for bulk migration edits.

Spec for five features in `dsc category pull` and `dsc category push`:

1. **`category pull` does not embed topic IDs** ✅ implemented
2. **`category push` ignores `--dry-run`** ✅ implemented
3. **`category push` silently creates new topics on slug mismatch** ✅ implemented
4. **No admonition/URL conversion on pull/push** ✅ implemented
5. **No `--no-bump` / `--skip-revision` for silent bulk edits** ✅ implemented

## Context: the real-world driver

`playbook.rcpch.tech` is being migrated to use `forum.rcpch.tech/c/playbook`
(category 34) as its canonical home. The workflow is:

1. `dsc category pull rcpch 34 discourse/` — snapshot all 27 topics to a
   Git-tracked local directory.
2. Edit files in `discourse/` and commit changes to Git.
3. `dsc category push rcpch 34 discourse/` — push edits back to Discourse.

The git history provides an offline audit trail. Discourse's built-in edit
revisions provide an online one. The two together give full provenance.

This workflow has a hard governance constraint: **no push to the forum without
human review of exactly what will change, and no accidental topic creation or
deletion.** Both of those requirements are blocked by gaps 1–3 below.

Tested against `forum.rcpch.tech` (Discourse stable, June 2026), category 34,
27 topics.

---

## Gap 1 — `category pull` does not embed topic IDs in output files

### What happens now

`category_pull` (in `src/commands/category.rs`) iterates over every topic in
the category, fetches the first post's raw content, and writes it to a file
named by `slugify(&topic.title)`:

```rust
// src/commands/category.rs  category_pull()
for topic in category.topic_list.topics {
    let topic_detail = client.fetch_topic(topic.id, true)?;
    let raw = topic_detail.post_stream.posts.get(0)
        .and_then(|p| p.raw.clone())
        .unwrap_or_default();
    let filename = format!("{}.md", slugify(&topic.title));
    write_markdown(&dir.join(filename), &raw)?;
}
```

At this point the code has `topic.id` in scope, but writes only the raw
Markdown body. The mapping from local file → Discourse topic ID is lost the
moment the file is written.

### Why this matters

`category_push` matches local files to remote topics using
`find_topic_match()` (same file):

```rust
fn find_topic_match<'a>(
    topics: &'a [TopicSummary],
    title: &str,
    path: &Path,
) -> Option<&'a TopicSummary> {
    let slug = slugify(title);
    topics.iter().find(|topic| {
        topic.slug == slug
            || topic.title.eq_ignore_ascii_case(title)
            || path.file_stem()
                .map(|s| s.to_string_lossy().eq_ignore_ascii_case(&topic.slug))
                .unwrap_or(false)
    })
}
```

This relies entirely on the title or filename continuing to match the remote
topic's slug. If **any** of the following happens, the match fails silently
and a **new duplicate topic is created**:

- The topic's title is edited locally (slug changes)
- `slugify()` produces a different result than Discourse's own slugifier for
  edge cases (accented characters, punctuation, very long titles)
- A file is renamed for organisational reasons
- The topic's slug is changed directly in Discourse

Gap 3 describes the silent-create consequence; the root fix is here in Gap 1.

### Metadata format: YAML front matter (stripped before push)

Pulled files get standard YAML front matter (`---` fences). Discourse does not
"know" about front matter — if a file were pasted manually into a Discourse
topic, the `---` lines would render as horizontal rules and the YAML would
appear as plain text. This is not a problem in practice because all Discourse
writes go via `dsc`, which calls `strip_frontmatter()` before sending content
to the API. The metadata is local-only and never reaches the published post.

```markdown
---
title: Dependency management
topic_id: 412
url: https://forum.rcpch.tech/t/dependency-management/412
pulled_at: 2026-06-22T09:19:00Z
---

[raw markdown body follows, unchanged — existing HTML comment headers preserved]
```

The existing HTML comment blocks (e.g. `<!-- Authors: ...\nOrigin: ... -->`)
in topics are part of the raw body from Discourse and are preserved unchanged
below the YAML front matter. They remain invisible when rendered in Discourse
(HTML comments are stripped) and in MkDocs (same). `authors` and `origin` are
not added to the YAML front matter by `category pull` — those fields live in
the existing HTML comment convention already established in these files. They
can be added manually by a human editor and will be preserved on re-pull
(since the YAML front matter block is overwritten but the HTML comment body is
left as-is from the remote).

### Reference: API calls observed in the field

Category topic list (already used by `category_pull`):

```
GET /c/playbook/34.json
Api-Key: <redacted>
Api-Username: pacharanero

→ 200 OK
{
  "topic_list": {
    "topics": [
      { "id": 394, "title": "About this Playbook", "slug": "about-this-playbook" },
      { "id": 412, "title": "Dependency management", "slug": "dependency-management" },
      ...
    ]
  }
}
```

The `id` field is present on every `TopicSummary` in the list response and is
already modelled in `TopicSummary` (`src/api/models.rs`). No new API calls are
needed — the fix is purely about propagating data that is already fetched.

---

## Gap 2 — `category push` ignores `--dry-run`

### What happens now

The global `--dry-run` / `-n` flag is parsed at the top level in
`src/main.rs`:

```rust
// src/main.rs (line ~43)
let dry_run = cli.dry_run;
```

But the call site for `category_push` omits the argument:

```rust
// src/main.rs (line ~226)
} => commands::category::category_push(&config, &discourse, &category, &local_path),
//                                                                                 ^
//                                                   dry_run is NOT passed here
```

Compare with `topic push`, which correctly passes `dry_run`:

```rust
// src/main.rs (line ~144)
} => commands::topic::topic_push(&config, &discourse, topic_id, &local_path, dry_run),
```

And `category_push`'s function signature has no `dry_run` parameter at all:

```rust
// src/commands/category.rs (line ~144)
pub fn category_push(
    config: &Config,
    discourse_name: &str,
    category: &str,
    local_path: &Path,
    // dry_run: bool  ← missing
) -> Result<()>
```

### Why this matters

The governance constraint for this workflow is: **always dry-run first,
review the plan, then execute.** Without a working `--dry-run`, there is no
safe preview step. Running `dsc category push --dry-run` today silently
executes a live push. Users who read `dsc --help` and see `--dry-run` listed
as a global flag have no indication it is not honoured by this subcommand.

### What is needed

1. Add `dry_run: bool` to `category_push()`'s signature.
2. Pass `dry_run` from the call site in `main.rs`.
3. In the push loop, gate all mutating operations on `!dry_run`:

```rust
if dry_run {
    if let Some(topic) = find_topic_match(&topics, &title, &path) {
        let url = format!("{}/t/{}/{}", base_url, topic.slug, topic.id);
        println!("[dry-run] ~ would update topic {} \"{}\" ({}) with {} bytes",
            topic.id, title, url, raw.len());
    } else {
        println!("[dry-run] + would create new topic \"{}\" ({} bytes) in category {}",
            title, raw.len(), category_id);
    }
} else {
    // existing update / create logic
}
```

The `~` (change) / `+` (create) / `=` (unchanged) sigils are already used
elsewhere in `dsc` dry-run output (e.g. `setting push --dry-run`) — use the
same convention for consistency.

Optionally, for `=` (unchanged): compare the local body against the fetched
remote body and emit `=` if they are byte-identical. This avoids
no-op API writes and makes the dry-run output meaningful even when nothing
has changed.

---

## Gap 3 — `category push` silently creates new topics on slug mismatch

### What happens now

When `find_topic_match()` returns `None`, `category_push` immediately creates
a new topic:

```rust
// src/commands/category.rs  category_push()
if let Some(topic) = find_topic_match(&topics, &title, &path) {
    // update existing topic
    client.update_post(post.id, &raw)?;
} else {
    // ← no warning, no --no-create guard, just creates
    let topic_id = client.create_topic(category_id, &title, &raw)?;
    topics.push(TopicSummary { id: topic_id, title: title.clone(), slug: slugify(&title) });
}
```

There is no flag to suppress this behaviour, no warning to the operator, and
no dry-run output (see Gap 2). The only signal of creation is the absence of
an error.

### Why this matters

Accidental topic creation is hard to undo cleanly. Discourse does not expose
a delete-topic API to regular API clients; topics must be archived/unlisted
rather than deleted. In a curated category like a Playbook, orphaned duplicate
topics pollute the index and confuse readers. The governance rule for this
workflow is that **no new topics should ever be created without deliberate
human intent**.

This gap is primarily addressed by Gap 1 (with `topic_id` in the `<!--dsc-meta`
block, the slug match is no longer needed for known topics). But an explicit
guard is still valuable for cases where a completely new file is introduced.

### What is needed

Add an `--updates-only` flag to `dsc category push`:

```text
dsc category push [OPTIONS] <DISCOURSE> <CATEGORY> <LOCAL_PATH>

Options:
  --updates-only   Only update existing topics; error if a local file has no
                   remote match instead of creating a new topic
  -n, --dry-run    ...
```

When `--updates-only` is set and neither `<!--dsc-meta topic_id` nor
`find_topic_match()` resolves, emit a clear error:

```
error: no matching topic found for "my-new-file.md" (title: "My New File")
hint: remove --updates-only to allow new topic creation, or check the filename matches an existing topic slug
```

The default behaviour (create on mismatch) is preserved so existing
workflows are not broken.

---

## Gap 4 — No admonition/URL conversion on pull/push

### Background

The playbook workflow uses a single folder of Markdown files that feeds both
Discourse (via `dsc category push`) and a Zensical/MkDocs static site. The
two platforms have incompatible conventions for two common patterns:

**Admonitions:**
- MkDocs: `!!! note "Title"\n    Content` (pymdownx admonition syntax)
- Discourse: no native admonition syntax; use blockquotes with bold lead-ins

**Internal cross-links:**
- MkDocs: relative file paths — `[see versioning](../versioning.md)`
- Discourse: full forum URLs — `[see versioning](https://forum.rcpch.tech/t/versioning/NNN)`

Currently these conversions are done manually, which is error-prone and
creates friction when content moves between platforms.

### What is needed

Two optional conversions on `dsc category push` and `dsc category pull`.
They are opt-in; without them the raw Markdown is preserved exactly.

#### `--convert-admonitions <style>`

Convert MkDocs/Zensical admonitions to a chosen Discourse representation on
**push**, and reverse only that representation on **pull**:

```text
dsc category push forum 34 ./playbook --convert-admonitions=quote-callouts
dsc category pull forum 34 ./playbook --convert-admonitions=plain-blockquote
```

`quote-callouts` is the recommended target where the [Quote
Callouts](https://meta.discourse.org/t/quote-callouts/350962) theme component
is installed and attached to the forum's active theme. It maps directly onto
the component's Obsidian-style source syntax, retaining the authored type and
custom title:

```markdown
# Input (MkDocs/Zensical)
!!! warning "Protect production"
    Take a backup before changing this setting.

# Output (`quote-callouts`)
> [!warning] Protect production
> Take a backup before changing this setting.
```

The component supports the useful MkDocs types directly (`note`, `abstract`,
`info`, `todo`, `tip`, `success`, `question`, `warning`, `failure`, `danger`,
`bug`, `example`, and `quote`) and custom configured types. Unknown types are
kept in the raw Markdown; the component applies its configured fallback
appearance.

Foldable MkDocs variants round-trip too:

| MkDocs/Zensical | Quote Callouts |
|---|---|
| `!!! note "Title"` | `> [!note] Title` |
| `??? warning "Title"` | `> [!warning]- Title` |
| `???+ tip "Title"` | `> [!tip]+ Title` |

The converter handles nested admonitions and leaves fenced code blocks alone.
On pull it only recognises the precise `[!type]` form, so ordinary blockquotes
remain ordinary blockquotes.

`plain-blockquote` is the component-free, email-safe fallback. It writes a
specific bold emoji lead-in, for example:

```markdown
> **⚠️ Warning — Protect production**
> Take a backup before changing this setting.
```

Only this precise form is reversed on pull. It canonicalises aliases such as
`info` → `note` and `caution` → `warning`, so use `quote-callouts` where
preserving the authored type matters.

`dsc` does not try to detect the Quote Callouts component: choosing
`quote-callouts` explicitly is the operator's acknowledgement that it is
deployed. Without the component, the post is still a readable ordinary
blockquote. It is a theme component, not a server plugin, so email
notifications do not receive its visual styling; email readers see the
underlying quote content.

#### `--rewrite-links` flag

On **push** (MkDocs → Discourse): rewrite relative Markdown links to full
forum URLs. Requires a resolved map of `{filename_stem}` → `{topic_id, slug}`
from the front matter in the same directory (or a fresh category listing).

```markdown
# Input
[See versioning](../versioning.md)

# Output
[See versioning](https://forum.rcpch.tech/t/versioning/NNN)
```

Algorithm:
1. Scan body for `[text](path)` where `path` does not start with `http` and ends in `.md`.
2. Derive the stem: `path.rsplit('/').last().strip_suffix(".md")`.
3. Look up the stem in the local `topic_id` map.
4. If found, rewrite the URL. If not found, emit a warning (do not silently drop the link).

On **pull** (Discourse → MkDocs): rewrite full `forum.rcpch.tech/t/…` URLs
back to the flat export's `filename.md` paths. This is best-effort; links to
non-playbook topics (e.g., sysadmin topics) are left as full URLs.

### Priority

Both directions of admonition conversion and link rewriting are implemented.

### Backward compatibility

All conversion flags are opt-in. Default push/pull behaviour is unchanged.

---

## Gap 5 — No `--no-bump` / `--skip-revision` for silent bulk edits

### Background: Discourse notification behaviour for edits

When `dsc category push` updates an existing topic's first post via
`PUT /posts/{id}.json`, Discourse does **not** send inbox notifications to
topic watchers or trackers. Notifications are only triggered by new replies
and new topics. So bulk editing via `dsc` does not spam anyone's notification
inbox.

However, edited topics are **bumped to the top of the category activity feed**
by default (Discourse orders topics by `last_posted_at` / `bumped_at`). For a
bulk migration push of 20+ topics, this causes the entire category to
re-sort — visually noisy for anyone browsing the category at the time.

**Current `dsc` behaviour:** `update_post()` sends only `post[raw]`. It does
not send `no_bump` or `skip_revision`, so every push bumps the topic and
creates a revision entry.

### Practical guidance (no `dsc` change needed for now)

The Playbook category (`forum.rcpch.tech/c/playbook/34`) is currently
**private**. The bulk migration edits should all be done before the category
is made public. When the category is private, the bump behaviour is invisible
to non-members, and the team can choose to mute the category in their own
notification preferences during the migration window if desired.

Once the category is public and you want to do **quiet maintenance edits**
(correcting typos, updating links, etc.) without churning the activity feed,
`--no-bump` becomes important.

### What is needed

Add a `--no-bump` flag to `dsc topic push` and `dsc category push`:

```text
dsc topic push [OPTIONS] <DISCOURSE> <TOPIC_ID> <LOCAL_PATH>
dsc category push [OPTIONS] <DISCOURSE> <CATEGORY> <LOCAL_PATH>

Options:
  --no-bump   Update post content without bumping the topic in the
              activity feed. Passes no_bump=true to the API.
              Use for silent maintenance edits.
```

Implementation: add `"no_bump"` → `"true"` to the form payload in
`update_post()` when the flag is set:

```rust
// src/api/topics.rs  update_post()
pub fn update_post(&self, post_id: u64, raw: &str, no_bump: bool) -> Result<()> {
    let path = format!("/posts/{}.json", post_id);
    let no_bump_str = no_bump.to_string();
    let mut payload = vec![("post[raw]", raw)];
    if no_bump {
        payload.push(("post[no_bump]", no_bump_str.as_str()));
    }
    // ...
}
```

Optionally, add `--skip-revision` as a companion flag (passes
`post[skip_revision]=true`) to suppress edit history entries during bulk
migration. This is a stronger "silence" but prevents the revision trail from
being useful — consider whether you want it. For the playbook migration,
**don't** use `--skip-revision` (Discourse revision history is part of the
audit trail).

### Reference: API field names observed

From the Discourse source and Meta documentation:
- `post[no_bump]` = `"true"` — prevents topic bump on post edit
- `post[skip_revision]` = `"true"` — prevents new revision entry
- Neither is currently sent by `dsc`

---

## Implementation order

### Phase 1 — `category pull` embeds YAML front matter (Gap 1, pull side) ✅

- [x] Add `strip_frontmatter(raw: &str) -> (HashMap<String, String>, String)` helper to `src/utils.rs`.
- [x] Move `current_utc_iso8601()` and `yaml_scalar()` from `src/commands/topic.rs` to `src/utils.rs` and share.
- [x] Update `category_pull()` to call `render_category_topic()` which prepends YAML front matter (`title`, `topic_id`, `url`, `pulled_at`) before writing each file.

### Phase 2 — `category push` routes by front-matter `topic_id` (Gap 1, push side) ✅

- [x] In `category_push()`, call `strip_frontmatter()` on each file to separate metadata from body.
- [x] If `topic_id` is present, use it directly to route the update (skip `find_topic_match()`).
- [x] Pass only the stripped body to `client.update_post()` and `client.create_topic()`.
- [x] `find_topic_match()` retained as fallback for files without front matter.
- [x] `topic push` also strips front matter before sending.

### Phase 3 — working `--dry-run` for `category push` (Gap 2) ✅

- [x] `dry_run: bool` parameter added to `category_push()`.
- [x] `dry_run` passed at the call site in `src/main.rs`.
- [x] Dry-run output uses `~` / `+` / `=` sigils; no-op writes skipped when body is byte-identical to remote.

### Phase 4 — `--updates-only` guard (Gap 3) ✅

- [x] `updates_only: bool` parameter added to `category_push()` and wired from CLI.
- [x] When `updates_only` is true and no match is found, a structured error is emitted instead of calling `create_topic()`.

### Phase 5 — admonition/URL conversion (Gap 4) ✅

- [x] Add `--convert-admonitions <quote-callouts|plain-blockquote>` to `category push` and `category pull`.
- [x] Implement MkDocs/Zensical admonition → selected Discourse representation on push, including nested and foldable forms while preserving fenced code blocks.
- [x] Implement selected Discourse representation → MkDocs/Zensical best-effort reversal on pull, matching only the specific generated syntax.
- [x] Add `--rewrite-links` flag to `category push` and `category pull`.
- [x] Implement relative-path → full-forum-URL rewriting on push, resolving the final filename stem through local front matter and the fresh category listing; unresolved targets warn without changing the link.
- [x] Implement full-forum-URL → flat local `filename.md` best-effort reversal on pull, preserving URL fragments and leaving external/out-of-category links unchanged.

### Phase 6 — `--no-bump` and `--skip-revision` flags (Gap 5)

- [x] Add a `PostEditOptions { no_bump, skip_revision }` parameter to `update_post()` in `src/api/topics.rs` (payload built by the unit-tested `post_edit_payload()`).
- [x] Wire `--no-bump` flag through `topic push`, `category push` CLI → `topic_push()`/`category_push()` → `update_post()`.
- [x] Wire `--skip-revision` flag the same way (optional companion; help text notes it suppresses Discourse revision history).
- [x] Document in `dsc topic push --help`: "use `--no-bump` for silent maintenance edits to avoid churning the activity feed." — `strip_frontmatter()`
  returns an empty map and the full file content, and `find_topic_match()` is
  used as before. This covers files edited before this feature shipped and
  files added manually without a pull.
- Default `category push` behaviour (create on mismatch) is unchanged unless
  `--updates-only` is explicitly set.
- Conversion flags (`--convert-admonitions`, `--rewrite-links`) are opt-in.
- `topic pull` (non-`--full`) is not changed.

## Out of scope

- Title editing via `category push`: changing a Discourse topic's title
  requires `PUT /t/{slug}/{id}.json` with `title` and is not in scope here.
- Topic deletion: explicitly out of scope. `dsc` should never delete topics
  in a category push workflow.
- Converting Discourse-only markup (e.g. `[quote]` BBCode, `@mentions`,
  Discourse-specific emoji) to MkDocs — these are best left as-is or
  handled by the human editor.

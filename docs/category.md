# dsc category

List, pull, push, and copy categories, plus sync category *definitions*.

Two distinct kinds of sync live under `dsc category`:

- **Topic content** — `pull`/`push` move the Markdown *topics inside* a category.
- **Category definitions** — `def pull`/`def push` and `show`/`get`/`set` manage the category objects themselves (name, colour, permissions, description, topic template, tag rules, ordering). This is the config-as-code counterpart to `dsc tag pull/push` and `dsc setting pull/push`.

## dsc category list

```
dsc category list <discourse> [--format text|json|yaml] [--tree] [--verbose]
```

Lists all categories with their IDs and names.

Flags:

- `--tree` — print categories in a hierarchy, with subcategories indented under parents.
- `-v`, `--verbose` — include additional fields where supported.

## dsc category pull

```
dsc category pull <discourse> <category-id-or-slug> [<local-path>] [--convert-admonitions <style>] [--rewrite-links]
```

Pulls the category into a directory of Markdown files. If `<local-path>` is omitted, writes to a new folder in the current directory (named from the category slug/name). Files are named from topic titles.

Each file gets YAML front matter binding it to its remote topic:

```markdown
---
title: Dependency management
topic_id: 412
url: https://forum.example.com/t/dependency-management/412
pulled_at: 2026-06-22T09:19:00Z
---

[the topic's first post follows here as Markdown]
```

`category push` reads `topic_id` from this block to route updates by ID, so renaming the file or editing the title no longer risks creating a duplicate topic. The front matter is local-only metadata: `dsc` strips it before sending content to Discourse, so the `---` block never reaches the published post. Files without front matter (e.g. ones you author by hand) still work - they fall back to slug/title matching.

- `--rewrite-links` — rewrite same-forum links to pulled category topics as local `filename.md` paths. The category export is flat, so it cannot reproduce an original parent-directory layout.

## dsc category push

```text
dsc category push [OPTIONS] <discourse> <category-id-or-slug> <local-path>
```

Pushes local Markdown files up to the category, updating existing topics and (by default) creating new ones for files with no remote match. Files are matched by `topic_id` front matter first, falling back to slug/title matching.

The push prints a plan with a sigil per file: `~` (update), `+` (create), `=` (unchanged, skipped). Bodies byte-identical to the remote post (ignoring trailing whitespace) are reported `=` and not re-sent.

Flags:

- `-n`, `--dry-run` — print the plan without writing anything. Run this first and review before pushing for real.
- `--updates-only` — only update existing topics; error with a hint instead of silently creating a new topic when a file has no remote match. Use for curated categories where accidental topic creation must be impossible.
- `--no-bump` — update posts without bumping their topics to the top of the category activity feed (sends `post[no_bump]=true`). Use for silent bulk maintenance edits.
- `--skip-revision` — update posts without recording an edit-history revision (sends `post[skip_revision]=true`). Suppresses the online audit trail; use sparingly.
- `-a`, `--convert-admonitions <quote-callouts|plain-blockquote>` — convert MkDocs/Zensical admonitions while pushing, or the selected generated form back while pulling. Omit it to preserve raw Markdown.
- `--rewrite-links` — rewrite relative `.md` links such as `../versioning.md` to matching same-category forum topic URLs. Unresolved local targets are left intact and reported as warnings on stderr.

### Admonition conversion

Use `quote-callouts` when the [Quote Callouts](https://meta.discourse.org/t/quote-callouts/350962) theme component is installed and attached to the forum's active theme:

```text
dsc category push forum 34 forum-export/ --convert-admonitions=quote-callouts
dsc category pull forum 34 forum-export/ --convert-admonitions=quote-callouts
```

It converts `!!! warning "Title"` to `> [!warning] Title` (and reverses it), preserves supported/custom callout types, and carries MkDocs `???` / `???+` folding to Quote Callouts `-` / `+`. It does not transform fenced code examples or ordinary blockquotes.

Choose `plain-blockquote` for a component-free target:

```text
dsc category push forum 34 forum-export/ --convert-admonitions=plain-blockquote
```

This generates a readable bold emoji lead-in inside a normal quote. It is the safer choice for email-heavy forums: Quote Callouts is browser-side theme styling, so email notifications contain the underlying quote rather than a styled callout. `dsc` does not detect component installation; selecting `quote-callouts` is an explicit deployment prerequisite.

### Link rewriting

Use `--rewrite-links` to switch between a flat local category export and its same-forum topic URLs:

```text
dsc category pull forum 34 forum-export/ --rewrite-links
dsc category push forum 34 forum-export/ --rewrite-links
```

On pull, `https://forum.example.com/t/versioning/42#details` becomes `versioning.md#details` when topic 42 is in the pulled category. Links to another forum or to a topic outside the category stay as full URLs.

On push, any relative `.md` path is resolved by its final filename stem, so `../versioning.md` and `versioning.md` both resolve to the local `versioning.md` file's `topic_id` front matter and then to the current remote topic slug. Links with no matching category file are preserved and reported as warnings. Fenced code examples and image destinations are not rewritten.

### Recommended workflow

```text
dsc category pull forum 34 forum-export/      # snapshot (front matter embedded)
# edit files in forum-export/, commit to git for an offline audit trail
dsc category push -n forum 34 forum-export/   # review the plan
dsc category push forum 34 forum-export/      # apply
```

## dsc category copy

```
dsc category copy <source-discourse> <category-id-or-slug> [--target <target-discourse>]
```

Copies the specified category. If `--target` is omitted, copies within the same Discourse.

- The copied category name is set to `Copy of <original category name>`.
- The copied category slug is suffixed with `-copy` (e.g., `staff` -> `staff-copy`).
- All other fields match the source, except the ID which is assigned by Discourse.

`<category-id-or-slug>` can be found using `dsc category list`.

## Category definitions

Version-control a forum's category structure the way `tag pull/push` handles the tag taxonomy.

```
dsc category def pull <discourse> [categories.yaml]   # snapshot every definition to a file
dsc category def push <discourse> <categories.yaml>   # apply the file (upsert; never deletes)
```

- `def pull` writes one `categories.yaml` (or `.json` by extension) holding every category's definition - name, slug, colour, position, parent, `read_restricted`, description, topic template, permissions, tag rules, and display knobs. Usage counts and other volatile fields are dropped so re-pulls diff cleanly.
- `def push` reconciles the server toward the file: it **creates** missing categories and **updates** changed ones, matching by `id` (stable), then `slug`, then `name`. It never deletes. `--dry-run` prints the plan with `+` (create), `~` (update), `=` (unchanged) sigils and names each field that would change. A file entry with no `id` that matches nothing is flagged loudly - it would create a new category, so if you meant to rename an existing one, keep its `id` in the file to preserve its topics, or use `dsc category rename` below.
- The file schema is strict: unknown top-level or category fields are errors rather than silently ignored. This protects declarative configuration from misspellings and unsupported settings.
- The push is idempotent: a pull followed by a push with no edits reports every category `= unchanged`. Description terminal line endings are normalised because Discourse removes them from category descriptions.

### Single-field access

For a quick one-field read or edit without rewriting the whole file - mirrors `dsc setting get/set` and `dsc theme show`:

```
dsc category show <discourse> <category>            # all definition fields
dsc category get  <discourse> <category> <field>    # one field
dsc category set  <discourse> <category> <field> <value>
```

- `<category>` resolves by `id`, `slug`, or `name`.
- `<field>` is one of: `name`, `slug`, `color`, `text_color`, `position`, `parent`, `read_restricted`, `description`, `topic_template`, `permissions`, `allowed_tags`, `allowed_tag_groups`, `minimum_required_tags`, `sort_order`, `default_view`, `subcategory_list_style`, `num_featured_topics`, `show_subcategory_list`.
- List fields (`allowed_tags`, `allowed_tag_groups`) take a comma-separated value; an empty value clears the list. Pass `--append` or `--remove` (mutually exclusive) to add or remove the given comma-separated items instead of replacing the whole list - e.g. `dsc category set forum1 support allowed_tags urgent,bug --append`.
- `permissions` takes `group:level,...` where level is `full`, `create_post`, or `readonly` (e.g. `staff:full`). Granting any group other than `everyone` also sets `read_restricted=true`, matching the admin UI.
- `show`/`get` honour `--format text|json|yaml`; `set` honours the global `--dry-run`.

Notes:

- `description` is read from the plain-text form; on write, Discourse re-cooks it as the category's "About" topic excerpt (settles a moment after a create).
- When `def push` creates a category whose `parent` is itself brand-new in the same file, run the push twice (or create the parent first) - a parent is resolved against categories that already exist on the server.

## dsc category diff

Compare the stable, editable definition fields of two explicitly selected live categories:

```bash
dsc category diff forum-a general forum-b announcements
dsc category diff forum-a 12 forum-b 37 --format json
```

```text
dsc category diff <discourse-a> <category-a> <discourse-b> <category-b>
```

Each category resolves independently by `id`, `slug`, or `name`, so the categories may have different names or slugs. The diff reports only changed definition fields from the same surface as `category show/get/set`; it excludes server-specific IDs and volatile usage counts. `--format json|yaml` preserves booleans, numbers, arrays, and permission maps as native values rather than display strings.

Aliases: `df`.

## dsc category rename

Rename a category, preserving its topics. Uses a safe `PUT /categories/{id}.json` by the resolved category id - the recommended path when a `categories.yaml` entry has no `id`, since `def push` can't tell a rename from a delete+create in that case (see the note above).

```bash
dsc category rename myforum old-name new-name        # rename
dsc -n category rename myforum old-name new-name     # dry-run
```

`<category>` resolves by `id`, `slug`, or `name`, same as `show`/`get`/`set`.

Aliases: `rn`.

Refuses to run when:

- the category does not resolve (bad id/slug/name),
- a different category already has the new name,
- old and new names are identical after trimming,
- the new name is empty after trimming.

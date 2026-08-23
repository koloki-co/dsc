# dsc search

Search topics on a Discourse install.

```text
dsc search <discourse> <query> [--format text|json|yaml]
dsc search all <query> [--tags <tag1,tag2,...>] [--format text|json|yaml]
```

Hits `/search.json?q=…` and prints the matching topics. The query is passed through verbatim, so any Discourse search filter syntax works (`status:open`, `category:foo`, `tags:bug`, `@user`, etc.).

Default text output is one topic per line, ID first — easy to pipe into `awk` or `cut`:

```bash
dsc search myforum "release notes"
# 1525  Daily bookmarks
#  789  Release notes — March 2026

dsc search myforum "release notes" | awk '{print $1}'   # IDs only
dsc search myforum "release notes" --format json        # full structured output
```

Each result includes `id`, `title`, `slug`, `posts_count`, `category_id`, and `tags`.

## Search every forum

`dsc search all <query>` runs the same query against every configured forum and combines the results. Use `--tags` to search only forums matching any comma- or semicolon-separated tag, case-insensitively. Text output prefixes each row with the forum name; JSON and YAML add a `forum` field to each normal search result. The command continues after an individual forum fails so successful results still reach stdout, reports failures on stderr, and exits non-zero if any forum could not be searched. An empty tag filter is rejected rather than searching the entire fleet.

```bash
dsc search all "tags:bug status:open"
dsc search all "tags:bug status:open" --tags production
dsc search all "release notes" --format json | jq -r '.[] | [.forum, .id] | @tsv'
```

`all` is currently a reserved positional selector, so a configured forum named `all` cannot be addressed by this command. Fleet selector normalization is tracked in the roadmap.

## Examples

```bash
# Find all open topics tagged "bug"
dsc search myforum "tags:bug status:open"

# Find recent posts by a specific user
dsc search myforum "@alice after:2026-01-01"

# Pull every match into Markdown
dsc search myforum "release notes" --format json \
  | jq -r '.[].id' \
  | xargs -I{} dsc topic pull myforum {}
```

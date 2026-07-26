# dsc tag

Manage the tag taxonomy (tags and tag groups) as a version-controlled file. For applying/removing a tag on a specific topic, see [`dsc topic tag/untag`](topic.md#dsc-topic-tag).

## dsc tag list

```text
dsc tag list <discourse> [--format text|json|yaml]
```

Lists all tags visible to the authenticated user, with the topic count beside each. Default text output is two columns (tag name, count) and is sortable/cuttable.

## dsc tag pull

```text
dsc tag pull <discourse> [path]
```

Serialises the full tag taxonomy (tags and tag groups) to a single file. Default path: `tags.yaml`. Format is inferred from the file extension (`.yaml`/`.yml` or `.json`).

Only definitions are emitted — usage counts and read-only fields are excluded so repeated pulls stay diff-clean.

Tag groups require an admin API key. If the key lacks admin scope, groups are omitted with a warning.

```bash
dsc tag pull myforum
dsc tag pull myforum tags.json
```

### File schema (version 1)

```yaml
version: 1

tags:
  - name: covers
    description: Topics with cover images

tag_groups:
  - name: Role
    one_per_topic: false
    parent_tag: null
    permissions: # group names; levels: full | create_post | readonly
      everyone: full
    tags:
      - guitarist
      - bassist
```

## dsc tag push

```text
dsc tag push <discourse> <path> [--prune] [--dry-run] [--yes]
```

Reads a taxonomy file and reconciles server state toward it.

- **Default (upsert)**: creates missing tags/groups, updates changed ones, never deletes.
- **`--prune`**: additionally deletes tags and tag groups present on the server but absent from the file. Because deleting the wrong tags is destructive, `--prune` requires a *complete* taxonomy snapshot - one pulled with an admin key, so tag groups were readable - and refuses to run otherwise. When the plan contains any deletion, the push also requires `--yes` after you review it with `--dry-run`.
- Idempotent: a push with no file change is a no-op.
- Supports `--dry-run` (`-n`) to preview the plan without sending writes.
- `--yes`: confirm a plan that deletes tags or groups (see `--prune`). Not needed for upsert-only plans.

Tag groups require an admin API key. If not accessible, group reconciliation is skipped with a warning. Permissions in the file use group names; `dsc` resolves the numeric group IDs required by the Discourse API.

```bash
dsc -n tag push myforum tags.yaml          # dry-run: show plan
dsc tag push myforum tags.yaml             # apply upserts
dsc tag push myforum tags.yaml --prune --yes  # full sync (deletes extras; --yes confirms)
```

## dsc tag rename

Rename a tag, preserving every topic association. Discourse rewrites topic tag lists in-place rather than dropping and re-adding, so this is the safe alternative to editing a tag name in `tags.yaml` and running `dsc tag push` (which sees the old name as removed and the new name as added).

```bash
dsc tag rename myforum old-name new-name        # rename
dsc -n tag rename myforum old-name new-name     # dry-run
```

Aliases: `rn`.

Refuses to run when:

- the old tag does not exist on the server,
- a tag with the new name already exists (merging is not supported),
- the new name contains whitespace (Discourse tags are slug-style),
- old and new names are identical after trimming.

## Notes

- **Renames**: prefer `dsc tag rename` over editing the name in a pulled file. A name change in the file is indistinguishable from delete + create, which drops topic associations.
- Tag groups are admin-only; `pull` and `push` degrade gracefully when using a non-admin key.
- The desired tag set on push = the union of every `tags[].name` and every name listed under any group's `tags:`.

## Examples

```bash
# Pull, edit, push workflow
dsc tag pull myforum
$EDITOR tags.yaml
dsc -n tag push myforum tags.yaml   # preview
dsc tag push myforum tags.yaml      # apply

# Rename a tag preserving topic membership
dsc -n tag rename myforum bug defect    # preview
dsc tag rename myforum bug defect       # apply
```

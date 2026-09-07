# dsc board

List, show, and snapshot Discourse Boards - the official kanban plugin (core, Business/Enterprise plans, shipped September 2026). This is a read-only surface: `dsc` never creates, updates, or deletes boards, columns, or cards. A guarded `board push` is a later phase.

Boards live behind `/boards/api/*`, an undocumented plugin API. A 404 from any `dsc board` command can mean Boards is disabled, unlicensed on the current plan, unavailable on this Discourse version, or the board does not exist.

## List boards

```text
dsc board list <discourse> [--format text|json|yaml]
```

One row per board accessible to the configured API user: id, name, slug, ACL summary (`read`/`write`/`manage`, plus `anon-read` when set), and its tag/category constraints.

```bash
dsc board list myforum
dsc board list myforum --format json
```

## Show a board

```text
dsc board show <discourse> <board-id> [--format text|json|yaml]
```

Prints the board's metadata followed by every column and its cards. A topic card shows the referenced topic's title and `topic_id`; a floater card shows its own title and notes (floater cards have no topic behind them).

```bash
dsc board show myforum 3
dsc board show myforum 3 --format yaml
```

## Snapshot a board

```text
dsc board pull <discourse> <board-id> [<file>] [--force]
```

Writes a stable snapshot of the entire board - including floater cards - to a local file: YAML by default (`board.yaml`), JSON when the path ends `.json`. Floaters have no topic behind them, so omitting them from the snapshot would misrepresent the board and would make a later `push` unable to restore it. Card order within each column is the file's list order; the server's own `position` values are large, opaque integer gaps that are never meaningful across a pull/push round trip, so they are not persisted.

Refuses to overwrite an existing file unless `--force` is given.

```bash
dsc board pull myforum 3 roadmap.yaml
dsc board pull myforum 3 roadmap.json
dsc board pull myforum 3 roadmap.yaml --force
```

### File schema (version 1)

```yaml
version: 1
board_id: 3
board_name: Roadmap
discourse_version: 2026.9.0
pulled_at: "2026-09-07T02:00:00Z"
board:
  id: 3
  name: Roadmap
  slug: roadmap
  tag_names: [discourse]
  can_manage: true
  ...
columns:
  - id: 10
    title: Backlog
    color: 2f7ed8
    cards:
      - id: 100
        card_type: floater
        title: Write spec
        notes: draft
      - id: 101
        card_type: topic
        title: null
        topic_id: 1261
        topic:
          id: 1261
          title: Discuss roadmap
```

`board` and each column/card carry every field the server returned (unrecognised fields are preserved rather than dropped), so the file is safe to inspect even as the plugin evolves.

## Notes

- Auth is the standard configured `apikey`/`api_username`; the acting user needs the board ACL `view` to read.
- `push` (guarded, declarative writes) is a later phase - see `spec/commands/boards.md`.

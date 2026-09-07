# dsc board

List, show, and snapshot Discourse Boards - the official kanban plugin (core, Business/Enterprise plans, shipped September 2026). This phase exposes no explicit create, update, or delete commands. However, Discourse's board-detail GET endpoint performs its own board maintenance: it can create or delete tag-driven topic cards and records one board-view event per user per day. Consequently, `dsc board show` and `dsc board pull` can change server state and refuse to run under global `--dry-run`. A guarded `board push` is a later phase.

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

Prints the board's metadata followed by every column and its cards. A topic card shows the referenced topic's title and `topic_id`; a floater card shows its own title and notes (floater cards have no topic behind them). Fetching this detail invokes Discourse's normal board backfill and view-history behavior described above.

```bash
dsc board show myforum 3
dsc board show myforum 3 --format yaml
```

## Snapshot a board

```text
dsc board pull <discourse> <board-id> [<file>] [--force]
```

Writes a stable snapshot of the entire board visible to the configured API user - including floater cards - to a local file: YAML by default (`board.yaml`), JSON when the path ends `.json`. Floaters have no topic behind them, so omitting them from the snapshot would misrepresent the board and would make a later `push` unable to restore it. Card and column order is expressed by list order; the server's own `position` values are large, opaque integer gaps that are never meaningful across a pull/push round trip, so they are not persisted. Even when a column displays cards by recency, the snapshot canonicalizes them by persisted position to avoid activity-driven diff noise. Fetching the snapshot invokes Discourse's normal board backfill and view-history behavior described above.

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
  name: Roadmap
  slug: roadmap
  category_ids: []
  tag_names: [discourse]
  require_confirmation: false
  show_tags: true
  card_style: detailed
  show_topic_thumbnail: false
columns:
  - id: 10
    title: Backlog
    icon: null
    default_sort: priority
    tag_name: null
    move_to_category_id: null
    move_to_assigned: null
    move_to_status: null
    color: 2f7ed8
    cards:
      - id: 100
        card_type: floater
        title: Write spec
        notes: draft
        tags: [documentation]
        topic_id: null
        assigned_to:
          type: User
          username: alice
      - id: 101
        card_type: topic
        title: null
        notes: null
        tags: []
        topic_id: 1261
        assigned_to: null
```

Schema v1 contains only stable, mutable board fields and identity fields needed for later reconciliation. Board, column, and card IDs identify pulled objects and may be omitted when authoring new objects for the later push phase. Assignees retain their `User`/`Group` type so equal user and group names cannot be confused. The schema deliberately omits opaque positions, tag IDs where names are available, embedded topic metadata, permissions, ACLs, timestamps, usernames that record creation, and unrecognised API fields. JSON/YAML output from `board show` remains the appropriate way to inspect the full response model.

## Notes

- Auth is the standard configured `apikey`/`api_username`; the acting user needs the board ACL `view` to read.
- `--dry-run` permits `board list`, but refuses `board show` and `board pull` because the detail endpoint can mutate board cards and view history.
- `push` (guarded, declarative writes) is a later phase - see `spec/commands/boards.md`.

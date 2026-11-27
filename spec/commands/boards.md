# `dsc board` - manage and sync Discourse Boards

Spec for driving the official Discourse Boards feature (core plugin, formerly
"Kanban v2", shipped September 2026, Business/Enterprise plans) through its
plugin API: listing boards, snapshotting them to files, and (later) pushing
board definitions back. Driver: R53-style declarative sync plus Marcus's
local-first Markdown Kanban research - a `board pull` would let his local
Kanban tooling view a Discourse roadmap as ordinary Markdown cards while
Discourse remains the write-master.

## Motivation

Discourse Boards organises topics and standalone cards into kanban boards at
`/boards`. Everything a sync tool needs lives behind a JSON plugin API, but
that API is undocumented in the official API docs and was invisible to
`dsc`. Discovery on 2026-09-03 against `bawmedical.co.uk`
(Discourse 2026.9.0, commit `dd4cc4f4cc`) captured the full request/response
shape; this spec records it so implementation does not need to re-derive it.

## Current state (as of 2026-11-27)

Phase 1 provides `dsc board list`, `show`, and `pull`. Phase 2 provides the bundled `to-do` template through offline `dsc board templates` and guarded `dsc board scaffold`. Scaffold is create-only: it reads the side-effect-free index, refuses collisions, creates the empty board and five columns in one transactional request, then verifies the response and refreshed index before reporting success. It does not create cards, modify existing boards, or add access grants. The detail GET used by `show` and `pull` invokes Discourse's own topic-card backfill and board-view history writes.

## API surface (observed 2026-09-03)

All endpoints are mounted under `/boards/api/` on the forum and return JSON
with `Accept: application/json`. Auth is the standard admin `Api-Key` +
`Api-Username` headers; the acting user needs the board ACL `view` to read and
`manage` to mutate (defaults: admins and moderators manage everything).

```
GET    /boards/api/boards.json                     -> {"boards":[BoardSerializer...]}
GET    /boards/api/boards/available.json?topic_id= -> {"boards":[BasicBoardSerializer... + topic_is_member, topic_card_id]}
GET    /boards/api/boards/:id.json                 -> {"board":{...},"columns":[ColumnSerializer with cards embedded]}
POST   /boards/api/boards.json                     (201) body {"board":{"name":...}}
PUT    /boards/api/boards/:id.json                 (200) body {"board":{...}}  - partial-ish; name required in practice
DELETE /boards/api/boards/:id.json                 (204) cascades columns and cards
POST   /boards/api/boards/:id/move-column.json     (200) body {"column_id":N,"direction":-1|1}
POST   /boards/api/boards/:id/constraint-preview.json (500 observed - see quirks)
PUT    /boards/api/boards/:id/check-constraint-mismatches.json (204)

POST   /boards/api/boards/:board_id/columns.json   (201) body {"column":{"title":...,"icon":...,"color":...}}
PUT    /boards/api/boards/:board_id/columns/:id.json (200) body {"column":{...}}  - title required
DELETE /boards/api/boards/:board_id/columns/:id.json

POST   /boards/api/boards/:board_id/cards.json     (201) body {"card":{"column_id":N,"title":...,"notes":...,"after_card_id":N,"tag_names":[...],"assigned_to_name":...,"topic_id":N}}
PUT    /boards/api/boards/:board_id/cards/:id.json (200) body {"card":{...}}  - move column / reorder via after_card_id
DELETE /boards/api/boards/:board_id/cards/:id.json (204)
DELETE /boards/api/boards/:board_id/columns/:column_id/cards.json  (clear column)

POST   /boards/api/boards/:board_id/topic-moves.json (201) body {"topic_id":N,"to_column_id":N}
```

### Key serializer fields

Board: `id`, `name`, `slug`, `category_ids`/`tag_ids`/`tag_names` (constraints),
`anonymous_can_read`, `require_confirmation`, `show_tags`, `card_style`
(`detailed`|`simple`), `show_topic_thumbnail`, `can_write`, `can_manage`,
`created_by.username`, `columns[]`, `acl`.

Column: `id`, `title`, `icon`, `position`, `default_sort` (`priority`|`recency`),
`tag_id`/`tag_name` (column's tag binding), `move_to_category_id`,
`move_to_assigned`, `move_to_status`, `color`, `cards[]` (sorted per
`default_sort`: `position` order for priority, `recency_at` desc for recency).

Card: `id`, `board_id`, `column_id`, `card_type` (`floater`|`topic`),
`position` (fractional-style integer ordering, 65536, 131072, ...),
`title` (null for topic cards - topic supplies it), `notes`, `tag_ids` and
`tags` objects (`id`, `name`, `slug`) for floaters, `topic_id` + embedded
`topic` (topic cards; topic has title,
slug, category_id, tags, bumped_at, closed, image_url, posts_count),
`created_at`/`updated_at` (floater only), `column_changed_at`, `recency_at`,
`created_by.username`, `assigned_to`.

### Behaviours and quirks observed

- **The detail GET mutates server state.** Current core source calls
  `TopicSync.backfill_board`, which can create and delete tag-driven topic
  cards, then calls `Boards::ViewBoard`, which records one view per user per
  board per day. `board show` and `board pull` therefore refuse under global
  `--dry-run`; `board list` remains safe. Verified against Discourse commit
  `a6ade3e9915f427d115f082a2ddbc6c208c5f3c1` on 2026-09-07.
- **Positions are large integer gaps** (65536, 131072). Client reordering is
  by `after_card_id` / `before_card_id`-style relative placement, not by
  sending an absolute position; the server recomputes. Moving the last card
  to `after_card_id` of the other card returned `position: 0` - do not
  attempt to persist absolute positions in snapshots; treat them as opaque
  and express order as a list.
- **Column tag binding**: `PUT column` with `tag_name` silently ignored the
  tag on the version tested (200 with `tag_id: null`) until the update also
  carried every other mutable field. Workaround that worked: send the full
  column payload (`title` + `tag_name` + others) on every update. Also note
  `PUT columns/:id` without `title` 400s ("Title can't be blank") - column
  updates are whole-object, not patches.
- **Board-level constraints work via `tag_names`** on `PUT board`
  (resolved to `tag_ids` server-side). Column `color` is a bare hex string
  without `#` (`"2f7ed8"`); `#2f7ed8` 400s with "Color is invalid".
- **`POST topic-moves`** requires `to_column_id` (not `column_id`); wrong
  field yields a misleading "To column can't be blank".
- **`POST move-column`** requires `direction: -1|1` (relative), not
  `before_column_id`/`after_column_id`.
- **Topic card sync is tag-driven**: creating a topic card left the topic's
  own tags untouched (`t/1261` kept `2025, discourse, journal`), but with a
  column bound to tag `discourse`, the card is auto-represented by the tag.
  Constraint-mismatch resolution can mutate the topic - that is the danger
  zone for a push surface.
- **`constraint-preview` 500'd** in every payload variant tried (likely an
  upstream bug or a payload-shape mismatch versus the running release;
  `check-constraint-mismatches` returned 204 fine).
- Deleting a board cascades everything (204, empty index afterwards); good
  for cleanup, and dangerous enough that `dsc board push --prune` must be
  guarded exactly like tag pruning.

## Proposed CLI surface

```text
dsc board list <discourse> [--format text|json|yaml]
dsc board show <discourse> <board-id> [--format text|json|yaml]
dsc board pull <discourse> <board-id> [<file>] [--force]
dsc board templates [--format text|json|yaml]                                   # planned
dsc board scaffold <discourse> <template> [--name <name>] [--format text|json|yaml] [--dry-run] # planned
dsc board push <discourse> <file> [--dry-run] [--yes]   # later phase
```

- `list` - one row per accessible board (id, name, slug, constraints, ACL summary).
- `show` - full board: columns, per-column cards (type, title/topic, position, tags, assignee). This invokes Discourse's detail GET and its server-side sync/history behavior.
- `pull` - snapshot **the entire board visible to the API user, including floater cards**, to stable YAML/JSON, mirroring `setting pull` / `tag pull` conventions (schema `version`, `pulled_at`, `discourse_version`, credential-free `provenance`). Floater cards have no topic behind them, so each becomes a title + notes (+ tag names, typed assignee) entry in the snapshot; topic cards reference their `topic_id`. Decided 2026-09-03 with the maintainer: omitting floaters would make the snapshot a false picture of the board and would make a later `push` unable to restore it. The file retains only stable mutable fields plus optional reconciliation IDs; omitted column/card IDs represent objects to create in the later push phase. It excludes opaque positions, raw unknown API fields, ACL/runtime fields, timestamps, and embedded topic metadata. Card and column order are lists; cards are canonicalized by persisted position even for recency-sorted columns so topic activity does not create diff noise. Assignees retain their user/group discriminator. Pull invokes the same server-side sync/history behavior as `show`.
- `push` - declarative create/update of columns and floater cards; topic cards
  are created by `topic_id` reference only (never by content). Prune
  (deleting boards/columns/cards missing from the file) requires explicit
  `--prune` and honours the global `--dry-run`. Out of scope: mutating
  topics as a side effect of board moves - `dsc` must never silently edit
  forum content to place a card (the tag-bound-column auto-flow does this
  server-side anyway).

### Create from a template

`scaffold` creates a new, empty board from a named template. It doesn't import cards or reconcile an existing board, so it doesn't need `push` or a previous `pull`. `templates` lists the bundled templates and their ordered column titles offline. Use one registry for both commands; add more templates as concrete workflows emerge. The first template, `to-do`, creates a board called **To-do** unless `--name` is supplied:

| Column | Purpose |
| --- | --- |
| Ideas | Suggestions we haven't committed to. |
| Backlog | Work we've decided to do, but haven't started. |
| In progress | Work under way. |
| Completed | Finished work. |
| Reference | Material worth keeping handy, outside the task flow. |

Start on one explicitly named forum. Build a stable slug from the effective name and check the Boards index for it. If the same board name and ordered columns already exist under that slug, report a no-op without adopting the board or touching its cards or permissions. If the slug exists with a different layout or name, stop and suggest `--name` or manual review. Don't generate a suffixed name or slug to get past a collision. A board absent from the permission-filtered index may still exist; rely on the server's unique slug and refuse on a rejected or ambiguous create. After a timeout, re-list before any retry.

`--dry-run` shows the selected forum, template, effective name and slug, ordered columns, restricted access and exact intended create request. It uses only the side-effect-free board index, not the detail GET. Apply creates the board and columns together, then verifies the returned board and confirms it in the index. The first version grants no access beyond Discourse's normal creator and mandatory admin access. It creates no cards, public grants, category or tag bindings, column move actions or site settings. Fleet targeting, custom template files and existing-board updates are out of scope for the first version.

The [Discourse create action and service](https://github.com/discourse/discourse/tree/5eb2603b38634e60dca145e03d8ffaf7a74d5276/plugins/boards) support a single `POST /boards/api/boards.json` with the following payload. Source review confirms that board, ordered columns and ACL creation run in one transaction; the upstream request spec covers creating ordered columns. Verify this exact scaffold request, response and non-member visibility on Koloki Demo before rollout. No scaffold write was tested against a live forum during this design pass.

```json
{
  "board": {
    "name": "To-do",
    "slug": "to-do",
    "columns": [
      { "title": "Ideas" },
      { "title": "Backlog" },
      { "title": "In progress" },
      { "title": "Completed" },
      { "title": "Reference" }
    ],
    "acl": []
  }
}
```

## Phases

### Phase 1 - inspection and snapshots (blocking)

- [x] `board list` and `board show` with text/JSON/YAML output.
- [x] `board pull` snapshot with schema version + provenance header.
- [x] Accurate floater-tag object modelling and a stable snapshot DTO separate
  from the raw API response models.
- [x] Detail-GET side effects documented; `show` and `pull` refused under
  global `--dry-run` because no side-effect-free detail endpoint exists.
- [x] Request-budget test coverage against the mock Discourse (board endpoints mocked).
- [x] Graceful 404 when the plugin is disabled/not licensed - translated to a
  Boards-specific hint (unlicensed, disabled, unavailable, or missing board)
  rather than a pre-flight `admin/site_settings.json` check, matching the
  existing Data Explorer precedent.

### Phase 2 - board templates and create-only scaffolding

- [x] Ship `board templates` and `board scaffold` with the bundled `to-do` template.
- [x] Test offline discovery, exact column order, zero writes in dry-run, duplicate prevention, conflicting or unavailable boards, and preservation of existing boards. Test that a successful create is not reported when verification fails.
- [ ] Pilot with Koloki Demo: create once, check access as a non-staff member, and rerun without adding a second board.

### Phase 3 - guarded content sync

- [ ] `board push` for board/column/floater-card definitions with a complete
  dry-run plan (create/update/unchanged per object, `-` reset markers as in
  other sync commands).
- [ ] Column updates always send the full column object (title required).
- [ ] Topic-card placement only via explicit `topic_id`; refuse to mutate
  topic category/tags/status as a side effect (no constraint-fix automation).
- [ ] `--prune` with `--yes` and a dry-run enumeration of every deletion.

## Backward compatibility

New command surface; no existing command changes. The global `--dry-run`
contract intentionally refuses `board show` and `board pull` before resolving
configuration because Discourse's GET endpoint can write server state.

## Out of scope

- Mutating topics (category/tag/status/assign) to satisfy board constraints.
- Managing board ACLs beyond read-only display (the flattened ACL format was
  not captured this session).
- Non-admin surfaces; everything here assumes the configured admin API key.
- Any UI beyond the CLI.

## Reference: captured exchanges

Created a board, two columns (one with icon+color), two floater cards, one
topic card via `topic-moves` (topic 1261), reordered cards, moved a card
between columns, rebound a column's tag, toggled `card_style`/`show_tags`,
bound board-level constraint to tag `discourse`, moved a column both
directions, deleted a floater card, then deleted the board (204; index
confirmed empty; topic tags unchanged). Full JSON payloads retained in this
session's transcript; response shapes are documented above. Tested against
Discourse 2026.9.0-latest (dd4cc4f4cc5aa73d8eb8efc3c154f4e139ff6052).
Serializer and controller behavior re-verified in current core source at
`a6ade3e9915f427d115f082a2ddbc6c208c5f3c1` on 2026-09-07.

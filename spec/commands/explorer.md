# `dsc explorer` - inspect and run Discourse Data Explorer queries

> **Status: Phases 1 and 2 implemented (unreleased).** `list`, `show`, and `run` use the canonical bundled-plugin routes with safe pagination, typed structured output, parameter files, exact query export, CSV download, explain, limits, filtering, and sorting. Managed query mutation remains deliberately separate.

Spec for the core Discourse Data Explorer plugin. Goal: let an agent or administrator safely discover, inspect, and run existing trusted SQL reports without manually driving the admin UI or leaking the database-wide power of arbitrary SQL authoring. Driver: Data Explorer is now bundled with Discourse and is the practical read-only diagnostic surface for questions that ordinary admin APIs cannot answer, including notification forensics.

## Motivation

Many operational and forensic questions need data that Discourse's admin API does not expose: precise notification-level inheritance, skipped-email records, moderation-history joins, or cross-table diagnostics. Data Explorer already provides an administrator-controlled SQL query catalogue and a parameterized execution API, but `dsc` cannot currently discover or run those reports. Agents therefore fall back to browser automation, hand-written `curl`, or ask an administrator to copy results from the UI. `dsc explorer` should make the common safe workflow scriptable: list a known catalogue, inspect parameter contracts, run a selected saved query, and export results.

## Context

Data Explorer is a bundled core plugin mounted at `/admin/plugins/discourse-data-explorer`. Its compatibility routes under `/admin/plugins/explorer` remain available but are legacy aliases; `dsc` must use the canonical path.

The plugin stores administrator-authored queries, plus built-in default queries with negative IDs. Query execution is parameterized: SQL declares a `-- [params]` header, and callers send a JSON object through the `params` request field. The server validates and resolves typed values, including user, category, topic, group, date, list, and current-user parameters.

Running a saved query is read-only at the CLI layer but executes arbitrary SQL already stored on the forum. Data Explorer itself restricts query management to administrators and rate-limits API query runs. Query definition authoring is intentionally separate from normal report execution because it grants persistent database access to every future caller with query permission.

## Current state (as of 2026-07-28)

`dsc` has no Data Explorer command. `dsc report` remains planned and is intended for curated dashboard reports, not arbitrary saved SQL. `dsc notify` is separately planned for API-backed notification forensics where possible, but some necessary tables have no admin API and require Data Explorer.

## Proposed CLI surface

```text
dsc explorer list <discourse> [--filter <text>] [--format text|json|yaml]
dsc explorer show <discourse> <query-id> [--export <file>] [--format text|json|yaml]
dsc explorer run <discourse> <query-id> [--params <json>|--params-file <file>] [--csv <file>] [--explain] [--limit <n>] [--format text|json|yaml]
```

- **`dsc explorer list`** - paginates `GET /admin/plugins/discourse-data-explorer/queries.json`, following `load_more_queries`; supports server-side `--filter` by query name/description. Text prints ID, name, description, owner, last-run time, and any assigned group IDs. JSON/YAML retain the server's list metadata, including `is_default` for negative built-in query IDs. Empty text output is `No Data Explorer queries found.`
- **`dsc explorer show`** - calls `GET /admin/plugins/discourse-data-explorer/queries/<query-id>.json` and prints the definition, SQL, declared `param_info`, ownership, group access, and cached result when the server supplies one. `--export <file>` writes the exact query-definition export returned by `?export=true`; the command otherwise uses `--format text|json|yaml`. A negative built-in ID is valid. A 404 says the query is absent, hidden, or inaccessible to the configured API user.
- **`dsc explorer run`** - sends `POST /admin/plugins/discourse-data-explorer/queries/<query-id>/run.json` with `params` as a JSON object, plus optional `explain=true` and `limit=<n>`. `--params` must be a JSON object; `--params-file` reads the same object from JSON or YAML. The command validates that exactly one parameter source is supplied, but leaves type validation to Discourse because query definitions own parameter semantics. Default text output is a stable tabular rendering with column headings; JSON/YAML expose the returned `columns`, `rows`, executed `params`, duration, and optional `explain` plan. `--csv <file>` requests `run.csv?download=true`, writes the server CSV atomically, and cannot be combined with `--format` or `--explain`.
- **`--limit <n>`** - an explicit requested result cap. The CLI rejects zero and lets Discourse enforce its configured result maximum. It never offers a bypass for `QUERY_RESULT_MAX_LIMIT`.
- **`--explain`** - asks Discourse for PostgreSQL's execution plan without returning a different query definition. The text formatter prints the plan after result metadata; structured output uses the server's `explain` field.

Every subcommand requires Admin API credentials. `run` retries ordinary rate-limit responses via `DiscourseClient` and surfaces the plugin's per-10-second API query-run limit without silently retrying an unsafe or long-running SQL operation.

## Deliberately excluded command surface

```text
# Not in Phase 1
dsc explorer create ...
dsc explorer update ...
dsc explorer delete ...
dsc explorer preview <sql> ...
dsc explorer generate ...
dsc explorer schema ...
dsc explorer groups ...
```

Saved-query management and ad hoc preview both create a much larger governance and safety surface: SQL review, ownership, group visibility, hidden-query lifecycle, source-control schema, destructive query policy, and query cache invalidation. They should follow only after a concrete managed-query use case establishes an auditable file format and review workflow. AI SQL generation is explicitly outside `dsc`'s initial scope. Schema discovery is high-volume and its useful output format needs a separate concrete driver. Group report visibility is a user-facing reporting feature, not an admin agent workflow.

## Reference: API calls observed in the field

Upstream Discourse main, Data Explorer bundled core snapshot inspected 2026-07-28. Routes are defined in `plugins/discourse-data-explorer/config/routes.rb`; controller behavior is in `app/controllers/discourse_data_explorer/query_controller.rb`.

### List saved queries

```text
GET /admin/plugins/discourse-data-explorer/queries.json?offset=0&filter=notification
Api-Key: <redacted>
Api-Username: <admin>

→ 200 OK
{
  "queries": [
    {
      "id": 42,
      "name": "Notification audit",
      "description": "...",
      "username": "admin",
      "group_ids": [],
      "last_run_at": "2026-07-28T10:00:00.000Z",
      "user_id": 1,
      "is_default": false
    }
  ],
  "total_rows_queries": 1,
  "load_more_queries": "/admin/plugins/discourse-data-explorer/queries.json?offset=50"
}
```

The server page size is 50. It includes unpersisted default queries on the first page and represents them with negative IDs and `is_default: true`.

### Inspect or export one query

```text
GET /admin/plugins/discourse-data-explorer/queries/42.json
GET /admin/plugins/discourse-data-explorer/queries/42.json?export=true

→ 200 OK
{
  "query": {
    "id": 42,
    "name": "Notification audit",
    "description": "...",
    "sql": "-- [params]\n-- int :days = 30\n...",
    "param_info": [
      {"identifier":"days","type":"int","default":"30","nullable":false,"internal":false}
    ],
    "group_ids": [],
    "created_at": "...",
    "hidden": false
  }
}
```

`?export=true` returns the query export as an attachment and is the source for `--export` rather than reconstructing a portable format in `dsc`.

### Run a saved query

```text
POST /admin/plugins/discourse-data-explorer/queries/42/run.json
Content-Type: application/x-www-form-urlencoded
Api-Key: <redacted>
Api-Username: <admin>

params={"days":30}
limit=100
explain=false

→ 200 OK
{
  "success": true,
  "errors": [],
  "params": {"days":30},
      "duration_secs": 0.0123,
  "columns": ["username", "emails"],
  "rows": [["alice", 2]]
}
```

The plugin accepts the same run endpoint with `.csv` and `download=true` for CSV output. JSON result rows are ordered arrays aligned to `columns`; `explain=true` may add a PostgreSQL plan. Invalid parameter JSON or SQL errors return `422` with `success: false` and `errors`.

## Parameter contract

`dsc explorer run` deliberately accepts an object rather than inventing one flag per type:

```bash
dsc explorer run forum 42 --params '{"days":30,"category":"support"}'
dsc explorer run forum 42 --params-file notification-params.yaml
```

```yaml
days: 30
category: support
```

Discourse declares supported parameter types in SQL comments below `-- [params]`: `int`, `bigint`, `boolean`, `string`, `date`, `time`, `datetime`, `double`, entity IDs, list types, and server-injected `current_user_id`. The CLI rejects non-object parameter documents and duplicate parameter sources but does not duplicate the plugin's entity lookup, defaulting, nullable, or internal-parameter logic.

## Output contract

`list` and `show` follow the repository's normal `text|json|yaml` list/read conventions. `run` defaults to text for terminal use, JSON/YAML for automation, and uses an explicit `--csv <file>` to avoid mixing downloaded output with diagnostics. Data always goes to stdout or the named file; warnings, timing hints, and errors go to stderr. SQL and result data must never be printed in a diagnostic context that could be redirected accidentally from stderr.

## Phases

### Phase 1 - blocking

- [x] Add `ExplorerCommand` and `src/commands/explorer.rs` with `list`, `show`, and `run`.
- [x] Add typed API client methods for canonical Data Explorer list, show, JSON run, and CSV download routes.
- [x] Follow `load_more_queries` safely, deduplicate query IDs, and reject pagination loops or unknown response shapes.
- [x] Support negative built-in query IDs in CLI parsing and API methods.
- [x] Implement JSON-object parameter input from inline JSON and JSON/YAML files.
- [x] Render result columns/rows in text, JSON, and YAML without losing column order or type values.
- [x] Add offline API-shape, parameter-validation, pagination, and CLI parsing tests.
- [x] Add one ignored disposable-forum compatibility test that lists and runs bundled read-only query `-1` with a one-row limit when Data Explorer is enabled, or verifies the actionable disabled-plugin diagnostic otherwise.

### Phase 2 - iteration ergonomics

- [x] `show --export <file>` exact server export.
- [x] `run --csv <file>`, `--explain`, and bounded `--limit`.
- [x] Optional `--filter`, `--order`, and `--ascending` mapping to server list parameters.
- [x] Friendly parameter help that renders `param_info` before the user must handcraft JSON.

### Phase 3 - managed query definitions, only on real demand

- [ ] Define an auditable query-definition file format with provenance, SQL review notes, and group-access semantics.
- [ ] Add guarded `pull`/`push`/`diff` for administrator-owned saved queries.
- [ ] Decide how deletion, hiding, default negative IDs, cache invalidation, and group grants are represented before exposing mutation commands.

## Backward compatibility

Purely additive. `dsc report` remains reserved for curated report commands. The command depends on a bundled core plugin, but older Discourse versions or explicitly disabled Data Explorer installations can return 404; `dsc` must identify that condition and direct the operator to enable Data Explorer or check the server version.

## Out of scope

- Executing raw SQL supplied on the command line or stdin.
- Creating, updating, deleting, hiding, or changing group access for saved queries in Phase 1.
- AI query generation.
- Bypassing Data Explorer result, API-rate, or permission limits.
- Replacing `dsc report` dashboard-style reports.
- Running public/group report endpoints with User API authentication.
- Treating Data Explorer results as a substitute for a stable public admin API.

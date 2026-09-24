# dsc explorer

Discover, inspect, and run administrator-controlled saved queries from Discourse's bundled Data Explorer plugin. `dsc` does not accept arbitrary SQL or manage query definitions.

All commands require Admin API credentials and the `data_explorer_enabled` site setting. A 404 can mean Data Explorer is disabled, unavailable on that Discourse version, or the selected query is hidden or inaccessible.

## List queries

```text
dsc explorer list <discourse>
                  [--filter <text>]
                  [--order name|username|last-run-at] [--ascending]
                  [--format text|json|yaml]
```

The command follows every server page and deduplicates query IDs. Negative IDs identify built-in default queries and are valid throughout the command.

```bash
dsc explorer list myforum
dsc explorer list myforum --filter notification --order name --ascending
dsc explorer list myforum --format json
```

## Show a query

```text
dsc explorer show <discourse> <query-id> [--format text|json|yaml]
dsc explorer show <discourse> <query-id> --export <file>
```

Normal output includes the saved SQL, parameter contract, owner, group access, timestamps, and any cached result returned by Discourse. `--export` writes the exact portable query-definition attachment returned by the server with owner-only permissions and refuses to overwrite an existing file.

```bash
dsc explorer show myforum 42
dsc explorer show myforum -1 --format yaml
dsc explorer show myforum 42 --export notification-audit.dcquery.json
```

## Run a query

```text
dsc explorer run <discourse> <query-id>
                 [--params <json> | --params-file <json-or-yaml-file>]
                 [--limit <n>] [--explain]
                 [--format text|json|yaml]

dsc explorer run <discourse> <query-id>
                 [--params <json> | --params-file <json-or-yaml-file>]
                 [--limit <n>] --csv <file>

dsc explorer run <discourse> --query-name <exact-name> [options]
dsc explorer run --all --query-name <exact-name> [options]
dsc explorer run --tags <tag1,tag2> --query-name <exact-name> [options]
```

Parameters must form one JSON object. Inline values use JSON; parameter files accept JSON or YAML. Discourse remains responsible for validating each saved query's declared parameter types, defaults, nullability, and entity lookups.

```bash
dsc explorer run myforum 42 --params '{"days":30,"category":"support"}'
dsc explorer run myforum 42 --params-file notification-params.yaml --format json
dsc explorer run myforum 42 --limit 100 --explain
dsc explorer run myforum 42 --params-file notification-params.yaml --csv results.csv
dsc explorer run myforum --query-name "Email Reachability Snapshot" --format json
dsc explorer run --all --query-name "Email Reachability Snapshot" --format json
dsc explorer run --tags production --query-name "Email Delivery Outcomes" --params '{"days":90}' --format json
```

`--query-name` resolves a complete, case-sensitive saved-query name independently on each selected forum. Substring matches are ignored, duplicate exact names are refused with their IDs, and one missing or unavailable query does not suppress other fleet results. Fleet execution uses bounded concurrency, preserves configuration order in JSON/YAML, emits one result or error per forum, and exits non-zero after rendering if any forum failed. Numeric query IDs are forum-local and cannot be used with `--all` or `--tags`.

`--limit` must be positive and cannot bypass Data Explorer's server-side maximum. `--csv` writes the server-generated CSV atomically with owner-only permissions, refuses to overwrite an existing file, and cannot be combined with `--format`, `--explain`, `--all`, or `--tags`. Fleet CSV is deferred until directory and filename semantics are defined.

`run` honours `-n` / `--dry-run`: it prints the selected forums, query ID or unresolved exact name, parameters, limit, and destination, then exits without contacting the server. The saved SQL itself is read-only, but Discourse records `last_run_at` against the query and charges the API rate limit for every run, so a dry run must not send the request.

Text output is a stable table. JSON and YAML preserve cell types and server metadata, including relations and execution-plan fields that the current plugin may add.

## Security

Data Explorer executes saved SQL in a read-only database transaction, but query definitions and results can expose sensitive forum data. `dsc` cannot generically redact arbitrary result columns. SQL and result data are written only to stdout or the explicitly named export/CSV file; diagnostics and row counts go to stderr. Review a query with `show` and inspect its parameter contract before running it from an automated agent.

# Report evidence collection

> **Status: R65 Phase 1 in progress.** The first implementation extends `dsc explorer run` with exact query-name resolution and canonical fleet selectors. Evidence bundles, validation, and any report presentation kit remain later phases.

Spec for collecting deterministic, reviewable evidence that an external author or AI can turn into a narrative report. Goal: keep remote access, fleet behavior, provenance, and verification inside `dsc` while keeping interpretation and presentation outside the CLI. Driver: the 23 September 2026 email reachability and deliverability audit across 16 managed forums required a one-off collector before an AI could produce a useful self-contained HTML comparison report.

## Motivation

`dsc` can run one saved Data Explorer query on one forum, but a fleet comparison currently requires a custom program to find equivalent saved queries, continue past unavailable forums, preserve target order, and combine results. Once collected, an AI is well suited to selecting observations and composing a report, but it should not query production forums ad hoc or become the authority for what was collected. The durable boundary is a frozen evidence artifact produced by `dsc`; report prose, emphasis, charts, and HTML are derived outputs.

## Product boundary

`dsc` owns deterministic collection and verification:

- Explicit forum selection, bounded fleet execution, and per-forum failure isolation.
- Exact saved-query resolution, parameter handling, and typed result preservation.
- Stable schemas, timestamps, source identity, query-definition digests, checksums, completeness warnings, and private atomic writes.
- Validation that an evidence bundle is internally consistent and that a derived report identifies the bundle it used.

External tooling owns interpretation and presentation:

- Choosing the important observations and comparisons.
- Writing conclusions, caveats, and recommendations.
- Choosing sections, tables, charts, callouts, and responsive HTML.
- Maintaining organisation-specific branding, prompts, and a report design kit.
- Browser-based visual and accessibility inspection.

`dsc` does not invoke an LLM, store model credentials, validate the truth of narrative conclusions, silently enable Data Explorer, or attempt to define a universal report document format.

## Phase 1 CLI surface

Existing single-forum query-ID execution remains unchanged:

```text
dsc explorer run <discourse> <query-id> [options]
```

Exact query-name resolution is additive for one forum or a selected fleet:

```text
dsc explorer run <discourse> --query-name <exact-name> [options]
dsc explorer run --all --query-name <exact-name> [options]
dsc explorer run --tags <tag1,tag2> --query-name <exact-name> [options]
```

- `--query-name` lists the accessible saved-query catalogue on each selected forum and then matches the complete name case-sensitively. A substring match is never accepted.
- No exact match is a per-forum failure that names the requested query. More than one exact match is a per-forum failure that reports the duplicate IDs rather than choosing one.
- Fleet execution requires `--query-name`; numeric query IDs are forum-local and are not assumed portable.
- Fleet work uses the shared bounded executor, preserves configuration order, continues after per-forum failures, emits one success or error record for every selected forum, and exits non-zero after rendering when any forum failed.
- JSON and YAML fleet successes include `forum`, resolved `query_id`, `query_name`, and the complete typed Explorer result. Text groups each result beneath its forum and resolved query identity.
- `--csv` remains single-forum because a safe fleet directory and filename contract has not been designed. `--explain`, `--limit`, `--params`, and `--params-file` apply identically to every selected forum.
- Dry-run resolves selectors and parameters without contacting any forum. For name-based execution it reports the unresolved exact query name because discovering IDs would itself consume API requests.
- A disabled or unavailable Data Explorer remains an explicit per-forum failure. Read-only collection never changes `data_explorer_enabled`.

## Phase 2 evidence bundle

After Phase 1 proves the fleet execution contract, add an explicit evidence artifact rather than treating ordinary Explorer JSON as a durable audit format. The tentative surface is:

```text
dsc evidence collect <recipe.yaml> <discourse|--all|--tags <tags>> --output <directory>
dsc evidence verify <directory>
```

The recipe is a versioned local file naming one or more trusted data sources and their parameters. Data Explorer sources use exact query names. Additional existing read-only sources such as admin reports may be admitted only when a real report needs them.

The bundle manifest should include:

- Schema version, collection ID, collection start/end timestamps, and `dsc` version.
- The resolved target list in configuration order.
- Forum name, base URL, Discourse version and commit where available.
- Source kind, exact query name and resolved ID, executed parameters, and a SHA-256 digest of the query definition or SQL.
- Typed results, per-forum errors, warnings, and an explicit completeness state.
- Exact absolute reporting windows resolved once before fleet work begins.
- SHA-256 digests for every artifact in the bundle.

Collection writes into a private staging directory and publishes the completed bundle atomically where the platform permits it. Unknown recipe schema versions are rejected. `verify` performs no network access.

## Phase 3 report kit experiment

Keep the initial report kit outside `dsc`, alongside the management workspace. It should contain a versioned stylesheet, semantic component examples, responsive and print rules, accessibility guidance, and authoring instructions. An AI may use the kit to create a self-contained HTML report from one frozen evidence bundle.

Every derived report should identify the evidence bundle checksum and report-kit version in machine-readable metadata. After at least three materially different reports demonstrate a stable visual contract, decide whether `dsc` should embed and export the kit or deterministically assemble an HTML shell. Until then, CSS and HTML are not a CLI compatibility promise.

## Phase 4 derived-artifact validation

If report production becomes routine, extend offline verification to check that a report references an existing verified evidence digest, contains the required provenance and limitations sections, has no unintended remote assets, and satisfies basic semantic HTML rules. This validates linkage and structure, not AI-authored interpretation.

## Backward compatibility

Phase 1 is additive. `dsc explorer run <discourse> <query-id>` retains its current output and behavior. Fleet mode has a new forum-tagged output shape because there is no prior fleet contract. Later evidence bundle schemas are independently versioned and do not replace ordinary Explorer output.

## Out of scope

- Raw or AI-generated SQL execution.
- Silently creating, updating, or installing saved queries.
- Temporarily enabling Data Explorer as part of a read command.
- Fleet CSV until directory, filename, partial-failure, and overwrite semantics are specified.
- Calling an LLM or prescribing a model provider.
- Treating SMTP acceptance, other operational proxies, or AI conclusions as facts not supported by the collected sources.
- A universal HTML, chart, or narrative schema in `dsc`.

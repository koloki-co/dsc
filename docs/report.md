# dsc report

Fetch a single raw Discourse admin report, unmodified — the `/admin/reports/{id}.json` data Discourse itself computes, with no cross-report derivation or ratio math.

Distinct from [`dsc analytics`](analytics.md): `analytics` combines several reports into a curated community-health dashboard (growth/activity/health sections, computed ratios like posts-per-topic). `dsc report` is a thin pass-through/formatter over one report id at a time — useful for a quick look at a single metric, or for piping a report's raw series into another tool.

## Usage

```text
dsc report <discourse> <name> [--since <when>] [--format text|json|yaml]
```

- `<name>` — a Discourse admin report id, passed straight through to `/admin/reports/{id}.json`. Common ids: `signups`, `topics`, `posts`, `likes`, `flags`, `moderators_activity`, `trust_level_growth`, `time_to_first_response`, `topics_with_no_response`, `users_by_trust_level`. Check your Discourse instance's admin Reports page for the full list available on that forum — plugins can add more.
- `--since` / `-s` (default `30d`) — window length. `24h`, `7d`, `30d`, `1w`, `1y`, or an ISO-8601 timestamp. Same syntax as `dsc analytics --since`.
- `--format` / `-f` (default `text`) — `text`, `json`, or `yaml`.

## Examples

```bash
dsc report myforum signups
dsc report myforum posts --since 7d --format json
```

## Output

`text` prints the report id, the resolved window, the total and average (when Discourse computes one), then the report's data points. Discourse reports come in two shapes:

- **flat** (most reports): one `x`/`y` pair per day, printed one per line.
- **stacked** (e.g. `trust_level_growth`): several named series, each with its own points; printed indented under a `[series label]` header.

`json`/`yaml` emit the same fields (`report_id`, `since`, `start_date`, `end_date`, `total`, `average`, `higher_is_better`, `data`) with `data` as Discourse returns it, unflattened — use this if you need the raw shape for further processing.

## Auth

Requires admin scope, like `dsc analytics`. An unrecognised or renamed report id surfaces as a normal HTTP error from Discourse rather than a silent empty report.

This command does not honour `--dry-run` — it's read-only.

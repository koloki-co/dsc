# dsc analytics

Community-health snapshot for a Discourse — three sections (growth, activity, health) of metrics chosen so that each one, on its own, tells you something actionable. Designed to fit on a single screen and to pipe cleanly into a journal topic for trend tracking.

The full design rationale lives in [`spec/commands/analytics.md`](https://github.com/koloki-co/dsc/blob/main/spec/commands/analytics.md). This page documents the shipped command and its current limitations.

## Usage

```text
dsc analytics <discourse>
              [--since <when>] [--compare]
              [--snapshot] [--periods 24h,7d,30d,1y]
              [--section all|growth|activity|health]
              [--format text|table|json|yaml|markdown|markdown-table|csv]
```

Alias: `dsc stats` is accepted as a shorter alias; `analytics` is canonical.

### Modes

The command runs in one of three modes:

- **single window** (default): `--since 30d` — one column of numbers for the requested window.
- **compare**: `--since 30d --compare` — two columns (current and previous-period-of-equal-length) plus a delta-percent column.
- **snapshot**: `--snapshot` — N columns (default `24h, 7d, 30d, 1y`) showing each metric across multiple time horizons. Replaces `--since`+`--compare`. Best paired with `--format table`.

### Flags

| Flag | Default | Notes |
|------|---------|-------|
| `--since` / `-s` | `30d` | Window length. `24h`, `7d`, `30d`, `1w`, `1y`, or an ISO-8601 timestamp. Ignored with `--snapshot`. |
| `--compare` / `-c` | off | Pair the window with the immediately preceding window of equal length. Mutually exclusive with `--snapshot`. |
| `--snapshot` | off | Multi-window dashboard mode. Default periods are `24h, 7d, 30d, 1y`. |
| `--periods` | `24h,7d,30d,1y` | Comma-separated periods for `--snapshot`. Order is preserved in output. Requires `--snapshot`. |
| `--section` | `all` | Restrict output to one section (`growth`, `activity`, `health`). |
| `--format` / `-f` | `text` | One of `text`, `table`, `json`, `yaml`, `markdown` (`md`), `markdown-table` (`md-table`), `csv`. |

### Output formats

- **`text`** — fixed-width columns, no borders. The default; always machine-parseable.
- **`table`** — DuckDB-style box-drawing table with `┌─┬─┐` borders, right-aligned numbers, thousand separators on counts. Falls through to `text` automatically when stdout isn't a TTY (so you don't get borders polluting cron logs).
- **`json` / `yaml`** — schema-versioned (`"schema": 1`). Stub metrics carry `"not_implemented": true`. Each metric's value column lives under `values: { "<column-header>": <number-or-null> }`, so single/compare/snapshot all share one shape.
- **`markdown` / `markdown-table`** — pipe straight into `dsc topic reply` for a journal entry.
- **`csv`** — one row per metric. Columns: `section, metric, <one column per window>, desirable_direction, unit`.

### Latency

Snapshot mode makes ~9 reports × N periods API calls. Calls run through a 4-worker concurrent pool (just below typical nginx burst limits) and the cross-cutting 429 retry kicks in if a forum is sensitive — observed runs against busy forums complete in 5-10 seconds. Single-window remains under one second.

### Auth

Requires admin scope. Most reports are admin-only on Discourse. If your configured key isn't admin you'll see the standard `missing apikey for {discourse}` failure rather than a silently-truncated dashboard.

This command does **not** honour `--dry-run` — it's read-only.

## Sections

> **v1 status:** the sections below describe every metric the command is designed around, including some that aren't computed yet. v1 implements the per-report metrics directly; the per-user-walk metrics (new contributors, reactivated users, lost regulars, unique posters, top-10 share, returning poster rate, solo-thread rate) print `— (n/i)` in text and `"not_implemented": true` in JSON. See [What v1 actually computes](#what-v1-actually-computes) for the per-metric matrix, and [spec/commands/analytics.md - Implementation follow-ups](https://github.com/koloki-co/dsc/blob/main/spec/commands/analytics.md#implementation-follow-ups) for the design constraints (mainly per-user-walk latency vs gating behind a future `--full` flag).

### Growth

Is the community gaining or losing engaged people?

| Metric | Definition |
|--------|-----------|
| New contributors | Users whose **first ever post** was in the window. |
| Reactivated users | Users who posted in the window after **≥ 90 days** of silence. |
| Lost regulars | Users who posted ≥ 4 times in the prior 90 days but **zero** in the window. Reported only when `--since` is ≥ 30d. |
| Net active change | New contributors + reactivated − lost regulars. |
| Trust-level promotions | Count of TL0→TL1, TL1→TL2, TL2→TL3 promotions in the window. |

### Activity

Is the place busy and broad, or quiet and lopsided?

| Metric | Definition |
|--------|-----------|
| Topics created | New topics in the window (excluding PMs). |
| Posts created | New posts (replies + topic OPs). |
| Posts per topic | Posts ÷ topics — conversation depth signal. |
| Unique posters | Distinct users who created at least one post in the window. |
| Top-10 share | % of all posts in the window written by the 10 most active posters. Healthy public communities sit roughly in the 30–55% band; sustained > 70% is a fragility signal. |
| Reply coverage | % of topics created in the window that received at least one reply from a different user within 7 days. |
| Median time to first reply | Median minutes from topic creation to first reply, computed only over topics that received a reply. |

### Health

Are the people here treating the place — and each other — well?

| Metric | Definition |
|--------|-----------|
| Likes per post | Likes given ÷ posts created. |
| Returning poster rate | % of posters in the window who also posted in the previous window of equal length. |
| Flags raised | User-raised flags in the window (not auto-flags). |
| Flag resolution time | Median hours from flag creation to staff disposition. |
| Moderator action rate | Moderator actions per 1,000 posts in the window. Normalises moderator load against site activity. |
| Solo-thread rate | % of topics where only the OP posted (no replies, ever). |

## What v1 actually computes

Some metrics are computed straight from Discourse's `/admin/reports/{id}.json` endpoints; some need cross-API derivation we haven't implemented yet. Stubbed metrics print **`— (n/i)`** in text mode and JSON `"not_implemented": true`.

| Metric | v1 status |
|--------|-----------|
| Topics created | ✅ from `topics` |
| Posts created | ✅ from `posts` |
| Posts per topic | ✅ derived (posts / topics) |
| Reply coverage | ✅ derived (`topics` − `topics_with_no_response`) ÷ topics |
| Median time to first reply | ✅ from `time_to_first_response` (depends on Discourse version) |
| Trust-level promotions | ✅ from `trust_level_growth` |
| Likes per post | ✅ derived (`likes` / `posts`) |
| Flags raised | ✅ from `flags` |
| Flag resolution time | ⚠ from `flag_response_time` — not all Discourse versions expose this (renders `—` when unavailable) |
| Moderator action rate | ✅ derived (`moderators_activity` / `posts` × 1000) |
| New contributors | ⏳ stub — needs first-post detection |
| Reactivated users | ⏳ stub — needs per-user post-history walk |
| Lost regulars | ⏳ stub — needs per-user post-history walk |
| Net active change | ⏳ stub — depends on the three above |
| Unique posters | ⏳ stub — needs per-user breakdown |
| Top-10 share | ⏳ stub — needs per-user post counts |
| Returning poster rate | ⏳ stub — needs cross-window per-user comparison |
| Solo-thread rate | ⏳ stub — needs per-topic reply-count walk |

The stubbed metrics are tracked in [spec/commands/analytics.md - Implementation follow-ups](https://github.com/koloki-co/dsc/blob/main/spec/commands/analytics.md#implementation-follow-ups) and will land in patch releases.

## Examples

```bash
# Default 30-day window, plain text.
dsc analytics myforum

# Multi-window snapshot, DuckDB-style table.
dsc analytics myforum --snapshot --format table

# Custom periods.
dsc analytics myforum --snapshot --periods 1d,1w,1m,3m,1y --format table

# Week-over-week comparison.
dsc analytics myforum --since 7d --compare

# Only the activity section, in markdown.
dsc analytics myforum --section activity --format markdown

# Weekly community-health roll-up cron'd into a private staff topic.
dsc analytics myforum --since 7d --compare --format markdown \
  | dsc topic reply myforum 4242

# CSV append into a tracking spreadsheet (snapshot mode = one row per metric
# with one column per period).
dsc analytics myforum --snapshot --format csv >> ~/dsc-trends/myforum.csv
```

## Output formats

- **`text`** (default) — one screen, fixed-width columns. With `--compare`, each metric prints current and previous values plus a delta-percent. Arrows are colour-coded by *desirable* direction, not raw sign — "lost regulars going up" is red, "solo-thread rate going down" is green.
- **`json`** — stable schema (versioned via `"schema": 1`), suitable for trend tracking via cron. Stubbed metrics carry `"not_implemented": true`. Where a previous-window value is missing the field is `null`, never `0`.
- **`yaml`** — same schema as JSON.
- **`markdown`** — one heading per section, bullet list of metrics. Pipes cleanly into `dsc topic reply` / `dsc topic new`.
- **`markdown-table`** — one Markdown table per section. With `--compare` includes current/previous/Δ columns.
- **`csv`** — one row per metric. Columns: `section, metric, current, previous, delta, delta_pct, desirable_direction, unit`.

## Edge cases

- **New install with < `--since` history.** Window is currently used as-is; clamp behaviour will land in a patch. JSON includes `"clamped": true` when implemented.
- **Window of zero topics.** Posts-per-topic, reply coverage, and median time to first reply print `—` (em dash) rather than misleading zeros.
- **Reports unavailable on the Discourse version.** A one-line `[analytics] note:` is printed to stderr and the metric renders `—`. The rest of the dashboard still runs.
- **Time zones.** Windows are computed in UTC. Header prints UTC dates explicitly.

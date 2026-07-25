# `dsc update` - an append-only update log + skip-recently-updated

> **Status: implemented (unreleased).** A register of what `dsc update` did to
> each forum and when, so a fleet round is auditable and re-runnable without
> repeating the day's work. Extends the `UpdateOutcome` skip model from
> [update-concurrency.md](update-concurrency.md). Log write/render + skip-recent
> verified on koloki-demo (seeded log; `update`/`update all --skip-recent` skip
> without touching a forum). Stretch (central register topic) not built.

Driver: the Koloki fleet (~20 forums). Running `dsc update all`, one forum fails (igkt) and gets fixed by hand; re-running to catch it should NOT re-do the 19 that already succeeded. Historically this was tracked by hand in a "Servers" checklist topic on a personal Discourse.

## Motivation - it's not just a record

The record is useful on its own (audit: what ran, when, from→to version). But the load-bearing reason is efficiency, and it exposes a real gap: `run_update`'s order is **OS update → reboot → skip-if-current → rebuild**. `skip-if-current` only skips the *rebuild* - the OS update and **reboot happen first, unconditionally**. So re-running `dsc update all` to catch one straggler today would **reboot all 19 already-updated forums** just to no-op the rebuild. A "was this updated recently?" gate lets `dsc` skip the *whole* forum (like the rebuild-lock), before any reboot.

## The log

- **Append-only, one record per line, timestamp-first** - classic Unix logfile discipline (greppable, `tail`-able, `awk`-friendly, never rewritten). One record is appended per forum per `dsc update` pass, whatever the outcome.
- **Format: TSV**, columns: `timestamp` (ISO-8601 UTC) · `forum` · `outcome` · `from_version` · `to_version` · `detail`. `detail` holds a failure reason or short commit range; `-` for empty fields. TSV keeps it both human-glanceable and trivially parseable; the pretty view is rendered (below), so the raw log optimises for machines and `grep`.
- **Location:** `$XDG_STATE_HOME/dsc/update.log` (default `~/.local/state/dsc/update.log`) - log/state data, not config. Overridable with `DSC_UPDATE_LOG`. Created (with parents) on first write.
- **Outcomes recorded:** `updated` · `current` (was already on latest; rebuild skipped) · `skipped-recent` · `skipped-rebuild` (a `./launcher rebuild` was already running) · `failed` (detail = reason). This means a failure is now *recorded*, not just printed.
- Data comes straight from `UpdateOutcome` + `UpdateMetadata` (before/after version + commit already captured), plus a UTC timestamp.

Example:

```
2026-07-01T09:12:03Z	bawmedical	updated	2026.6.0	2026.7.0	-
2026-07-01T09:14:20Z	igkt	failed	2026.6.0	-	bootstrap exit 15 (mysql-dep template)
2026-07-01T11:30:41Z	igkt	updated	2026.6.0	2026.7.0	-
```

## `dsc update log`

Render the log; the raw file stays machine-first.

```
dsc update log [--latest] [--since <dur>] [--format md|text|json]
```

- default: the full chronological log as a table.
- `--latest`: collapse to **one row per forum** (its most recent record) - the "Servers" checklist view, generated instead of hand-maintained.
- `--since 7d`: window the output.
- `--format`: `md` (a Markdown table - paste-ready), `text` (aligned, default), `json` (the records as objects).

## Skip / prompt on recently-updated forums

A forum is "recently updated" if its most recent `updated`-or-`current` record is within the window (default **24h**). Three ways to handle it:

- **Prompt (interactive default).** With a TTY and no overriding flag, `dsc update` asks about recently-updated forums **up front, before any slow work starts**, so the operator answers everything and can then walk away for an unattended run. For `update all` this is one batched prompt: *"N forums were updated within 24h (list). Update them again anyway? [y/N]"* (default **N** = skip them). For a single `dsc update <forum>` it's a per-forum *"<forum> was fully updated 3h ago - update again? [y/N]"*. **No prompt ever fires mid-run.**
- **`--skip-recent [dur]`.** Silently skip recently-updated forums (optional duration overrides the 24h default). No prompt - the answer for cron / `-y` / non-TTY re-runs.
- **`--force`.** Update everything regardless; also overrides the rebuild-lock. No prompts, no skips.
- **Non-interactive without a flag** (piped, `-y`, or no TTY): behaviour unchanged - update everything (prompting is a human convenience that degrades to today's behaviour; automation must opt in with `--skip-recent`).

The skip happens at the **top of `run_update`**, before the OS update / reboot, and records a `skipped-recent` line. It composes with the other two gates - each outcome is logged:

| gate | question | when |
|---|---|---|
| rebuild-lock | is a `./launcher rebuild` running now? | top of `run_update` |
| skip-recent | did we fully update it within the window? | top of `run_update` |
| skip-if-current | is it already on the latest stable commit? | after reboot, skips only the rebuild |

## Phases

- [x] **Phase 1 - the log.** `src/commands/update_log.rs`: TSV append (O_APPEND-atomic per line, so parallel workers don't interleave), `$XDG_STATE_HOME/dsc/update.log` (or `$DSC_UPDATE_LOG`), parse, `dsc update log [--latest] [--since] [--format text|md|json]`. Every `run_update` outcome logged via `update_and_log` (updated/current/skipped-rebuild/failed). Unit-tested (duration parse, line round-trip, TSV sanitising, malformed-skip).
- [x] **Phase 2 - skip-recent + prompt.** `--skip-recent [dur]` (silent) and the front-loaded prompt (batched for `all`, per-forum for one; TTY-gated, non-interactive = unchanged behaviour); `--force` overrides both this and the rebuild-lock; `skipped-recent` outcome + log line. Verified on koloki-demo.

## Stretch

- **Central register topic.** Post/update a "Servers" topic on a nominated home Discourse from the log (automating the manual checklist). Reuses the changelog-posting plumbing; the log is its data source. Build only if the local file proves it earns the extra surface.

## Out of scope

- Rewriting the log (it is append-only; the current-state view is derived by `--latest`).
- Log rotation/pruning for now - it's one short line per forum per run; revisit if it ever matters.
- Front-loading the *existing* changelog-post prompt (a separate mid-run prompt, already mitigated by `-y`); could adopt the same up-front model later.

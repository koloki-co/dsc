# dsc — CLI design

The design philosophy and normative standards for the `dsc` command-line surface: what makes a pile of subcommands feel like *one tool*. Per-command usage lives in [docs/](../docs/); the cross-project version of these rules (they apply to every CLI Marcus builds) lives in [`~/code/house-style/rust-cli.md`](../../../house-style/rust-cli.md). This file is how `dsc` applies and extends them, with `dsc`'s own examples. It is normative for new commands.

## Philosophy

Six principles, in rough priority order. When they conflict, the earlier one wins.

1. **Scriptable and agent-friendly first.** `dsc` is used by humans, shell pipelines, and LLM agents in roughly equal measure. Data goes to stdout, everything else (hints, progress, errors) to stderr, and every command speaks `--format json`. If `dsc x | jq` doesn't get clean data, the command is wrong.
2. **Declarative over imperative.** Managing server state is a `pull → edit → push → diff` loop against a version-controllable file, not a scatter of one-shot setters. The file is the source of truth you can review; the tool reconciles.
3. **Plan before you act.** Every mutation can be previewed (`--dry-run`), is idempotent (send only what differs), and reports what it did (a diff/plan, not "done"). Acting on a production forum should never be a surprise.
4. **Safe by default.** Refuse the dangerous case with a message that teaches the safe one. Never print secrets. Sequence multi-step changes so a failure leaves a clean state.
5. **Consistent and discoverable.** One ordering, one error shape, one empty-list shape, one flag style - so knowing one command means knowing the next. Every command, flag, and argument has help text; `dsc --help` is the source of truth.
6. **Evidence-informed.** When available, real-use evidence and captured API signatures belong in the linked command spec.

## The surface

### Global options

- `dsc --config <path> <command>` (`-c`) selects a config file. Without it, `dsc` searches common local/user/system paths and falls back to `./dsc.toml`. See [config-path-resolution.md](commands/config-path-resolution.md).
- `dsc --dry-run <command>` (`-n`) is global - see [`--dry-run`](#--dry-run--n) below.
- Every command, subcommand, argument, and flag carries concise `--help` text. `dsc --help` is the authoritative, complete surface; the README showcases and indexes into [docs/](../docs/).

### Output: data on stdout, hints on stderr

The one rule that makes `dsc` composable: **stdout carries the result; stderr carries hints, progress, and errors.** A "did you mean…" note or a rate-limit wait message goes to stderr where it cannot corrupt a piped stream. Exit non-zero with a clear stderr message on any failure.

- A global `--format text|json|yaml`, honoured by **every** `* list` command at minimum; `text` is the default. Commands with richer needs may add `markdown`, `markdown-table`, `csv`, `urls`. Reuse the shared formatter helpers (`emit_result()` in `commands/common.rs`) rather than duplicating rendering.
- Single-value mutating commands still emit a small structured object under `--format json` (e.g. `{"topic_id":…,"post_id":…}`) so they script cleanly. In `text` mode, prefer a stable machine-usable line (a URL / id / path) over a prose sentence. (A shared `--quiet` for mutating commands is a planned addition.)

### Version and completions

`dsc version [discourse] [--format json]` is the canonical interface (a command, honouring `--format`); `--version`/`-V` remain the quick human/CI check. Completions are generated from the live `clap` command (`dsc completions <shell>` to stdout, `--dir` for packaging, `install` for humans) and must never drift from the surface. Both follow the house-style rules in `rust-cli.md`.

## Command patterns

The load-bearing patterns. New commands conform to these.

### Declarative resource sync: `pull` → edit → `push` (→ `diff`)

Anything managing stateful server resources follows the sync loop, not one-off setters. Exemplars: `setting pull/push/diff`, `tag pull/push`, `category pull/push`, `theme setting pull/push`, `theme field pull/push`.

- `pull` writes a local snapshot: YAML by default, JSON when the path ends `.json`. The file carries a `version: 1` schema integer and provenance (`pulled_at`, and the Discourse version where known). `push` rejects an unknown schema version.
- Server encodings awkward to hand-edit are expanded on pull and re-collapsed on push. Reference case: theme JSON-schema list settings (`header_links`) arrive as a string of escaped JSON and are expanded to a real YAML list; `theme field` bodies (SCSS/HTML) are written as raw text files.
- `push` is idempotent: read current server state, send **only the entries that differ**, and report the plan (changed / unchanged / unknown-on-server / reset). An untouched `pull → push` is a verified no-op.
- **Compare parsed values, never raw strings.** The file's serialisation differs from the server's (compact vs spaced, key order); string comparison makes everything look changed and defeats idempotency - the single most common bug in this pattern. This bit `theme setting push`; the fix was semantic (parsed-JSON) comparison. New sync commands must do the same.

### `--dry-run` / `-n`

Global flag. Every mutating command honours it and prints the **complete** plan it would execute (resolved names, the diff, the requests) while touching nothing - the review gate before acting on a production forum. If a command cannot yet construct such a plan, it fails non-zero before configuration resolution or any side effect; it must never silently execute. Read-only commands accept and ignore it so scripts can pass it uniformly. Provisioning flows (`backup setup-s3`) print the whole plan offline under `--dry-run`, including the exact external commands.

### Guard destructive / irreversible actions

- Refuse the dangerous case with a message that names the safe alternative: `theme delete` refuses the site default; `theme field push` refuses a git-backed remote and points at `theme update` / `theme duplicate`.
- Never print secrets: redact URL-embedded credentials (`https://***@host`) in output; an API secret is shown only on the one unavoidable creation line.
- Sequence multi-step provisioning "enable last" (set up dependencies, flip the activating switch at the end) so a mid-run failure leaves a clean disabled state - as used for `backup setup-s3` (`backup_location=s3` last) and reply-by-email.

### Every client capability has a command

If `src/api/*` exposes a method performing a distinct Discourse operation, a command reaches it. `delete_theme` once existed with no command wiring it - a dead capability. `pub fn` suppresses the dead-code lint, so audit periodically: each public client method should be referenced from `src/commands`/`main.rs`, directly or transitively.

## Consistency standards

The small, boring rules that keep the surface uniform.

### Error messages

- Missing config entry: `discourse not found: {name}`
- Missing remote resource: `{resource} not found: {identifier}` (via the shared `not_found()` helper - don't hand-roll per-command wording)
- Missing config value: `missing {field} for {resource}; please set {field} or check your config`
- Validate and fail fast (empty identifiers, obviously bad input) before making API calls.

### Empty lists

- `text`: print `No <resource> found.` (keep useful context where it helps, e.g. `No PMs found in {direction}.`)
- `json`/`yaml`: a normal empty array/object, never a magic string.

### Flag style

- Short flags are lowercase. Reuse established letters across commands: `-f`/`--format`, `-n`/`--dry-run`, `-p`/`--parallel`, `-b`/`--branch`.
- Equivalent cross-instance operations share flag semantics: `category copy` and `group copy` both take `--target`; `--tags` filters the same way in `setting set --tags` and `setting audit`.

### Command ordering

Subcommands sort alphabetically in `--help` (via `next_display_order = None`), so declaration order never leaks into the user-facing surface. Every command has an `after_help` "Examples:" block.

### Format-support baseline

Minimum `text|json|yaml` for every list command; preserve richer formats where already present; expand beyond the baseline only when a concrete need justifies it.

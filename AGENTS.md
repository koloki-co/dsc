# Notes for LLMs / agents using `dsc`

This file is for agents in **other sessions** who are using `dsc` as a tool to manage a real Discourse install. If you are contributing to `dsc` itself, read [spec/implementation.md](spec/implementation.md) instead.

## Get oriented fast

- `dsc --help` lists every top-level command (sorted alphabetically). Each command has its own `--help`, which ends with an `Examples:` block.
- [README.md](README.md) has the command index linking to per-command docs in [docs/](docs/).
- [docs/configuration.md](docs/configuration.md) covers `dsc.toml` and the env vars (`$DSC_CONFIG`, `$DSC_CONFIG_HOME`).
- `dsc config` shows the active config and where it came from. `dsc config check` probes every configured Discourse for API auth and SSH reachability - run it first on any new machine.
- **To discover site settings**, `dsc setting pull <discourse> <file>` writes a self-documenting catalog of *every* setting (value, default, type, category, and Discourse's own description) - the reference for what is available and adjustable. `dsc setting audit <key>` compares one setting's value across every configured forum.
- Every destructive command honours global `-n` / `--dry-run`. Use it before any push/set/install/restore.

## Pattern: pull → edit → push

Most state lives behind a `pull` / `push` pair: `topic`, `category`, `post`, `tag`, `setting`, `palette`, `theme`, `emoji`, `backup`. Snapshot to a local file, edit, push back. Snapshots are stable-sorted YAML (or JSON by file extension) so they diff cleanly under git.

`pull --changed-only` (where supported) keeps snapshots small and reviewable. `push --dry-run` always prints the plan with `~` (change), `=` (unchanged), `?` (unknown), `-` (reset).

## Bulk operations across forums

`dsc.toml` entries carry `tags`. Most read commands accept `--tags tag1,tag2` to filter; `dsc setting set --tags <tags> <key> <value>` writes across every matching install. Tag your installs by environment (`production`, `staging`), region, or customer.

## When you find a bug, tweak, or gap

Record it in [spec/roadmap.md](spec/roadmap.md), the single list of planned work. `dsc`'s author is actively shaping the tool around real-world use. Do not open an issue for a missing feature; write or update its spec and roadmap entry instead.

- **Bug** - add or update the roadmap entry with the exact command, expected and actual behaviour, and `dsc version`. Fix it directly when feasible.
- **Tweak or missing feature** - add a roadmap entry with the next unused stable `RXX` code, then add a linked command spec when the request needs design detail.
- **Real-install driver** - retain the observed API calls, redacted response, and Discourse version in the linked spec.

### Where to put it

Specs are two-tier (see [spec/README.md](spec/README.md)). A spec for a command or a discrete feature goes under [spec/commands/](spec/commands/), named slug-case after the command surface, mirroring `src/commands/` and [docs/](docs/): `spec/commands/theme-management.md`, `spec/commands/config-path-resolution.md`. Overarching, cross-cutting specs (config schema, CLI design, roadmap) stay at the [spec/](spec/) root. Don't suffix with `-spec.md` for new ones (legacy filenames have it; new ones don't need to). Every actionable request belongs in [spec/roadmap.md](spec/roadmap.md) under **Planned**, with the next unused stable `RXX` code.

### What makes a spec land fast

In rough priority order:

1. **The use case in one paragraph.** "I am doing X for a real Discourse; I need Y; today I work around it by Z." This is what unblocks design decisions.
2. **The exact API calls you are using as the workaround.** If you fell back to `curl` against the Discourse admin API, paste the request and a redacted response. This is gold - it removes the entire "what does the endpoint actually return?" discovery phase. Especially valuable for the Discourse admin API, which is not formally versioned.
3. **The proposed CLI surface.** What would the subcommand look like? Flags? Arguments? Look at how nearby commands are shaped (e.g. if you are proposing `dsc theme setting set`, mirror `dsc setting set`).
4. **Phasing.** If the work is multi-step, label phases by what is blocking you (`Phase 1 - blocking`, `Phase 2 - iteration ergonomics`, `Phase 3 - nice to have`). The author will probably ship Phase 1 first and revisit later phases on demand.
5. **Backward-compatibility note.** Does this change anything that currently works? Say so explicitly.
6. **Out of scope.** Just as important as the in-scope list. Pre-empts feature creep during implementation.

### Spec template

Copy this into a new file under `spec/`:

```markdown
# `dsc <area>` - <one-line summary>

Spec for <what>. Goal: <why>. Driver: <real-world use case, name it>.

## Motivation

<one paragraph: what are you trying to do, what does dsc not do today, how
are you working around it right now>

## Current state (as of YYYY-MM-DD)

<what `dsc <area>` does today; what's missing>

## Proposed CLI surface

```text
dsc <area> <verb> <args>   [flags]
```

<one bullet per new subcommand explaining behaviour, error cases, and
which Discourse API endpoint it maps to>

## Reference: API calls observed in the field

<paste actual requests/responses you used as the workaround, with secrets
redacted. Note the Discourse version you tested against.>

## Phases (if multi-step)

### Phase 1 - blocking

- [ ] ...

### Phase 2 - iteration ergonomics

- [ ] ...

### Phase 3 - nice to have

- [ ] ...

## Backward compatibility

<does anything that works today change? if so, what and why is it ok>

## Out of scope

<bullet list of things this spec deliberately does not cover>
```

### Good examples to copy

- [spec/commands/setting-sync.md](spec/commands/setting-sync.md) - clear phasing, file schema, dry-run output shown
- [spec/commands/theme-management.md](spec/commands/theme-management.md) - phased by what is blocking, names the real driver
- [spec/commands/config-path-resolution.md](spec/commands/config-path-resolution.md) - precedence chain table, explicit-selector safety section, tests list, non-goals

## What `dsc` will not do (and you should not file specs for)

- Anything that requires modifying the Discourse codebase itself (`dsc` is API + SSH + Docker only, no Ruby/JS).
- Authoring component source (SCSS/JS) - that belongs in the component's own repo.
- Holding secrets beyond what `dsc.toml` already does (no key-manager integration planned).
- Wrapping Discourse features that have no admin API (the CLI cannot exceed the API).

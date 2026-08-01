# dsc specs

Design specs for `dsc`, in two tiers.

## Overarching (this directory)

Cross-cutting documents that sit above any single command:

- [spec.md](spec.md) - internal spec: the `dsc.toml` config schema and the release/distribution rules.
- [cli-design.md](cli-design.md) - the normative CLI design philosophy: output/formats, the `pull → edit → push → diff` sync loop, `--dry-run`, destructive-action guards, error/empty-list/flag conventions. Anything about *how commands behave* lives here.
- [implementation.md](implementation.md) - the implementation plan and the working agreement for agents (commit discipline, keeping specs current, roadmap flow).
- [live-compatibility-tests.md](live-compatibility-tests.md) - R36's explicit opt-in, disposable-forum, serialization, and cleanup contract for real-Discourse tests.
- [roadmap.md](roadmap.md) - the single list of planned and in-progress work, with stable `RXX` reference codes for each actionable item. Shipped history is in [CHANGELOG.md](../CHANGELOG.md).

## Per-command ([commands/](commands/))

One spec per discrete feature or gap, named after the command surface it belongs to and mirroring `src/commands/` and [docs/](../docs/). A single command can own more than one spec when the work arrived in distinct pieces - for example `dsc category` has both [commands/category-workflow.md](commands/category-workflow.md) (the pull/edit/push loop) and [commands/category-definition-sync.md](commands/category-definition-sync.md) (syncing category *definitions*). Specs stay discrete rather than being merged, so each keeps its own driver, field-API reference, and phase checklist. Planned examples include [commands/explorer.md](commands/explorer.md) for Data Explorer query inspection and execution and [commands/backup-health.md](commands/backup-health.md) for S3 backup health.

## Conventions

- Retain a "Reference: API calls observed in the field" section in a spec when it is useful for reproducing an API-backed request (template in [../AGENTS.md](../AGENTS.md)). Record the Discourse version tested against - the admin API is not formally versioned.
- Every actionable item in [roadmap.md](roadmap.md) gets the next unused stable `RXX` code in the item title (`- [ ] **R12 - Title**`). Never renumber or reuse codes; keep the code when an item moves from planned to in-progress or done.
- Bugs, tweaks, and missing features all start in [roadmap.md](roadmap.md); do not create a second request list or use a GitHub issue for a feature gap.
- User-facing per-command usage lives in [docs/](../docs/), not here. Specs are design intent; docs are the reference.

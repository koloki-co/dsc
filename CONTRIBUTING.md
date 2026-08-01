# Contributing to `dsc`

Thanks for thinking about contributing. `dsc` is a small but actively-shaped project; the bar for landing changes is "useful, tested, narrow, and follows the existing patterns." This file points you at the right entry points and conventions.

## Quick orientation

| You want to … | Read |
|---|---|
| Build and test the project locally | [docs/development.md](docs/development.md) |
| Understand the CLI design standards | [spec/cli-design.md](spec/cli-design.md) |
| Use `dsc` to manage a real Discourse | [docs/](docs/) (per-command pages) |
| Use `dsc` from another LLM/agent session | [AGENTS.md](AGENTS.md) |
| File a feature request | See **"Filing issues"** below |
| Propose a substantial new command | Write a spec - see [AGENTS.md - "What makes a spec land fast"](AGENTS.md#what-makes-a-spec-land-fast) |

## Reporting bugs vs filing specs

- **Bug** — something `dsc` claims to do but does wrong. Open a [bug issue](.github/ISSUE_TEMPLATE/bug_report.md) with the exact command, expected vs actual, and `dsc version`.
- **Small feature request** — a flag, a tweak to existing output, a missing subcommand alias. Open a [feature issue](.github/ISSUE_TEMPLATE/feature_request.md).
- **Substantial new command or surface** — write a spec under [spec/](spec/) before code, using the [spec template in AGENTS.md](AGENTS.md#spec-template). Open a [spec issue](.github/ISSUE_TEMPLATE/spec_request.md) linking the draft if you'd like discussion first.

When in doubt, file the spec - it captures more context and the author can shrink it to an issue if it's smaller than it looks.

## Code contributions

### Setup

```bash
git clone https://github.com/koloki-co/dsc.git
cd dsc
cargo build
cargo test
s/install-hooks # optional but encouraged: enables the tracked pre-commit lint hook
```

### Conventions

- **Conventional commits.** `feat(area):`, `fix(area):`, `docs:`, `chore(deps):`, `ci(deps):`, etc. Used since v0.9; the `CHANGELOG.md` generator relies on this shape.
- **Tests.** Every new command path gets a unit test for the pure logic and (where it talks to Discourse) a test in `tests/` that exercises the public CLI. See `tests/topic-test.rs` for the shape.
- **Docs.** A new subcommand requires:
  - help text on the CLI (`#[arg(help = …)]` or doc-comment)
  - a section in the matching `docs/<command>.md`
  - a row in [README.md](README.md) "Documentation" if it's a new top-level command
- **Local hooks (optional but encouraged).** Run `s/install-hooks` once per checkout. The tracked pre-commit hook runs `s/lint` (fmt + strict Clippy); full tests remain separate so commits stay quick.
- **No version-bump commits without a feature** — `s/version++ patch` is for release-worthy changes, not docs polish. Pure docs commits use `docs:` prefix and no bump.

### Pull requests

- Keep PRs focused on one change.
- Run `s/test-fmt-clippy` before pushing; it mirrors CI's formatting, strict Clippy, and full test gate.
- Reference the issue or spec in the PR description.

## Project layout

```
src/
  cli.rs              # Clap derive structs - the CLI surface
  main.rs             # Top-level command dispatch
  api/                # Discourse HTTP client + response models
  commands/           # One module per top-level command
  config.rs           # dsc.toml resolution + parsing
docs/                 # User-facing per-command docs
spec/                 # Design specs (one per new surface)
tests/                # Integration tests (one file per command)
s/                    # Project scripts: build, test, docs, install, version++
.marcus/              # Author-private notes (gitignored)
```

## Support stance

`dsc` is best-effort, community-driven, no SLA. Add bugs, tweaks, and missing features to the [roadmap](spec/roadmap.md), the single list of planned work. Include the exact command, expected and actual behaviour, and `dsc version` for a bug; retain real-install evidence in the linked spec where it is useful. If you need a guarantee, hire a contractor.

## License

By contributing you agree your contribution is licensed under the same terms as the rest of the project ([GPL-2.0-or-later](LICENSE)).

# Roadmap

Planned and in-progress work for `dsc`. Shipped history lives in [CHANGELOG.md](../CHANGELOG.md); active field-driven specs (⭐) are indexed in [from-the-field.md](from-the-field.md) and should generally outrank speculative items.

Legend: [x] done, [~] in progress or partially done, [ ] not started. Stable roadmap codes (`R1`, `R2`, …) are never renumbered or reused.

## Shipped (highlights)

The built surface, grouped - see CHANGELOG for the full per-release detail.

- **Declarative sync** - `setting pull/push/diff`, `tag pull/push`/`rename`, `category pull/push` (front-matter routing, `--dry-run`, `--updates-only`, `--no-bump`/`--skip-revision`), plus `post`/`backup`/`emoji`/`topic` pull/push. Specs: [setting-sync](commands/setting-sync.md), [tag-sync](commands/tag-sync.md), [category-workflow](commands/category-workflow.md).
- **Theme management (complete)** - settings (incl. `pull/push`), fields (SCSS/HTML), assets (`set/unset`), enable/disable, attach/detach, palettes, `show`, remote `update`, API `install`/`delete`. Spec: [theme-management](commands/theme-management.md).
- **Compliance / cross-forum** - ⭐ `sar` (GDPR SAR export), `setting audit` (one setting across the fleet). Spec: [subject-access-request](commands/subject-access-request.md).
- **Content** - ⭐ `topic pull --full`, ⭐ `topic title`/`tags`, ⭐ `topic delete`/`restore`/`list --deleted`, negative-ID user-list fix. Specs: [topic-pull-full-thread](commands/topic-pull-full-thread.md), [topic-title-and-tags](commands/topic-title-and-tags.md), [topic-delete](commands/topic-delete.md), [user-list-negative-ids](commands/user-list-negative-ids.md).
- **Ops** - `update` (skip-if-current, rootless Docker, parallel), `harden` (PQ-hybrid SSH), ⭐ `backup setup-s3` Phase 1. Spec: [backup-s3-setup](commands/backup-s3-setup.md).
- **CLI / distribution** - universal `--format`, `completions install` (+ PowerShell), `man` pages, `version --format`, SIGPIPE-safe piping, config-path resolution, cargo-dist release + git-cliff changelog, `s/version++` one-command release, push/PR CI gate. Specs: [config-path-resolution](commands/config-path-resolution.md), [cli-design](cli-design.md).


## In progress

- [~] ⭐ **R10 - `category` Phase 5 link rewriting** - admonition conversion now ships as `--convert-admonitions=quote-callouts|plain-blockquote`; internal `--rewrite-links` remains. The Quote Callouts target requires the Arkshine theme component; the plain-blockquote target is portable and email-safe. Spec: [category-workflow](commands/category-workflow.md).

## 1.0 launch checklist

Required before announcing on [meta.discourse.org](https://meta.discourse.org). The stable `RXX` identifiers below are intentionally non-contiguous: completed items were removed rather than renumbered or reused.

### Release blockers

- [ ] **R35 - Record third-party asset provenance** - determine the licences and required notices for vendored Discourse/Font Awesome SVGs, then add REUSE/SPDX coverage and a regeneration/provenance record. Confirm the intended MIT exception for original `dsc` code/docs.
- [ ] **R38 - Make workflow security a blocking CI gate** - resolve Zizmor's release-workflow findings: eliminate the publish-time cache poisoning risk, replace or constrain cargo-dist's dynamic matrix shell fragments and container images, then add `zizmor --strict-collection .` as a blocking read-only CI job. REUSE validation follows R35's licence/provenance decision.

### Contract, documentation, and launch package

- [ ] **R36 - Isolate live compatibility tests** - make tests that contact Discourse explicit opt-in, disposable-resource based, serialised where needed, and cleanup-safe; retain offline tests as the ordinary local/CI gate.
- [~] **R37 - Post-1.0 CLI ergonomics** - make bare `dsc` exit successfully after showing its command summary, add shared `~` path expansion and Clap path hints, report `dsc version` without config resolution, and review cargo-binstall metadata and release checksum naming. HTTP client now has a 10s connect / 120s total timeout so a dead or misconfigured host fails fast instead of hanging the CLI.
- [ ] **R3 - Record an asciinema** (~30s) of the pull → edit → push → diff loop; embed in README.
- [ ] **R5 - Pre-circulate the Meta post** to a couple of Discourse regulars before posting.
- [ ] **R2 - Cut `v1.0.0`** from a fresh, clean, synchronised worktree after this checklist passes, with a release rehearsal (`s/test-fmt-clippy`, docs build, `cargo audit`, `cargo publish --dry-run`) and generated changelog review.

## Planned

### Docker app configuration ⭐

- [ ] ⭐ **R28 - `dsc app env` inspect, audit, and safe set** - read environment-variable names and non-secret values from Docker `app.yml`, audit one key across matching forums, then add guarded scalar edits with backup, dry-run, and optional rebuild. Driver: inspecting or raising `DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE` across the owned fleet. Spec: [app-environment](commands/app-environment.md).

### Content sync


- [ ] ⭐ **R29 - `dsc render` template placeholder substitution** - render local Markdown template files against per-forum variables from `dsc.toml` (`[template.vars]` globals, `[discourse.template]` per-forum, built-in `forum_baseurl`/`forum_name`/`forum_fullname`), so anonymised content templates are ready to push without manual find-and-replace. `--render` flag on `topic new`/`push`/`reply`/`category push` applies the same inline. Tera 2.0 engine. Driver: 24-template content-templates library in the discourses workspace. Spec: [template-rendering](commands/template-rendering.md).
- [~] ⭐ **R11 - `category` definition sync Phase 2/3** - Phase 1 shipped the blocking round-trip (`category def pull/push`, `category show/get/set`) for category definitions: description, permissions, position, topic template, and tag rules. Remaining work: rename, list `--append`/`--remove`, prune, and `def diff`. Spec: [category-definition-sync](commands/category-definition-sync.md).

### New command surfaces

- [ ] **R12 - `dsc chat`** - `chat channels` / `chat send <discourse> <channel> [<file>]` / `chat fetch <channel> [--since …]`. Mirrors the `topic`/`pm` split.
- [ ] ⭐ **R13 - `backup setup-s3` Phase 2/3** - `--reuse-user` (key rotation), `--use-iam-profile`, `--all`/`--tags`; then a native AWS SDK backend, `--retention` lifecycle, `backup status`. Spec: [backup-s3-setup](commands/backup-s3-setup.md).
- [ ] **R14 - `dsc install <name> --host <host>`** - declarative provisioning on a `dsc harden`-prepared box (templated `app.yml`, launcher bootstrap, poll `/about.json`, append to `dsc.toml`). Spec: [install](commands/install.md). Includes the remaining `harden` stage-3 items (timezone/swap/journald/unattended-upgrades/fail2ban/rootless-Docker/ufw - config keys wired, SSH execution + tests remain) and the `ssh_user`/`ssh_port` per-Discourse config fields `install` writes on success.

### Admin depth (demand-driven)

- [ ] **R39 - Emoji groups and bulk transfer** - investigate the new Discourse admin API for custom emoji group assignment and ZIP/CSV bulk import/export. If the API is available, add portable `dsc emoji` pull/push or import/export commands with manifest validation and dry-run support. Pinned picker groups are already configurable through the `emoji_picker_pinned_groups` site setting. Driver: [Discourse's July 2026 emoji groups and bulk import/export release](https://meta.discourse.org/t/pinned-emoji-groups-and-bulk-import-export-of-custom-emojis/408280).
- [ ] **R16 - `dsc report <name> [--period]`** - dashboard reports such as signups, DAU, posts, and likes; distinct from `analytics`.
- [ ] **R17 - `dsc webhook list|create|delete|ping`** - basic webhook administration.

### Cross-forum (the multi-install headline)

- [ ] **R19 - `dsc search all <query>`** - merged fan-out search.
- [ ] **R20 - `dsc report all <name>`** - aggregate a report across forums.
- [ ] **R21 - `dsc user find <email>`** - GDPR "which forum has this person" lookup.
- [ ] **R22 - `dsc backup create --all`** - reuse the `update all` parallel pattern for fleet backups.


## Stretch / exploratory

Speculative; build only on real demand. None are required for 1.0.

- [ ] **R24 - MCP server mode** - `dsc mcp serve` exposing commands as MCP tools. Overlaps `discourse-bawmedical-mcp` - consolidate vs coexist.
- [ ] **R25 - TUI** - `dsc tui` for interactive browsing. Big scope.
- [ ] **R26 - Config federation** - multiple config files + include-directives, for teams.
- [ ] **R27 - Discourse User API** - `dsc login` key-exchange for non-admin / scoped-bot auth; likely renames `api-key` → `admin-key`. Widens the *audience*, not the *capability* (most value needs admin scope regardless).

## Out of scope / removed

- ~~Shell completion *regeneration* as a tracked item~~ - superseded by the shipped `completions install`.
- ~~`dsc user password change`~~ - Discourse has no admin "set this password" endpoint by design; `user password-reset` covers the need.
- ~~`dsc user anonymize`~~ - rare enough for the Admin UI; not worth the destructive-confirmation UX.
- ~~`api-key create --scope`~~ - **parked 2026-06-29**. Scoped keys are low-value for `dsc` (nearly everything needs admin scope anyway) and blocked on an unconfirmed scoped-key `POST /admin/api/keys.json` body. Full-admin `api-key create` stays. Revisit on a concrete least-privilege consumer.

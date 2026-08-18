# Roadmap

The single list of planned and in-progress work for `dsc`. Checked entries may be implemented on `main` but not yet released; released history lives in [CHANGELOG.md](../CHANGELOG.md). Unreleased completed work is described as "implemented on `main`", not "shipped".

Legend: [x] done, [~] in progress or partially done, [ ] not started. Stable roadmap codes (`R1`, `R2`, …) are never renumbered or reused.

## Shipped (highlights)

The built surface, grouped - see CHANGELOG for the full per-release detail.

- **Declarative sync** - `setting pull/push/diff`, `tag pull/push`/`rename`, `category pull/push` (front-matter routing, `--dry-run`, `--updates-only`, `--no-bump`/`--skip-revision`), plus `post`/`backup`/`emoji`/`topic` pull/push. Specs: [setting-sync](commands/setting-sync.md), [tag-sync](commands/tag-sync.md), [category-workflow](commands/category-workflow.md).
- **Theme management (complete)** - settings (incl. `pull/push`), fields (SCSS/HTML), assets (`set/unset`), enable/disable, attach/detach, palettes, `show`, remote `update`, API `install`/`delete`. Spec: [theme-management](commands/theme-management.md).
- **Compliance / cross-forum** - `sar` (GDPR SAR export), `setting audit` (one setting across the fleet). Spec: [subject-access-request](commands/subject-access-request.md).
- **Content** - `topic pull --full`, `topic title`/`tags`, `topic delete`/`restore`/`list --deleted`, negative-ID user-list fix. Specs: [topic-pull-full-thread](commands/topic-pull-full-thread.md), [topic-title-and-tags](commands/topic-title-and-tags.md), [topic-delete](commands/topic-delete.md), [user-list-negative-ids](commands/user-list-negative-ids.md).
- **Ops / diagnostics** - `update` (skip-if-current, rootless Docker, parallel), `harden` (PQ-hybrid SSH), `backup setup-s3` and `backup health` Phase 1, saved Data Explorer query inspection/execution. Specs: [backup-s3-setup](commands/backup-s3-setup.md), [backup-health](commands/backup-health.md), [explorer](commands/explorer.md).
- **CLI / distribution** - universal `--format`, `completions install` (+ PowerShell), `man` pages, `version --format`, SIGPIPE-safe piping, config-path resolution, cargo-dist release + git-cliff changelog, `s/version++` one-command release, push/PR CI gate. Specs: [config-path-resolution](commands/config-path-resolution.md), [cli-design](cli-design.md).


## 1.0 launch checklist

Required before announcing on [meta.discourse.org](https://meta.discourse.org). The stable `RXX` identifiers below are intentionally non-contiguous: completed items may be retained as checked entries or summarised above, but are never renumbered or reused.

### Contract, documentation, and launch package

- [ ] **R3 - Record an asciinema** (~30s) of the pull → edit → push → diff loop; embed in README.
- [ ] **R5 - Pre-circulate the Meta post** to a couple of Discourse regulars before posting.
- [ ] **R2 - Cut `v1.0.0`** from a fresh, clean, synchronised worktree after this checklist passes, with a release rehearsal (`s/test-fmt-clippy`, docs build, `cargo audit`, `cargo publish --dry-run`) and generated changelog review.

## Planned

### CLI / distribution

- [x] **R45 - Zsh completion installer compatibility** - `dsc completions install` now writes Zsh completions to the shared `~/.zfunc` user-completion directory, matching the house convention and other Marcus Baw CLIs. Custom completion directories remain supported through `--dir`.
- [ ] **R47 - Theme-derived `dsc update` label colours** - replace the current deterministic hash-based ANSI label colour with an optional, validated per-Discourse key colour derived from its active theme/palette and cached in `dsc.toml`. Preserve `NO_COLOR`/`DSC_COLOR` behaviour and the hash fallback; the update workflow must not silently rewrite configuration. Driver: operators want multi-forum update output to visually match each forum's identity. Spec: [update-theme-colour](commands/update-theme-colour.md).

### Ops reliability

- [~] **R42 - `dsc update` failure detection and disk-guard recovery** - disk recovery is implemented: exact preflight measurement, safe dangling-image cleanup, re-measurement, and shell-quoted manual guidance with validated older `discourse/base` IDs. SSH failures now report exit status and bounded stdout/stderr context while logs retain only the concise diagnosis. Remaining investigation: reproduce why two 2026-07-29 launcher invocations returned non-zero after successfully updating `discourse_docker`; stderr itself was never the failure classifier. Spec: [update-failure-detection](commands/update-failure-detection.md).

### Docker app configuration

- [ ] **R28 - `dsc app` Phase 3 inventory** - low-priority follow-up: consider read-only inventory for selected non-`env:` `app.yml` keys (`templates`, `hooks`, `volumes`) if a concrete fleet need arises. The `app env` inspection, audit, and safe scalar edit workflow is complete. Spec: [app-environment](commands/app-environment.md).

### Content sync

- [~] **R44 - `dsc post info`** - Phase 1 shipped: read-only lookup from a post ID to minimal metadata and a canonical topic URL, including staff-visible soft-deleted posts and topics, returning post ID, topic ID, post number, URL, deletion state, and topic title/slug/category ID without emitting raw post content or author data. Driver: completed Reviewables and staff action logs identify spam posts but do not expose a URL suitable for Discourse AI spam-detector testing. Remaining: Phase 2 read-only `dsc reviewable list` surface, deferred pending a captured admin API response. Spec: [post-info](commands/post-info.md).
- [x] **R43 - Category content portability** - `category` topic-sync supports `--rewrite-links` plus Quote Callouts and plain-blockquote admonition conversion. Driver: MkDocs ↔ Discourse category content sync. Spec: [category-workflow](commands/category-workflow.md).
- [ ] **R29 - `dsc render` template placeholder substitution** - render local Markdown template files against per-forum variables from `dsc.toml` (`[template.vars]` globals, `[discourse.template]` per-forum, built-in `forum_baseurl`/`forum_name`/`forum_fullname`), so anonymised content templates are ready to push without manual find-and-replace. `--render` flag on `topic new`/`push`/`reply`/`category push` applies the same inline. Tera 2.0 engine. Driver: 24-template content-templates library in the discourses workspace. Spec: [template-rendering](commands/template-rendering.md).
- [~] **R11 - `category` definition sync Phase 2/3** - Phase 1 is released. On `main`, `category rename`, explicit live `category diff`, list-field `--append`/`--remove`, `required_tag_groups`, category type IDs, complete scalar custom-field maps, parent-by-name resolution, and up-front parent validation are implemented but not yet released. Remaining: `topic_title_placeholder`, logo/background assets, `icon`/`emoji`, same-file parent/child creation with cycle detection, and guarded `def push --prune`. Driver: York Music Marketplace needs the Discourse `support` category type to enable accepted answers, not an ignored `solved_enabled` category field. Spec: [category-definition-sync](commands/category-definition-sync.md).

### New command surfaces

- [ ] **R12 - `dsc chat`** - `chat channels` / `chat send <discourse> <channel> [<file>]` / `chat fetch <channel> [--since …]`. Mirrors the `topic`/`pm` split.
- [~] **R13 - `backup setup-s3` Phase 2/3** - `--use-iam-profile` (EC2 instance role, no static keys) and `--all`/`--tags` fleet fan-out are implemented on `main` but not yet released. Remaining: `--reuse-user` key rotation, then a native AWS SDK backend and `--retention` lifecycle. Spec: [backup-s3-setup](commands/backup-s3-setup.md).
- [x] **R41 - `dsc backup health` Phase 1** - fleet S3 evidence for newest backup timestamp/age, newest archive size, total bucket bytes, and object count, with stale/missing/inaccessible exit status. Later monitoring and remediation phases remain demand-driven in the linked spec. Driver: recurring manual `aws s3` checks to catch halted backups and unbounded bucket growth. Spec: [backup-health](commands/backup-health.md).
- [ ] **R14 - `dsc install <name> --host <host>`** - declarative provisioning on a `dsc harden`-prepared box (templated `app.yml`, launcher bootstrap, poll `/about.json`, append to `dsc.toml`). Spec: [install](commands/install.md). Includes the remaining `harden` stage-3 items (timezone/swap/journald/unattended-upgrades/fail2ban/rootless-Docker/ufw - config keys wired, SSH execution + tests remain) and the `ssh_user`/`ssh_port` per-Discourse config fields `install` writes on success.

### Admin depth (demand-driven)

- [x] **R46 - Palette push dry-run plan** - `dsc theme palette push --dry-run` validates the source, resolves an existing palette where relevant, prints the effective create/update changes and request sequence, and avoids all server and local writes. Driver: copying York Music's approved Marigold and Marigold Dark palettes safely before applying the live changes. Spec: [theme-management](commands/theme-management.md).
- [ ] **R39 - Emoji groups and bulk transfer** - investigate the new Discourse admin API for custom emoji group assignment and ZIP/CSV bulk import/export. If the API is available, add portable `dsc emoji` pull/push or import/export commands with manifest validation and dry-run support. Pinned picker groups are already configurable through the `emoji_picker_pinned_groups` site setting. Driver: [Discourse's July 2026 emoji groups and bulk import/export release](https://meta.discourse.org/t/pinned-emoji-groups-and-bulk-import-export-of-custom-emojis/408280).
- [ ] **R16 - `dsc report <name> [--period]`** - dashboard reports such as signups, DAU, posts, and likes; distinct from `analytics`.
- [x] **R17 - `dsc webhook list|create|delete|ping`** - basic webhook administration. Spec: [webhook](commands/webhook.md).
- [~] **R30 - `dsc notify who|skipped`** - Phase 1's `dsc group info --with-defaults` has shipped, surfacing a group's category/tag notification-level defaults straight from the existing group-show API response. Remaining: `dsc notify who`/`dsc notify skipped` read-only forensic inspection of `TopicUser`, `CategoryUser`, `TagUser`, and `SkippedEmailLog` records to answer "who is watching this topic and why did they get an email" without server or Data Explorer access. Driver: production notification cascade incident where the admin could not diagnose why specific users received emails for a dormant topic. Spec: [notification-forensics](commands/notification-forensics.md).

### Cross-forum (the multi-install headline)

- [x] **R19 - `dsc search all <query>`** - implemented on `main` but not yet released: merged fan-out search across every configured forum, printing one combined, forum-tagged result list. Continues past a single forum's failure (missing credentials, unreachable) so the rest of the fleet's results still land; exits non-zero if any forum could not be searched.
- [ ] **R20 - `dsc report all <name>`** - aggregate a report across forums.
- [x] **R21 - `dsc user find <email>`** - implemented on `main` but not yet released: GDPR "which forum has this person" lookup that fans out an admin user search across every configured forum and prints only forum, user ID, and username for exact case-insensitive email matches.
- [x] **R22 - `dsc backup create --all`** - implemented on `main` but not yet released: fans `backup create` out across every configured forum, continues past a forum that fails, and exits non-zero if any forum failed.
- [ ] **R48 - Fleet selector and output normalization** - replace `search all`'s reserved magic positional with explicit, consistent `--all`/`--tags` selectors while retaining compatibility; define one shared selector that rejects explicitly empty tag filters; align fleet mutation output and dry-run plans across `backup create`, `backup setup-s3`, search, and user lookup. Driver: PRs #94-#100 added useful fan-out commands with inconsistent target selection and output contracts.


## Stretch / exploratory

Speculative; build only on real demand. None are required for 1.0.

- [ ] **R24 - MCP server mode** - `dsc mcp serve` exposing a curated subset of commands as MCP tools, for clients that cannot spawn a binary. Blocked on one question: whether the official Discourse MCP already accepts an admin API key, since the whole case rests on it not doing so. Decisions already taken if it goes ahead: consolidate `discourse-bawmedical-mcp` into it rather than running two servers, ship the async runtime in a separate `dsc-mcp` crate so the CLI stays synchronous, default to read-only with writes opt-in, and reuse the existing dry-run and confirmation guards rather than re-implementing request building. Spec: [mcp-server-mode](commands/mcp-server-mode.md).
- [ ] **R25 - TUI** - `dsc tui` for interactive browsing. Big scope.
- [ ] **R26 - Config federation** - multiple config files + include-directives, for teams.
- [ ] **R27 - Discourse User API authentication** - add the user-approved device-code `dsc login/logout` flow and a per-forum User API auth profile after R36 provides isolated live compatibility tests. Initial candidate surface: read/search/pull, notifications, own PMs, uploads/invites, and content the authorizing user may create or edit; moderator/admin compatibility follows only after a disposable live endpoint/role spike. Existing Admin API auth remains supported with no silent privilege fallback. Spec: [user-api-authentication](commands/user-api-authentication.md).

## Out of scope / removed

- ~~Shell completion *regeneration* as a tracked item~~ - superseded by the shipped `completions install`.
- ~~`dsc user password change`~~ - Discourse has no admin "set this password" endpoint by design; `user password-reset` covers the need.
- ~~`dsc user anonymize`~~ - rare enough for the Admin UI; not worth the destructive-confirmation UX.
- ~~`api-key create --scope`~~ - **parked 2026-06-29**. Scoped keys are low-value for `dsc` (nearly everything needs admin scope anyway) and blocked on an unconfirmed scoped-key `POST /admin/api/keys.json` body. Full-admin `api-key create` stays. Revisit on a concrete least-privilege consumer.

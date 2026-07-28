# Live compatibility test isolation

> **Status: R36 complete. The corrected harness passed two complete runs plus forced-failure cleanup against Discourse `2026.7.0-latest` (`216dc56395c9c678c36e772a8bbf3ed04b1e7fcb`) on 2026-07-28.**

Cross-cutting specification for tests that contact a real Discourse. Goal: retain useful compatibility evidence without allowing ordinary tests, concurrent runners, malformed configuration, or failed assertions to mutate an unintended forum or leave test resources behind.

## Motivation

The original integration suite mixed offline CLI tests with live API and SSH tests. Live tests silently passed when configuration was absent or malformed, many read-only tests posted an unrelated marker reply, and several mutation tests overwrote shared topics or left categories, groups, topics, themes, emoji, and backups behind. `--test-threads=1` reduced collisions inside one test process but did not stop two runners from targeting the same forum.

R36 makes the safety boundary structural and fail-loud. Less live mutation coverage is preferable to unsafe or misleading coverage.

## Test tiers

| Tier | Invocation | Network and mutation contract |
|---|---|---|
| Offline | `cargo test` or `s/test-fmt-clippy` | No configured Discourse contact. Runs in ordinary local work and CI. |
| Live compatibility | `DSC_LIVE_TESTS=1 TEST_DSC_CONFIG=/absolute/path s/test-live` | Explicitly ignored tests run against a declared disposable forum under one cross-process lock. Reads are allowed; writes require unique markers and recoverable cleanup. |
| Whole-instance destructive | Manual runbook only | Backup creation/restore and similar operations require an ephemeral instance that is destroyed or reprovisioned afterward. They are not part of `s/test-live`. |

Every test that calls `test_discourse()` MUST carry `#[ignore = "live compatibility test; run through s/test-live"]`. `tests/live-harness-test.rs` scans integration-test source and fails ordinary CI if that boundary is omitted.

## Runner contract

`s/test-live` is the only supported live-test entry point. It:

1. Requires exactly `DSC_LIVE_TESTS=1`.
2. Requires `TEST_DSC_CONFIG` to be an absolute readable file.
3. Canonicalizes the config path and atomically acquires a cross-process lock for it under `${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}`; an existing lock without a readable live owner is never stolen automatically.
4. Exports an internal runner marker, a unique run ID, and a private persistent resource journal keyed by the config path.
5. Runs a fail-loud configuration preflight and stale-resource sweep.
6. Runs ignored tests with `--test-threads=1 --nocapture`.
7. Runs cleanup postflight even when the suite fails; leaked resources are removed and make the run fail.
8. Releases the process lock on normal exit or a handled signal.

The lock and resource journal are keyed by canonical config path, while the journal itself records the normalized forum URL and exact per-resource marker. One live config must therefore remain the canonical owner of its forum; do not create multiple config files pointing at the same endpoint and run them concurrently. Retargeting a path with preserved cleanup state fails before touching the new forum. Normal exits remove an empty journal. A failed postflight or handled interruption preserves it for the next preflight. A stale lock is reported for manual inspection instead of being reclaimed through a race-prone PID-file check.

Direct `cargo test -- --ignored` does not satisfy the contract. When live mode is enabled, the Rust loader refuses to proceed unless the runner marker and run ID supplied by `s/test-live` exist.

## Configuration contract

Live configuration is a dedicated, ignored credential file, not an ordinary production `dsc.toml`:

```toml
version = 1

[[discourse]]
name = "demo"
baseurl = "https://demo.example.com"
apikey = "<admin API key>"
api_username = "system"
disposable = true

test_topic_id = 123
test_category_id = 456
test_color_scheme_id = 789
test_group_id = 321
test_theme_id = 654

ssh_enabled = false
backup_enabled = false
```

The loader fails before a test request when:

- `version` is not 1.
- No forum is configured.
- A forum does not explicitly set `disposable = true`.
- Name, HTTPS base URL, API key, or API username is empty.
- Any required fixture ID is absent or zero.
- SSH is enabled without both `ssh_host` and `changelog_topic_id`.
- On Unix, the credential file is accessible by group or other users; mode 0600 or stricter is required.

The live preflight also requires the configured API user to be an administrator and the forum's `can_permanently_delete` site setting to be enabled. Discourse permits force-destruction only after an item is soft-deleted and imposes a five-minute safety window when the same administrator performs both operations.

Optional capability tests print `[live:skip]` with the missing capability instead of becoming invisible vacuous passes.

`ephemeral = true` is reserved for a future whole-instance destructive runner. It does not enable backup creation or restore in `s/test-live`.

## Resource ownership and cleanup

All created content uses the reserved `dsc-live-<run-id>-...` marker namespace.

- Topic push, sync, title, and deleted-list tests create a unique topic in `test_category_id`; a guard is armed before creation, records the topic and first-post IDs as soon as they are known, and falls back to marker discovery if the create response is lost.
- Update changelog tests arm marker-based reply cleanup before invoking the command, so output parsing or assertion failures cannot lose ownership of the post.
- Guards record resources atomically before soft deletion. Postflight polls `/posts/{id}/permanently_delete_check.json`, waits out Discourse's safety window when necessary, revalidates forum/category/marker ownership, uses `force_destroy=true`, confirms the resource is no longer fetchable, and only then removes its journal entry.
- Preflight consumes any journal preserved by an interrupted prior process, scans active topics in the fixture category, and enumerates soft-deleted topics with `/latest.json?status=deleted&category={id}` before purging marker matches.
- Postflight removes and reports any marked resource missed by a test guard, then fails the suite. Journalled resources are expected cleanup work rather than leaks.
- Cleanup errors fail a non-panicking test. During an existing panic they are reported without causing a double panic; postflight remains the final check.

Shared fixture mutations are forbidden in the standard live tier. The suite may read the configured topic, category, group, theme, and palette, but it must not overwrite them.

## Deliberately reduced live mutations

The following old tests were unsafe and are intentionally narrowed until a cleanup-capable API and a real compatibility need justify restoring mutation coverage:

- Category and group copy run as live dry-run plans; they do not leave copied resources.
- Category push runs as a live dry-run plan; it does not create topics.
- Theme pull/push performs the pull and previews the push; it does not update a shared theme.
- Palette live coverage is pull-only; it does not update a shared color scheme.
- Theme install, theme setting/toggle, plugin install/remove, notification read, topic reply, and setting set dry-run behavior run offline.
- Custom emoji upload mutation is removed because no confirmed deletion primitive exists in `dsc`.
- Theme duplicate mutation is removed because cleanup could not be prearmed reliably from the current command contract.
- Backup create and restore are removed from automation. Create leaks storage asynchronously; restore replaces the whole forum and belongs to an ephemeral-instance runbook.

These are honest coverage gaps, not silent skips. Read compatibility and offline behavior remain tested.

## Ordinary-suite isolation

Ordinary tests do not discover or load a live config. CI and release gates additionally set `DSC_LIVE_TESTS=0`. Integration fixtures must not depend on public Internet hosts merely to exercise fallback behavior; use invalid non-URL values, `example.invalid`, or a local deterministic fixture as appropriate.

## Verification required to close R36

- [x] Live tests are structurally ignored and policy-tested.
- [x] Opted-in malformed or unsafe configs fail loudly.
- [x] The config declares a disposable forum and uses private permissions.
- [x] The runner serializes tests and holds a cross-process config lock.
- [x] Standard live mutations use unique markers and prearmed cleanup.
- [x] Unsafe/unrecoverable mutations are dry-run, read-only, offline, or removed.
- [x] Preflight recovers active and soft-deleted marked resources after an interrupted process.
- [x] Postflight fails on leaked marked resources and permanently removes all journalled or discovered resources.
- [x] Ordinary test fixtures make no external Discourse requests.
- [x] Run the corrected complete suite twice against the disposable demo forum and record the Discourse version.
- [x] Force one post-creation assertion failure and verify Drop/postflight cleanup.
- [x] Start a second runner against the same config and verify lock refusal.

Observed evidence:

- On 2026-07-27, two consecutive complete `s/test-live` runs passed and a simultaneous second runner was refused while PID `3192853` held the config lock. Later review found that the apparent zero-resource cleanup was false evidence: `permanent=true` was ignored by Discourse, and `/search.json?q=status:deleted` does not discover deleted topics.
- On 2026-07-28, the corrected preflight found and permanently removed six soft-deleted topics left by those earlier runs. Deleted-topic list/restore then passed live through `/latest.json?status=deleted`, and postflight waited for the same-admin safety window before force-destroying its journalled topic.
- On 2026-07-28, `DSC_LIVE_TEST_FORCE_FAILURE=topic_push s/test-live topic_push` failed after creating and updating its disposable topic; corrected postflight retained the journal, waited for permanent-delete eligibility, force-destroyed the topic, and passed.
- On 2026-07-28, complete runs `20260728T100116Z-3874593` and `20260728T100842Z-3882464` both passed. Each run journalled six created topics/posts, waited for permanent-delete eligibility, revalidated forum/category/marker ownership immediately before deletion, force-destroyed every resource, confirmed each was absent, and left postflight clean.
- An offline process-level regression confirms a runner never removes an existing lock whose PID has not yet been published, closing the prior `mkdir`/PID-file race.
- Palette compatibility now consumes the current bare list response, rejects unknown success shapes, includes negative built-in IDs, sends `{name, hex}` color rows, and re-fetches before update so unchanged resolved values are not persisted as overrides.

A recurring hosted CI job is not required: the forum credentials and destructive authorization remain maintainer-controlled, while `s/test-live` provides the reproducible release-rehearsal command.

## Backward compatibility

Ordinary `cargo test` behavior only becomes safer and faster because live cases are ignored structurally. Existing live-test files must add `version = 1`, `disposable = true`, all required fixture IDs, and private file permissions before the new runner accepts them. This intentional break prevents an old ambiguous config from targeting a production forum.

## Out of scope

- Provisioning or destroying an ephemeral Discourse instance.
- Automated backup restore testing.
- Restoring mutation coverage without a prearmable cleanup strategy.
- Running live credentials in ordinary pull-request CI.

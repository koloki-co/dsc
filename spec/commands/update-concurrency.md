# `dsc update` - leaner parallelism flag + don't collide with an in-flight rebuild

> **Status: implemented (unreleased).** Two refinements to `dsc update`
> surfaced during a fleet update (`dsc update all -p -m 2 -y`): a cleaner parallel
> flag (`-p [N]`, `-m` removed), and - more importantly - a pre-flight that stops
> `dsc` from stomping a rebuild already running on a host. Both verified on
> koloki-demo (a held-open fake `./launcher rebuild` process; `dsc update` skips
> the forum before any OS op, exit 0).

Driver: the Koloki fleet update on 2026-07-01. One forum (igkt) failed to rebuild (an unrelated leftover MySQL import template), and while supervising a manual `./launcher rebuild app` in another terminal, re-running `dsc update all` would have been unsafe - see Motivation.

## 1. Leaner parallel flag: `-p [N]`

Today parallelism is two coupled flags: `-p` / `--parallel` (a bool) plus `-m` / `--max <N>` (worker count, default 3), with a runtime guard that `--max` requires `--parallel`. `dsc update all -p -m 2` is clunky.

Fold them into one optional-value flag, matching the `make -j` / `cargo -j` convention:

```text
dsc update all              # sequential
dsc update all -p           # parallel, default width (3)
dsc update all -p 5         # parallel, 5 workers
```

- `parallel: Option<usize>` via clap `num_args = 0..=1`, `default_missing_value = "3"`, `value_parser` range `1..` (so `-p 0` is rejected for free - one manual guard gone).
- `--max` / `-m` is removed. Pre-1.0, so this is an acceptable breaking flag change; note it in the changelog.
- Dispatch simplifies: `parallel.is_some()` ⇒ parallel mode with that width; the "max requires parallel" and "max ≥ 1" checks disappear. `update_all` takes `Option<usize>` (None = sequential, Some(n) = n workers). Single-forum `dsc update <name> -p` still errors ("parallel only applies to `update all`").
- **Caveat (document in `--help`):** optional-value flags need the forum name *before* `-p`. `dsc update all -p 2` is unambiguous; `dsc update -p all` would make clap read `all` as the worker count and error loudly (not silently). Conventional ordering avoids it.

## 2. Don't collide with an in-flight rebuild

### Motivation

`dsc update`'s order is **OS update → `sudo reboot` → wait for SSH → check-if-current → `./launcher rebuild app`**. So if a rebuild is already running on a host (a supervised manual `./launcher rebuild`, or another `dsc` run) and you start `dsc update` against it, `dsc`'s *first destructive act is to reboot the box* - killing the in-flight rebuild and possibly leaving a half-bootstrapped container. This is a real hazard, not just an ergonomic wart. The user's workflow - "safely re-run `dsc update all` while I have a supervised rebuild going in another terminal" - must skip the busy host, not reboot it.

This is distinct from the existing **skip-if-current** gate (`is_discourse_up_to_date`, which compares the running commit to the latest commit on the configured branch on GitHub and skips the *rebuild* when nothing is stale). That gate is about *staleness*; this one is about *concurrency*. They compose.

### Behaviour

- **Pre-flight check at the very top of `run_update`, before the OS update / reboot.** SSH in and detect an in-progress rebuild. Simplest reliable signal is the supervising launcher process, which stays alive through the whole bootstrap:

  ```text
  pgrep -f 'launcher rebuild'
  ```

  (Catches a rebuild started by a human or by another `dsc` run. `dsc`'s own rebuild uses `./launcher rebuild app`, so a second `dsc` correctly detects the first and skips.)
- If detected → **skip the whole forum** (do not OS-update, do not reboot, do not rebuild) and report it distinctly, separate from the staleness skip:
  - `igkt: rebuild already in progress - skipped`
  - vs the existing `already at the latest <branch> commit - skipping rebuild`
- **On by default** - you never want two rebuilds or a reboot-during-rebuild. A `--force` escape hatch overrides for the rare "I know, proceed anyway" case.
- Cost: one cheap SSH round-trip per forum up front. Worth it.

### Reporting

`update all` should tally the three outcomes clearly so a fleet run is legible: **updated**, **skipped (up to date)**, **skipped (rebuild in progress)**, plus any **failed**. This also makes `dsc update all -p` self-safe if two workers or two invocations ever target the same host.

### Fleet failure policy

- Sequential `dsc update all` is strictly stop-on-first-failure: the next forum is not started until the current forum has completed and passed verification.
- Parallel mode can have up to the selected width already in flight. On the first failure, workers stop admitting queued forums, while already-started updates are allowed to finish so `dsc` does not detach their SSH sessions or abandon their result records.
- A launcher SSH exit 255 is never retried automatically. It is ambiguous whether the remote mutation started, so retrying could overlap an active rebuild.

## Phases

- [x] **Phase 1:** `-p [N]` folding (removed `-m`); dispatch + `update_all` signature simplified; `--help` notes the arg ordering. `-p 0` rejected; `-p` on a single forum rejected.
- [x] **Phase 2:** pre-flight rebuild-lock check at the top of `run_update` via `REBUILD_CHECK_CMD` (`pgrep -f '[l]auncher rebuild'`, bracketed to avoid self-match, always exits 0); returns `UpdateOutcome::SkippedRebuildInProgress`; `--force` override. Verified live on koloki-demo.
- [x] **Phase 3:** stop parallel queue admission after the first failure while allowing already-started forums to finish. Each attempted forum retains its own update-log outcome; queued forums remain untouched for a later resumable pass.

## Out of scope

- A dsc-owned lock file. It would only coordinate dsc-with-dsc, not dsc-with-a-manual-rebuild, which is the actual need; process detection covers both.
- Killing / joining an in-flight rebuild. `dsc` observes and steps aside; it does not attach to or tail someone else's rebuild.

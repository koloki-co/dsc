# `dsc update` failure detection and disk-guard recovery

> **Status: R42 active after a reproduced post-reboot race on 2026-09-23. Disk recovery and SSH diagnostics are implemented; reboot transition verification, final service verification, and stop-on-failure fleet admission are now implemented on `main`. Durable detached rebuild jobs remain a future reliability phase.**

The field report came from a 13-forum fleet run on 2026-07-29 (`dsc 0.12.1`) taking every managed forum from Discourse `2026.7.0-latest` to `2026.8.0-latest`. The disk guard was a confirmed `dsc` logic error. Source comparison established that v0.12.1 and current code both classified SSH success by exit status; the proven diagnostic defect was that a non-zero command discarded stdout and its exit status, then displayed ordinary git stderr as though it were the reason for failure. Why the historical launcher invocation returned non-zero remains an investigation rather than a resolved false-failure classification.

Related specs: [update-concurrency](update-concurrency.md), [update-log](update-log.md).

## Defect 3 - reboot disconnect skipped readiness and raced the rebuild

### Field report

On 2026-09-23, `dsc 0.19.0 update all -y` repeatedly failed on `bawmedical` immediately after the OS reboot:

```text
[Baw Medical Discourse [bawmedical]] Rebooting server
[Baw Medical Discourse [bawmedical]] Waiting for server to come back online
[Baw Medical Discourse [bawmedical]] Checking if Discourse update is needed
[Baw Medical Discourse [bawmedical]] Running Discourse update
Error: Discourse rebuild failed on bawmedical (ssh exit 255)
stderr (tail):
kex_exchange_identification: read: Connection reset by peer
```

An immediate rerun then failed its first SSH command with `Connection refused`; waiting before retrying allowed the same host to update normally. Expected: `dsc` should reconcile the ordinary SSH disconnect caused by reboot and should not start launcher until the new boot is stably reachable. Actual: readiness ran only when the reboot SSH process exited zero. A reboot that accepted the command and reset SSH with exit 255 silently skipped the entire wait. Even an exit-zero reboot could race because the first probe could reach the old boot before shutdown began.

### Resolution

- [x] Read `/proc/sys/kernel/random/boot_id` before reboot.
- [x] Submit reboot exactly once. Treat SSH exit 255 as indeterminate, not as success or as a reason to retry the mutation.
- [x] Require a different boot ID and three consecutive successful probes of that same new boot before any GitHub check or launcher invocation.
- [x] Fail before rebuilding if the boot transition cannot be confirmed within the bounded wait.
- [x] Require final Discourse version retrieval rather than logging success with an unknown post-update state.
- [x] When GitHub supplied a newer target, fail verification if launcher exited zero but the running core commit remained unchanged.
- [x] Keep sequential fleet updates stop-on-first-failure. In parallel mode, stop admitting queued forums after the first failure and wait for already-started work to finish.

Raw launcher exit 255 remains deliberately non-retryable: SSH cannot distinguish a command that never started from one still running after the connection was lost. A future durable-job phase may detach launcher under a dsc-owned operation ID with atomic remote status and logs, allowing safe reconnection without duplicate submission.

### Verification

- [x] Unit tests reject the old boot ID, require repeated observations of one new boot ID, and reset stability after a failed probe or another boot transition.
- [x] Unit tests accept truncated matching commit hashes and reject a successful rebuild whose commit remained unchanged.
- [ ] Process test: a stateful fake SSH endpoint returns exit 255 from reboot, then old/refused/new boot-ID observations, and proves launcher starts exactly once only after stability.
- [ ] Live verification on the next non-critical fleet update.

## Reported failure 1 - misleading SSH failure diagnostics

### Field report

Two forums (`bawmedical`, `rcpch`) were recorded as `failed` with no version transition. The `detail` field persisted to the update log was git's ordinary fetch summary:

```
ssh command failed for bawmedical: From https://github.com/discourse/discourse_docker
   e7f1201..7d4fa59  main              -> origin/main
 * [new branch]      build-cache       -> origin/build-cache
 * [new branch]      chrisr/pg18-image -> origin/chrisr/pg18-image
 * [new branch]      dependabot/bundler/image/setup_wizard/excon-1.5.0 -> ...
```

Git writes fetch and progress reporting to **stderr** on success. This is normal, documented behaviour, not an error channel. The current `run_ssh_command_with_tail()` helper already decides failure from the child exit status, so the exact path that produced this report must be reproduced before treating stderr handling as the root cause.

### Reported impact

`dsc` aborted during the launcher invocation. Post-hoc inspection confirmed that launcher's `discourse_docker` pull stage had succeeded - both working trees were correctly at `7d4fa59` and level with `origin/main` - while Discourse itself remained on the old version after the OS update and reboot. The non-zero SSH status proves that some launcher stage failed or was interrupted, but the old diagnostic discarded stdout and the exit code, so the captured git stderr cannot identify which stage.

Both succeeded on retry with no intervention. `discourse_docker` was already current and emitted no fetch summary on the retry, but that correlation is not proof that stderr caused the original non-zero exit.

The other 11 forums did not record the same diagnostic. The available log cannot establish whether their launcher output or execution path differed in any other material way.

### Resolution

Failure detection for remote steps continues to key on the **exit status** of the remote command, never on the presence or content of stderr.

- [x] Capture stdout and stderr separately and treat stderr as diagnostic context, not as a failure signal.
- [x] Report the failed step and SSH exit status, retaining bounded tails from both streams for terminal diagnosis.
- [x] Persist only the concise first-line failure in the update log rather than flattening raw command output into `detail`.
- [x] Retain the existing exit-status classifier; no output-content heuristic was added.
- [ ] Reproduce the historical launcher non-zero exit or retain enough evidence from a future occurrence to identify the failing launcher stage.

### Verification

- [x] Unit test: a remote step returning exit 0 with non-empty stderr is classified as success.
- [x] Unit test: a remote step returning non-zero with empty stderr is classified as failure and names its exit status.
- [x] Regression test: simulated `git fetch` output containing `* [new branch]` lines and a `..` range summary does not produce a failure classification.
- [x] Regression test: a failed step retains substantive stdout alongside git progress from stderr.

## Defect 2 - the disk guard cannot self-heal

### Observed

`rcpch` refused to start:

```
Error: insufficient disk space on rcpch: 4G free (minimum 5G).
Please run an interactive update via SSH to clean up space, then retry.
```

The host had 3.9G free (87% used). The cause was accumulated Docker images: 13.64GB total, 12.98GB (95%) reclaimable, six images with only two in use - stale `discourse/base` layers from prior rebuilds.

### Impact

Before R42, the guard ran **before** the update while cleanup ran **after** it. A host below the threshold therefore could not recover through `dsc`, no matter how many times it was run, and the error directed the operator to SSH.

Manual remediation on this host recovered 3.9G to 12G free (87% to 61%) - comfortably above the threshold - using only operations `dsc` already knows how to perform.

Note that `./launcher cleanup` alone is not always sufficient. It did not reclaim the stale `discourse/base` images; targeted `docker rmi` by image ID was required. Post-update `cleanup` on this same host reclaimed only 660MB.

### Requirement

When the pre-flight disk check fails, `dsc` should attempt recovery before refusing.

- [x] On insufficient space, run the cleanup step **first**, re-measure, and proceed if the threshold is then met. Report both measurements in the summary.
- [x] Use a fixed `docker image prune -f` for preflight rather than reusing the configurable, potentially broader post-update cleanup hook. Re-measure immediately and proceed if the threshold is met.
- [x] If cleanup is insufficient, preserve the newest listed `discourse/base` ID and print validated older IDs as manual no-force `docker rmi` candidates. Do not remove tagged base images automatically because creation order alone does not prove they are unused.
- [x] Never run `docker system prune -a` automatically. If recovery fails, print shell-quoted rootful/rootless inspection commands and exact candidate `docker rmi` commands where available, with explicit verification and no-force warnings.
- [x] Mention journal usage inspection as policy-dependent manual follow-up rather than deleting logs automatically.
- [x] Only refuse if space is still insufficient **after** recovery, and report initial, final, reclaimed, and minimum values.
- [x] Report post-update disk usage prominently enough to act as an early warning. A second forum in the same run finished at 82% used with 6.9G free - above the guard, but on the same trajectory.

### Verification

- [x] Unit test: a host below the threshold that rises above it after cleanup proceeds with the update.
- [x] Policy test: a host still below the threshold refuses with both measurements and safe manual commands.
- [x] Policy test: low-space output can carry older validated base-image IDs without automatically removing them.
- [x] Unit test: Docker image IDs are validated and deduplicated before interpolation into a remote command.
- [x] Unit test: rootful/rootless command generation never uses `prune -a`.
- [x] Process test: streamed command handling uses the real child exit status and retains separate stdout/stderr context.

## Out of scope

- Changing the 5G threshold itself. The value was not the problem.
- The move from rounded whole-GiB `df -BG` output to 1 KiB block accounting enforces the existing 5 GiB threshold accurately; hosts just below 5 GiB no longer round up and pass.
- Automatic scheduled cleanup independent of `update`. If fleet disk drift needs its own surface, that is a separate item alongside R41 (`backup health`) rather than part of `update`.

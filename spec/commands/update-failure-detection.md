# `dsc update` failure detection and disk-guard recovery

> **Status: Specification for R42. Disk-guard recovery is unimplemented; the reported stderr failure path needs reproduction and regression coverage.**

The field report came from a 13-forum fleet run on 2026-07-29 (`dsc 0.12.1`) taking every managed forum from Discourse `2026.7.0-latest` to `2026.8.0-latest`. The disk guard is a confirmed `dsc` logic error; the reported stderr failure path remains an investigation until it is reproduced.

Related specs: [update-concurrency](update-concurrency.md), [update-log](update-log.md).

## Reported failure 1 - git's stderr progress output needs reproduction

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

`dsc` aborted the run before `./launcher rebuild app`. Post-hoc inspection confirmed the pull had in fact **succeeded** - both `/var/discourse` working trees were correctly at `7d4fa59` and level with `origin/main`. The OS update and reboot had also completed. The forums were therefore left in a half-updated state: current `discourse_docker`, rebooted host, but Discourse still on the old version, and the log asserted a failure that had not happened.

Both succeeded on retry with no intervention, because `discourse_docker` was already current so the fetch emitted nothing. That retry-succeeds-without-change signature is diagnostic of this class of bug.

The 11 forums that did not exhibit it were, on the evidence, those whose fetch produced no new-branch summary.

### Requirement if reproduced

Failure detection for remote steps must key on the **exit status** of the remote command, never on the presence or content of stderr.

- Capture stdout and stderr separately; treat stderr as diagnostic context, not as a failure signal.
- Ensure the remote command's exit status is what propagates. Where steps are chained or piped over SSH, set `pipefail` or otherwise guarantee the meaningful status is not masked by a later element of a pipeline.
- Where a step's success genuinely cannot be read from exit status, assert a **positive postcondition** (for example, `git rev-parse HEAD` equals `origin/main`) rather than inferring failure from output text.
- Never persist raw stderr into the log's `detail` field as though it were an error message. If a step fails, `detail` should state which step failed and its exit status, with output attached as context.

### Verification

- Unit test: a remote step returning exit 0 with non-empty stderr is classified as success.
- Unit test: a remote step returning non-zero with empty stderr is classified as failure.
- Regression test: simulated `git fetch` output containing `* [new branch]` lines and a `..` range summary does not produce a failure classification.

## Defect 2 - the disk guard cannot self-heal

### Observed

`rcpch` refused to start:

```
Error: insufficient disk space on rcpch: 4G free (minimum 5G).
Please run an interactive update via SSH to clean up space, then retry.
```

The host had 3.9G free (87% used). The cause was accumulated Docker images: 13.64GB total, 12.98GB (95%) reclaimable, six images with only two in use - stale `discourse/base` layers from prior rebuilds.

### Impact

The guard runs **before** the update; `./launcher cleanup` runs **after** it. A host that drifts below the threshold therefore can never recover through `dsc`, no matter how many times it is run, even though the very operation that would free the space is already part of the workflow. The tool's own error text concedes this by directing the operator to SSH.

Manual remediation on this host recovered 3.9G to 12G free (87% to 61%) - comfortably above the threshold - using only operations `dsc` already knows how to perform.

Note that `./launcher cleanup` alone is not always sufficient. It did not reclaim the stale `discourse/base` images; targeted `docker rmi` by image ID was required. Post-update `cleanup` on this same host reclaimed only 660MB.

### Requirement

When the pre-flight disk check fails, `dsc` should attempt recovery before refusing.

- On insufficient space, run the cleanup step **first**, re-measure, and proceed if the threshold is then met. Report both measurements in the summary.
- Prefer targeted removal of unused `discourse/base` images by ID over `docker system prune -a`. A blunt prune also discards the current base image, forcing an unnecessary multi-gigabyte re-download on the next rebuild. Images that are parents of an in-use image will correctly refuse deletion and must not be treated as an error.
- Consider offering journal vacuuming as part of recovery; journald had grown to 815MB on the observed host and vacuuming to 200MB freed a further 624MB.
- Only refuse if space is still insufficient **after** recovery, and say so explicitly ("cleanup reclaimed N, still below minimum").
- Report post-update disk usage prominently enough to act as an early warning. A second forum in the same run finished at 82% used with 6.9G free - above the guard, but on the same trajectory.

### Verification

- Unit test: a host below the threshold that would rise above it after cleanup proceeds with the update.
- Unit test: a host still below the threshold after cleanup refuses, with both measurements in the error.
- Unit test: a `docker rmi` refusal caused by dependent child images is not classified as a cleanup failure.

## Out of scope

- Changing the 5G threshold itself. The value was not the problem.
- Automatic scheduled cleanup independent of `update`. If fleet disk drift needs its own surface, that is a separate item alongside R41 (`backup health`) rather than part of `update`.

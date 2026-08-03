# dsc update

Runs remote OS and Discourse update workflows over SSH.

```
dsc update <name|all> [-p [N]] [--skip-recent [DUR]] [--force] [--no-changelog] [--yes]
dsc update log [--latest] [--since <DUR>] [--format text|md|json]
```

## Flags

- `--parallel [N]` (or `-p`) — run updates concurrently (only with `all`). `-p` uses 3 workers; `-p N` uses N. Put the forum name before `-p` (e.g. `update all -p 4`).
- `--skip-recent [DUR]` — skip forums fully updated within a window (default `24h`; e.g. `--skip-recent 6h`). For unattended re-runs; see [Skipping behaviour](#skipping-behaviour).
- `--force` — update even if a `./launcher rebuild` is already running on the host, or if the forum was updated recently (overrides both guards).
- `--no-changelog` — skip changelog posting.
- `--yes` (or `-y`) — auto-confirm the changelog post prompt (non-interactive mode).

## Update workflow

1. Refuse or skip if a `./launcher rebuild` is already running, unless `--force` is set.
2. Fetch the initial Discourse version and OS details.
3. Check available 1 KiB blocks on `/` and compare them without whole-GiB rounding. If space is below the configured minimum, run the [disk recovery](#disk-guard-and-recovery) sequence before making any OS or Discourse change.
4. Run the OS package update over SSH and reboot if applicable.
5. Check whether the running Discourse commit matches the latest `stable` branch commit on GitHub. If they match, skip the rebuild.
6. Run the Discourse rebuild (`./launcher rebuild app`) only if step 5 found a newer commit available.
7. Fetch final version information, run Docker cleanup (`docker container prune -f && docker image prune -f`), and report root disk usage.
8. Optionally post a changelog checklist to the configured topic.

If the OS update command fails, `dsc update` aborts after attempting the rollback command (when configured).

OS-update and Discourse-rebuild success is determined only by the SSH process exit status. Output written to stderr, including ordinary git fetch progress, does not turn a successful command into a failure. A failed streamed command reports its exit status and bounded tails from both stdout and stderr; the update log stores only the concise first-line diagnosis.

## Disk guard and recovery

The preflight requires at least 5 GiB free on `/` by default. When the host is below that threshold, `dsc` performs a conservative recovery before refusing:

1. Run a fixed `docker image prune -f`, which removes dangling images but not tagged images or stopped containers, and re-measure free space. The configurable post-update cleanup hook is deliberately not reused during preflight.
2. Continue the update only if the minimum is now met.
3. If space is still low, list `discourse/base` images and preserve the newest ID when constructing manual suggestions. `dsc` does not delete tagged base images automatically because creation order does not prove an image is unused.

The automatic preflight never runs `docker system prune -a`, which could discard the current base image and force a large re-download. A locally configured `DSC_SSH_CLEANUP_CMD` still controls post-update cleanup and remains the operator's responsibility.

If recovery cannot free enough space, the error reports the initial and final measurements and prints shell-quoted commands tailored to rootful or rootless Docker. Suggested `docker rmi` commands contain validated older image IDs, but the operator must confirm an image is unused before running them. The output also suggests `docker system df`, `docker image ls --no-trunc discourse/base`, and `df -h /`, with an explicit warning not to add `--force`. Journal inspection is mentioned separately because deleting logs depends on the host's retention policy.

## Changelog template

The changelog is posted as a checklist to the topic specified by `changelog_topic_id`:

```md
- [x] OS updated {{ubuntu_os_version}}
- {% if rebooted %} {{[x] Server rebooted}} {% endif %}
- {% if recovered %} [x] Preflight disk recovery: {{ before_free }} -> {{ after_free }} available {% endif %}
- [x] Updated Discourse:
  - Initial version: {{ before_version }} [{{ before_commit_hash | truncate 7 }}](https://github.com/discourse/discourse/commit/{{ before_commit_hash }})
  - Updated version: {{ after_version }} [{{ after_commit_hash | truncate 7 }}](https://github.com/discourse/discourse/commit/{{ after_commit_hash }})
- [x] Docker cleanup total reclaimed space: {{ reclaimed_space }}
- [x] Root disk usage (df -h /): {{ root_disk_usage }}
```

## Parallel updates

```bash
# Update all installs, 4 at a time, non-interactively
dsc update all -p 4 --yes
```

In sequential mode (without `-p`), updates run one-by-one. `all` is a reserved name for `dsc update all`.

## Rootless Docker

If `docker_rootless = true` is set on a Discourse entry, the update command drops `sudo -n` from Docker and launcher commands. This is required for instances provisioned with `dsc harden` (which defaults to rootless Docker). Without this flag, commands like `sudo ./launcher rebuild app` fail because the root user has no Docker context.

## Skipping behaviour

Three independent gates can skip work; each is recorded in the [update log](#update-log):

- **No `ssh_host`:** `dsc update all` skips any Discourse instance with no `ssh_host` configured (read-only references like Discourse Meta, or instances not managed via SSH).
- **Rebuild already running:** at the very top of a forum's update (before the reboot), `dsc` checks for an in-flight `./launcher rebuild` on the host and skips the whole forum if one is found — so re-running never reboots a box mid-rebuild. Override with `--force`.
- **Updated recently:** with `--skip-recent [DUR]`, a forum whose last successful update is within the window (default `24h`) is skipped entirely. Interactively (a TTY, no flag), `dsc` instead lists the recently-updated forums and asks **up front** whether to update them again — so a re-run to catch one straggler doesn't reboot the rest. Override with `--force`.
- **Already up to date:** before the rebuild, `dsc` compares the running commit with the latest `discourse/discourse` `stable` commit on GitHub. If they match, only the *rebuild* is skipped (OS updates and reboot still run). Unreachable GitHub or unknown commit → rebuild proceeds (fail-open).

## Update log

Every `dsc update` pass appends one line per forum to an append-only log — a register of what was updated, when, and to which version.

```
dsc update log            # full chronological history
dsc update log --latest   # one row per forum (most recent state) - a fleet checklist
dsc update log --since 7d --format md
```

- Outcomes recorded: `updated`, `current` (already on latest), `skipped-recent`, `skipped-rebuild`, `failed`.
- Format: tab-separated, timestamp-first (greppable/`tail`-able); the pretty view is rendered by `update log`.
- Location: `$XDG_STATE_HOME/dsc/update.log` (default `~/.local/state/dsc/update.log`), overridable with `DSC_UPDATE_LOG`.

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `DSC_SSH_OS_UPDATE_CMD` | `sudo -n DEBIAN_FRONTEND=noninteractive apt update && sudo -n DEBIAN_FRONTEND=noninteractive apt upgrade -y` | OS update command. |
| `DSC_SSH_OS_UPDATE_ROLLBACK_CMD` | *(none)* | Rollback command if OS update fails. |
| `DSC_SSH_REBOOT_CMD` | `sudo -n reboot` | Reboot command. |
| `DSC_SSH_OS_VERSION_CMD` | `lsb_release -d \| cut -f2` | OS version detection (fallback: `/etc/os-release`). |
| `DSC_SSH_UPDATE_CMD` | `cd /var/discourse && sudo -n ./launcher rebuild app` | Discourse rebuild command. |
| `DSC_SSH_CLEANUP_CMD` | `sudo -n docker container prune -f && sudo -n docker image prune -f` | Post-rebuild cleanup command. Rootless installs omit `sudo -n`; preflight always uses the fixed, narrower `docker image prune -f`. |
| `DSC_SSH_STRICT_HOST_KEY_CHECKING` | `accept-new` | SSH host key checking mode (set empty to omit). |
| `DSC_SSH_OPTIONS` | *(none)* | Extra SSH options (space-delimited). |
| `DSC_DISCOURSE_MIN_FREE_GB` | `5` | Minimum free GiB required on `/` after automatic recovery. |
| `DSC_DISCOURSE_BOOT_WAIT_SECS` | `15` | Seconds to wait after rebuild before fetching `about.json`. |
| `DSC_UPDATE_LOG` | `$XDG_STATE_HOME/dsc/update.log` | Path to the append-only update log. |
| `DSC_COLOR` | `auto` | ANSI color output (`auto`/`always`/`never`). `NO_COLOR` also disables color. |

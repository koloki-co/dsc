# `dsc update` - Docker daemon selection preflight

Spec for a read-only preflight that checks rootful/rootless Docker selection before `dsc update` touches a host. Goal: never rebuild a second `app` under the wrong daemon against the same shared data. Driver: roadmap R81, the November 2026 mixed-daemon field incident.

## Motivation

`dsc update` chooses rootful (`sudo -n docker`, `sudo -n ./launcher`) or rootless Docker from `docker_rootless` in `dsc.toml` alone, and an unset value means rootful. On 29 October 2026 a forum whose config omitted `docker_rootless`, although it had been installed rootless, was rebuilt with `sudo /var/discourse/launcher rebuild app`. By 6 November the rootful `app` was serving while the older rootless `app` restarted repeatedly against the same `shared/` data, leaving the live backup temporary directory unwritable. Field recovery stopped the unused restart loop, repaired only verified paths, and took a fresh backup. Nothing in `dsc` noticed the disagreement between configuration and host.

## Current state (as of 2026-11-27)

Before this change, nothing in `dsc update` inspected which daemon held `app`. The rootless checkout preflight (R42) ran only when `docker_rootless = true`.

## Behaviour (Phase 1, implemented on branch)

After the initial version/OS fetch and before the rootless checkout preflight, disk recovery, OS update, reboot, or rebuild, `dsc update` runs one read-only SSH probe (`DAEMON_PROBE_CMD` in `src/commands/update_daemon.rs`) and prints `Checking Docker daemon selection`.

The probe gathers, as `KEY=VALUE` lines:

| Key | Source | Meaning |
| --- | --- | --- |
| `ROOTFUL_ENABLED` | `systemctl is-enabled docker.service docker.socket` | `enabled`/`enabled-runtime` on either unit counts as installed |
| `ROOTFUL_ACTIVE` | `systemctl is-active` on the same units | `active` on either unit counts as active |
| `ROOTLESS_UNIT` | `~/.config/systemd/user/docker.service` or `systemctl --user cat` | the SSH user has a rootless daemon unit |
| `ROOTLESS_ACTIVE` | `systemctl --user is-active docker.service` | the SSH user's rootless daemon is running |
| `ROOTFUL_APP` | `sudo -n docker -H unix:///var/run/docker.sock ps -a --filter name=^app$` | `app` state, `none`, or `?` when the query failed |
| `ROOTLESS_APP` | `docker -H unix://$XDG_RUNTIME_DIR/docker.sock ps -a --filter name=^app$` | same, for the rootless socket |
| `ROOTLESSKIT_USERS` | `ps -eo user=,comm=` | other users running `rootlesskit` |

Each `docker` call names its socket explicitly, so an inherited `DOCKER_HOST` cannot make both probes hit the same daemon. A disabled, inactive system `docker.service` is what Docker's own rootless setup leaves behind from the `docker-ce` package, so it does not count as a rootful installation in use. Without that rule, almost every rootless host would trigger the mixed-installation warning.

### Classification

The selected daemon is the one `docker_rootless` names. The other daemon is the unselected one.

| Condition | Result |
| --- | --- |
| `app` is `running` or `restarting` under the other daemon | **Refuse**: rebuilding would start a second app against the same shared data |
| Selected daemon has no `app`; the other daemon has one in any state | **Refuse**: `docker_rootless` is probably wrong for this host |
| Both daemons installed (active or enabled-but-inactive) | Warn, continue |
| Stopped (`exited`, `created`, `dead`, ...) `app` under the other daemon | Warn, continue |
| Another user runs `rootlesskit` | Warn, continue (that daemon's containers are not visible) |
| Selected daemon could not be queried | Warn, continue (the later steps fail on their own if it really is unusable) |
| Installed other daemon could not be queried | Warn: a duplicate `app` cannot be ruled out |
| Whole probe fails over SSH | Warn, continue: state not verified |

A refusal is an ordinary pass failure: it exits non-zero, is recorded as `failed` in the update log, and happens before any disruptive step. Inability to inspect a daemon is always reported as unknown, never as absence. A daemon with no sign of installation is treated as having no `app`.

`dsc` never starts, stops, enables, disables, or migrates a daemon or container, never changes `sysctl` or ownership, and never switches `docker_rootless` itself. `--force` does not bypass a refusal, because `--force` overrides the in-progress-rebuild and recent-update guards, and a daemon disagreement is a different hazard.

## Tests

`src/commands/update_daemon.rs` unit tests cover:

- rootful-only and rootless-only hosts with no warnings, including the disabled `docker-ce` unit on a rootless host
- a mixed installation with an inactive daemon and no duplicate container, which warns
- the 6 November shape (rootless `app` restarting, rootful selected), which refuses
- the 29 October trigger (rootless `app` only, config unset), which refuses
- a stopped duplicate, which warns
- live apps on both daemons, which refuses whichever daemon is selected
- an inaccessible selected daemon and a garbled probe, both unknown and never absent
- another user's `rootlesskit`
- a read-only guard on the probe text, with explicit sockets
- the probe executed under a real `sh`, so it is valid POSIX shell and always emits every key

## Phases

### Phase 1 - blocking (implemented on branch, pending live verification)

- [x] Read-only probe and classification before any disruptive step
- [x] Mixed-installation warning even without duplicate containers
- [x] Refusal on a live duplicate or misdirected `docker_rootless`
- [ ] Live verification on rootful-only, rootless-only and mixed hosts

### Phase 2 - investigation

- [ ] Establish which `dsc` version and install/config path produced the omitted `docker_rootless` (historical `dsc add`/import defaults)
- [ ] Consider surfacing the same probe in `dsc config check`, so the disagreement is visible without starting an update

### Phase 3 - nice to have

- [ ] Inspect other users' rootless daemons when the SSH user has permission, rather than only naming them

## Backward compatibility

Hosts where configuration and Docker state agree see one extra stage line and no behaviour change. A host where `app` is live under the unselected daemon, or exists only there, now refuses instead of being rebuilt. That is the intended change. The fix is to correct `docker_rootless` or stop the unused container. The probe adds one SSH round trip per forum.

## Out of scope

- Automatic daemon migration, switching, or `docker_rootless` rewriting
- Recursive ownership repair of shared data
- Rootless privileged-port capability checks (R86)
- Any change to `./launcher` or Discourse itself

# `dsc install` and `dsc harden` stage 3 - provisioning spec

Spec for `dsc install` (Phase 1 implemented on `main`, 2026-09-29) and the not-yet-shipped finishing items in `dsc harden` stage 3. Together these complete the from-zero Discourse bootstrap story: a fresh Ubuntu IP becomes a hardened box becomes a running Discourse with an entry in your `dsc.toml`.

## Motivation

`dsc update` assumes an already-running Discourse. `dsc harden` (stages 1+2 shipped) prepares a fresh server for one. The gap is the middle: declaratively installing Discourse on a hardened box. Before Phase 1, this was done by hand (clone `discourse_docker`, edit `app.yml`, `launcher bootstrap`, `launcher start`, then add a `[[discourse]]` block to `dsc.toml` manually). `dsc install` closes that gap for the standard single-container case.

## Current state (as of 2026-09-29)

- `dsc harden` ships stages 1 + 2 (preflight, non-root sudo user, key-only sshd on a non-default port, self-lockout guard). Stage 3 (timezone, swap, journald, unattended upgrades, fail2ban, rootless Docker, ufw) has its config keys wired in [src/commands/harden.rs](../../src/commands/harden.rs) `HardenOpts` but the SSH-side execution and tests are pending.
- `dsc install` Phase 1 is implemented ([src/commands/install.rs](../../src/commands/install.rs)): memory/disk preflight, idempotent `discourse_docker` clone/pull, templated `app.yml` render + upload, streamed `launcher bootstrap && start`, `about.json` poll, `[[discourse]]` append. Deliberately **did not wait for stage 3** — the phasing below originally called that a blocking prerequisite, but real usage (bringing up Numun's Discourse) needed `install` before stage 3 landed, so `install`'s docs instead document installing Docker manually as a prerequisite in the meantime. See [docs/install.md](../../docs/install.md) for the user-facing description and the deviations from this spec's original CLI surface (`--image` and `--bootstrap-admin` deferred; see "Deviations from the original design" below).
- `DiscourseConfig` gained `ssh_user`/`ssh_port` fields, written by `dsc install` on success. `dsc update` and the rest of the SSH-target-resolving commands still resolve `ssh_host` through `~/.ssh/config` as before; wiring them to consume `ssh_user`/`ssh_port` directly is a follow-up, not required for `install` itself to work (an operator can add an `~/.ssh/config` `Host` alias for the new box if they want `dsc update` etc. to use it under its bare name).

## Deviations from the original design (Phase 1)

- **No `--image` flag.** Base-image customisation is rare and adds template complexity without a concrete driver yet; deferred to Phase 2.
- **No `--bootstrap-admin`.** Manual first-admin signup only, as the spec's own "Default (manual)" mode describes; Phase 3 is unchanged.
- **Poll `http://`, not `https://`, for `/about.json`.** A freshly-bootstrapped `standalone.yml` has no SSL template enabled (Let's Encrypt is a separate, later step involving DNS), so HTTPS would never succeed at this stage regardless of timeout. Poll window widened from the originally-specced 60s to 300s, since `launcher start` can return before Puma has finished coming up.
- **Transport is base64-over-SSH, not heredoc/scp.** Matches `dsc harden`'s existing sshd-drop-in transport: the rendered `app.yml` is base64-encoded and decoded remotely, so no character in a hostname, email, or SMTP password can be misparsed as shell syntax. YAML-level safety (so those same values can't break the *file's* structure) is handled separately by single-quote-escaping every substituted value.
- **No `--host` SSH connection caching / state file.** Each `dsc install` invocation takes `--ssh-user`/`--ssh-port` directly (defaulting to `dsc harden`'s own defaults) rather than reading a transient state file from a prior `harden` run.

## `dsc harden` stage 3 - finishing items

In rough execution order. All gated behind the existing config keys; the work is the SSH-side execution path.

1. **Timezone + time sync.** `timedatectl set-timezone <timezone>` (default `UTC`), then verify `timedatectl status` shows synchronised. Install `chrony` if `systemd-timesyncd` is unavailable.
2. **Swap file.** Check `swapon --show`; if no swap, create `/swapfile` of `swap_size_gb` (default 2 GB), `mkswap` + `swapon`, persist in `/etc/fstab`, set `vm.swappiness=10` via `/etc/sysctl.d/99-dsc.conf`.
3. **Journald cap.** Write `SystemMaxUse=<journald_max_use>` (default `500M`) to `/etc/systemd/journald.conf.d/size-cap.conf`, `systemctl restart systemd-journald`.
4. **Unattended security upgrades.** Ensure `unattended-upgrades` is installed; write `/etc/apt/apt.conf.d/20auto-upgrades` with both `Update-Package-Lists` and `Unattended-Upgrade` set to `1`.
5. **fail2ban.** `apt install fail2ban`; minimal jail for sshd on the new port.
6. **Rootless Docker** (when `docker_rootless = true`, which is the default): `curl -fsSL https://get.docker.com | sh`, then `apt install uidmap`, then as the new user `dockerd-rootless-setuptool.sh install`, then `sudo setcap cap_net_bind_service=ep $(which rootlesskit)` so Discourse can bind 80/443, then `systemctl --user restart docker`, then `loginctl enable-linger <new_user>` so the user-level systemd units survive logout.
7. **`ufw`.** Allow 22, `<ssh_port>`, 25, 80, 443; allow 60000:61000/udp when `--mosh` flag is present (CLI flag still TODO). Apply each `extra_ufw_allow` entry. `ufw --force enable`.

### Gotchas to remember

- **sshd port change + cloud firewall.** Hetzner / Digital Ocean / etc. have their own firewall layer that `dsc` can't reach. The new SSH port needs opening *there too*. Document in stdout near the end of harden output: "now open port `<ssh_port>` in your cloud provider's firewall".
- **Rootless Docker + privileged ports.** The `setcap cap_net_bind_service=ep` step on `rootlesskit` is non-optional for Discourse to bind 80/443. Easy to forget; bake into the harden output, not just the docs.
- **`loginctl enable-linger`.** Without it, user-level systemd units (rootless Docker daemon, the Discourse container) die on SSH disconnect. Same status as the setcap line - non-optional.
- **MOSH ports.** Only opened when the operator asks for them. Add a `--mosh` flag at the same time as the rest of stage 3 wiring.
- **Ubuntu version drift.** Stage 3 should test the OS detection on whatever ships at the time. Today the harden code accepts `ID=ubuntu`; `discourse_docker` may not have a base image for the very newest LTS yet (2-3 month lag historically). The pragmatic answer is "try the previous LTS first if you hit this".

## `dsc install` - new command

Templated `app.yml` + `launcher bootstrap + start` + `dsc.toml` write, all over SSH.

### CLI surface (as implemented, Phase 1)

```text
dsc install <name> --host <host>
                   [--ssh-user discourse] [--ssh-port 2227]
                   --email admin@example.com[,other@example.com]
                   [--smtp-host …] [--smtp-port …] [--smtp-user …] [--smtp-pass-stdin]
                   [--branch <git-revision>]
                   [--dry-run]
```

`--image` and `--bootstrap-admin` from the original design (below) remain unimplemented; see "Deviations" above.

### What it does

1. **Connect** as `<ssh-user>@<host>:<ssh-port>`, both taken directly from CLI flags (defaulting to `dsc harden`'s own defaults, `discourse`/`2227`) — no transient state file.
2. **Memory + disk preflight.** `free`-equivalent (`/proc/meminfo`); bail below 1024 MB, warn below 2048 MB. Free space on `/var` (`df`); bail below 5 GB, warn below 30 GB. Same thresholds `dsc harden` itself uses.
3. **Clone `discourse_docker`** to `/var/discourse` if not present (`test -d /var/discourse/.git`). Otherwise `git pull`.
4. **Render `app.yml`** locally from a template based on `discourse_docker`'s own `samples/standalone.yml` — substitute `DISCOURSE_HOSTNAME`, `DISCOURSE_DEVELOPER_EMAILS`, an optional SMTP block, and an optional `params.version`. No interactive `discourse-setup`. Base64-transported into `containers/app.yml` on the remote (see "Deviations" above for why, not heredoc/scp).
5. **`sudo -n ./launcher bootstrap app && sudo -n ./launcher start app`**, streamed live with a progress spinner and a bounded 20-line stdout/stderr tail retained for the error message on failure.
6. **Poll `http://<host>/about.json`** until it succeeds (300s timeout, 5s interval — see "Deviations" for why HTTP not HTTPS). Fail with a clear error pointing at `docker logs app` if it never comes up.
7. **Append a `[[discourse]]` entry** to `dsc.toml` via the existing `save_config` path: `name`, `baseurl: https://<host>`, `ssh_host: <host>`, `ssh_user: <ssh-user>`, `ssh_port: <ssh-port>` (omitted when 22). Leave `apikey` and `api_username` empty. Refuses up front if a discourse of that name already exists.
8. **Print a status line** with where the new `[[discourse]]` entry landed and a reminder to create and set an API key (no separate "next steps footer" beyond that one line; `--bootstrap-admin` is not implemented, see below).

### First admin flow

Two modes:

- **Default (manual).** User opens `https://<host>/admin` in a browser, signs up the first admin, generates an admin API key in the Admin UI, pastes it into `dsc.toml`. Predictable, no `rails runner` required.
- **`--bootstrap-admin` (later, not implemented).** `docker exec app rails runner …` to create the admin and mint the API key in one shot, populating `dsc.toml` with both fields. Roadmapped as a follow-up - safer to have shipped the manual mode first, which is what Phase 1 does.

### Honours `--dry-run`

Yes, fully — implemented. Prints every SSH command it would run (including the rendered `app.yml`'s upload), the launcher invocation, the `about.json` poll it would perform, and the `[[discourse]]` block that would be appended to `dsc.toml`. No SSH connection or HTTP request happens at all under `--dry-run`; the local `dsc.toml` write is also skipped.

## Config schema additions

Implemented in `DiscourseConfig` in [src/config.rs](../../src/config.rs):

```rust
#[serde(default, deserialize_with = "deserialize_opt_string_empty_as_none")]
pub ssh_user: Option<String>,
#[serde(default, deserialize_with = "deserialize_opt_u64_zero_as_none")]
pub ssh_port: Option<u64>,
```

`dsc update` and other SSH-target-resolving commands still read only `ssh_host` (via `~/.ssh/config`); consuming `ssh_user`/`ssh_port` directly there is unimplemented follow-up work, not required for `install` to write correct entries.

## Tests

- Stage 3 individual steps: still to be tested in isolation once implemented (mock SSH session, assert generated config files match golden snapshots).
- `dsc install`'s `app.yml` templating is unit-tested without any SSH: valid-YAML round-trip via `serde_yaml`, hostname/email/SMTP substitution, `params.version` presence/absence, and a single-quote-injection-attempt test proving the YAML structure survives a hostile value. Memory/disk preflight bail/warn thresholds are unit-tested directly. CLI parsing (defaults, comma-separated `--email`, `--smtp-pass-stdin requires --smtp-host`) is tested in `src/cli.rs`. `--dry-run` writing nothing (neither the remote host nor local `dsc.toml`) is tested via `install()` directly with an in-memory `Config`.
- The SSH/launcher/HTTP-poll side is not yet integration-tested against a real host (no `fake-ssh`-fixture coverage for `install` specifically); real-host verification happened manually against the Numun box. A fixture-based integration test analogous to `commands::file`'s `fake-ssh` tests would be a reasonable follow-up.

## Out of scope

- Provisioning the cloud VM itself (`hcloud server create`, `doctl compute droplet create`, etc.). Out of `dsc`'s lane.
- `discourse-setup` interactive wizard parity. The whole point is to not need it.
- Multi-container Discourse setups (separate `data.yml` + `web_only.yml`). Single-container `app.yml` only for v1.
- Discourse downgrades. `--branch tests-passed` and `--image base` are the safe path; if a user wants to pin a specific commit they can edit `app.yml` after install.

## Phasing

The original ordering below turned out to be the wrong order for real usage: `dsc harden` stage 3 wasn't needed to unblock `dsc install`, since Docker can be (and was) installed manually as a documented prerequisite. Renumbered to reflect what actually happened.

### Phase 1 - `dsc install` minimum viable path (implemented, 2026-09-29)

`--host`, `--ssh-user`/`--ssh-port`, `--email`, SMTP flags, `--branch`, manual-admin mode, `ssh_user`/`ssh_port` config fields. Single-container `app.yml` only. Driver: bringing up Numun's Discourse.

### Phase 2 - `--image` and finish `dsc harden` stage 3

Base-image override for `install`; the stage-3 items above (timezone/swap/journald/unattended-upgrades/fail2ban/rootless-Docker/ufw) so `dsc harden` stops requiring a manual Docker install before `dsc install` can run. Not currently blocking anything — demand-driven.

### Phase 3 - `--bootstrap-admin`

`rails runner` admin creation + API-key minting. Risky (one-shot, hard to rerun) so save for after Phase 1 has soaked further.

### Phase 4 - polish

Memory preflight refinements, parallel installs (`dsc install all` from a multi-name flag set), more SMTP providers' presets, `app.yml` template variations, fixture-based (`fake-ssh`) integration tests for the launcher/poll steps.

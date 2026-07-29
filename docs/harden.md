# dsc harden

Turn a fresh Ubuntu server reachable via `ssh root@host` into a hardened box ready for `dsc install`. **WIP** — currently ships **stages 1 and 2**; stage 3 is in development.

## Usage

```text
dsc harden <host> --pubkey-file <path> [--new-user <name>] [--ssh-port <port>] [--ssh-user root]
```

Starting from a fresh `ssh root@host` that you can already log in to (typical for a cloud-provisioned VM with your initial SSH key on creation), `dsc harden` runs stages 1 and 2 end-to-end. After it succeeds, the SSH entry-point on the box has moved to `discourse@host:2227` (or whatever you configured); root login is disabled, password auth is disabled, and only the named user can SSH at all.

## What stage 1 does

1. **Preflight** — confirms the remote is Ubuntu, has ≥ 1 GB RAM (warns < 2 GB), has ≥ 5 GB free on `/var` (warns < 30 GB), and that the SSH user is currently root.
2. **Creates the new user** with `adduser --disabled-password`. Skipped if the user already exists.
3. **Grants sudo NOPASSWD** by dropping a single-file snippet into `/etc/sudoers.d/`, validated through `visudo -cf` before being moved into place. Safer than editing the main sudoers, and idempotent.
4. **Installs the supplied pubkey** into the new user's `authorized_keys` with correct perms. Deduplicated, so re-running doesn't append duplicate lines.
5. **Verifies new-user SSH actually works** by opening a second SSH session as the new user and running `whoami`. **If this fails, the run errors out before stage 2 touches sshd** — so a bad pubkey can never lock you out of the box. The original `ssh root@host` is still usable for debugging.

## What stage 2 does

6. **Writes `/etc/ssh/sshd_config.d/90-dsc-harden.conf`** with: `Port <ssh-port>`, `PermitRootLogin no`, `PasswordAuthentication no`, `PubkeyAuthentication yes`, `MaxAuthTries 3`, `LoginGraceTime 30`, `AllowUsers <new-user>`, `X11Forwarding no`, `AllowAgentForwarding no`, `ClientAliveInterval 300`, `ClientAliveCountMax 2`, plus modern cipher/KEX/MAC pins (chacha20-poly1305, curve25519-sha256, hmac-sha2-*-etm) — drops CBC ciphers and SHA-1 MACs.
7. **Validates the file** with `sshd -t` *before* installing it. A typo can't break the daemon's next restart.
8. **Patches `ssh.socket`** on Ubuntu's socket-activated systems via `/etc/systemd/system/ssh.socket.d/90-dsc-harden.conf` so the systemd listener actually binds the new port. Without this step the sshd config knows about the new port but systemd is still listening on 22 — a subtle Ubuntu 22.04+ pitfall.
9. **Verifies the new port** by opening a third SSH session as `<new-user>@<host>:<ssh-port>`. Same self-protection logic as stage 1 — if this fails, you get a clear error pointing at how to roll back from the still-open root session.

After a successful run: root@22 stops responding (port 22 isn't listened to anymore), and the only way in is `<new-user>@<host>:<ssh-port>` with the configured pubkey.

### Idempotency caveat

Stages 1 and 2 are individually idempotent within the fresh-box flow — if you re-run `dsc harden` and stage 1 has already been done, step-by-step skip messages confirm this. **However**, after stage 2 has run successfully the box is no longer reachable as `root@host:22`, so a naive `dsc harden <host>` will fail at step 1 with `Connection refused`. To re-run after a successful harden you either need to update your `~/.ssh/config` for the host so the alias resolves to `<new-user>@<host>:<ssh-port>`, or pass `--ssh-user discourse` explicitly (CLI doesn't yet expose `--initial-port`; tracked as a follow-up).

## What stage 3 will add

Tracked in [spec/commands/install.md](https://github.com/koloki-co/dsc/blob/main/spec/commands/install.md). Briefly: UTC timezone, time-sync verified, swap file (2 GB by default), journald log cap, unattended security upgrades, fail2ban, rootless Docker (per the Bawmedical playbook — `setcap cap_net_bind_service=ep` on rootlesskit, `loginctl enable-linger`), and `ufw` opened for Discourse's standard ports.

## Configuration (`[harden]` block)

Every flag has a sensible built-in default. You can override defaults globally in `dsc.toml`'s `[harden]` block, and override that on a per-run basis with the CLI flag. Resolution: **CLI flag → `[harden]` block → built-in default**.

```toml
[harden]
new_user                     = "discourse"
ssh_port                     = 2227
docker_install_url           = "https://get.docker.com"
docker_rootless              = true
swap_size_gb                 = 2          # 0 to skip
journald_max_use             = "500M"
timezone                     = "UTC"
unattended_security_upgrades = true
fail2ban                     = true
mosh                         = false      # opt-in; opens UDP 60000-61000
# sshd_ciphers, sshd_kex, sshd_macs override dsc's policy overlays
# (not brittle full allow-lists)
# extra_ufw_allow = ["3000/tcp", "192.168.1.0/24"]
```

Read [`dsc.example.toml`](https://github.com/koloki-co/dsc/blob/main/dsc.example.toml) for the full annotated block.

## Why publish a hardening routine?

A reasonable concern: doesn't publishing the exact steps `dsc harden` takes give attackers a blueprint? Short answer — yes a tiny bit, but no in any meaningful sense, and the auditability gain dwarfs the fingerprinting cost.

**What an attacker actually learns:**

- The defaults you'll likely run (port 2227, user `discourse`, fail2ban tuning, ufw rule list). Mostly fingerprinting value, not exploit value. Port scanners find non-default SSH in under a minute anyway, and `PasswordAuthentication no` makes the username irrelevant for brute force.
- The list of packages and configs. Already implied by "this is a Discourse server"; nothing new.
- Bug surface in the hardening tool itself. This is the real risk — see SECURITY.md.

**What you gain by publishing:**

- **Auditability.** You can read [`src/commands/harden.rs`](https://github.com/koloki-co/dsc/blob/main/src/commands/harden.rs) before running it as root on a fresh box. The alternative — closed binary with full root — is much worse for trust.
- **Crowd review.** Security researchers can spot mistakes before attackers do. Every major hardening tool is open (ansible-hardening, OpenSCAP, dev-sec.io, CIS, lynis) for exactly this reason.
- **Reproducibility.** You can rebuild the same posture by hand from the published steps if you don't want the tool.

**Mitigations dsc applies:**

- Every "magic" default is configurable — port, username, swap size, packages, the lot. Power users who want extra obscurity can deviate; defaults stay readable for everyone else.
- No secrets, backdoor users, or default keys are ever shipped.
- Every non-obvious step has an inline `// why:` comment so reviewers can reason quickly.
- Vulnerabilities should be reported privately — see [SECURITY.md](https://github.com/koloki-co/dsc/blob/main/SECURITY.md).

**The one genuinely new risk:** once enough people use `dsc harden`, attackers can fingerprint dsc-hardened boxes (specific port + user + ufw rule combination + sshd algorithm list). For most users this is acceptable — being identifiably-hardened is still safer than being unhardened — but worth knowing so power users can opt out of any default they care about.

## Testing the new user before going further

Stage 1's self-lockout guard catches the common failure mode (bad pubkey, wrong username, sshd quirks), but it's still good practice to test the new-user SSH yourself before invoking stage 2. From your laptop:

```bash
ssh -i ~/.ssh/<server-key> <new-user>@<host> 'sudo whoami'   # should print "root"
```

If that succeeds, you're safe to run subsequent stages.

## Examples

```bash
# Bare-minimum first run on a fresh Hetzner box; uses every default.
dsc harden 192.0.2.1 --pubkey-file ~/.ssh/myserver.pub

# Dry-run first to inspect the exact commands.
dsc --dry-run harden 192.0.2.1 --pubkey-file ~/.ssh/myserver.pub

# Override a default on a per-run basis.
dsc harden 192.0.2.1 --pubkey-file ~/.ssh/myserver.pub --new-user ops --ssh-port 40022
```

## Related

- [`dsc install`](https://github.com/koloki-co/dsc) — declarative Discourse install on a hardened box. WIP, lands after `dsc harden` is feature-complete.
- [`dsc update`](update.md) — runs OS + Discourse rebuilds via SSH. Already shipped; complements `dsc harden` once the box is provisioned.
- [`dsc config check`](config.md) — verifies API and SSH connectivity for every install in `dsc.toml`. Good first thing to run after `dsc install` adds your new box.

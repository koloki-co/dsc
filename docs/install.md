# dsc install

Provisions Discourse on a `dsc harden`-prepared box: clones `discourse_docker`, renders `containers/app.yml`, runs `launcher bootstrap && launcher start`, polls for a live site, and appends a `[[discourse]]` entry to `dsc.toml`.

**Phase 1** — the default single-container `standalone.yml` stack at the default path, **targeting rootless Docker only** (see [Prerequisites](#prerequisites)). Not yet implemented: a rootful-Docker target, `--image` (base image override), and `--bootstrap-admin` (create the first admin account non-interactively). See [spec/commands/install.md](https://github.com/koloki-co/dsc/blob/main/spec/commands/install.md) for the full design and remaining phases.

## Usage

```text
dsc install <name> --host <host>
                    [--ssh-user discourse] [--ssh-port 2227]
                    --email admin@example.com[,other@example.com]
                    [--smtp-host <host>] [--smtp-port <port>] [--smtp-user <user>] [--smtp-pass-stdin]
                    [--branch <git-revision>]
```

`--ssh-user`/`--ssh-port` default to match `dsc harden`'s own defaults (`discourse`/`2227`), so a straight `dsc harden <host> --pubkey-file … && dsc install <name> --host <host> --email …` works with no extra flags in the common case.

## What it does

1. **Preflight** — checks RAM (hard floor 1 GB, warns below 2 GB) and free disk on `/var` (hard floor 5 GB, warns below 30 GB), the same thresholds `dsc harden` uses.
2. **Clones `discourse_docker`** to `/var/discourse` if it isn't already there; otherwise `git pull`s it. Idempotent — safe to re-run after a partial failure.
3. **Renders `containers/app.yml`** from a template based directly on `discourse_docker`'s own `samples/standalone.yml`: Postgres + Redis + rate-limited web, ports 80 and 443 exposed, `docker_manager` pre-installed. **SSL (Let's Encrypt) is requested unconditionally** — `templates/web.ssl.template.yml` and `templates/web.letsencrypt.ssl.template.yml` are always included, with `LETSENCRYPT_ACCOUNT_EMAIL` set to the first `--email`. There's no real downside to this even when DNS isn't pointed at the box yet: cert issuance just fails quietly and the site keeps serving plain HTTP, and re-running `dsc install`/`launcher rebuild` once DNS is ready picks the cert up with no further editing needed. `DISCOURSE_HOSTNAME` and `DISCOURSE_DEVELOPER_EMAILS` are always set; the SMTP block is included only if `--smtp-host` is given (otherwise the rendered file carries a comment noting mail is unconfigured); `params.version` is set only if `--branch` is given (omitting it keeps `discourse_docker`'s own default, `tests-passed`). Every substituted value is YAML single-quote-escaped, and the whole file is transported to the remote host base64-encoded — so a password or hostname containing `$`, backticks, or quotes can't break the shell command or the YAML structure.
4. **Uploads it** to `/var/discourse/containers/app.yml` (`install -m 0644`, no `sudo` — see Prerequisites).
5. **Runs `launcher bootstrap app && launcher start app`** as the plain SSH user with `DOCKER_HOST` exported explicitly for the rootless socket (no `sudo`), streaming a live spinner with the latest output line (this step routinely takes 5-20 minutes); on failure, the last 20 lines of stdout and stderr are included in the error.
6. **Polls `http://<host>/about.json`** every 5 seconds for up to 5 minutes. Deliberately plain HTTP, not HTTPS — a freshly-bootstrapped `standalone.yml` has no SSL template enabled yet (that's a separate later step involving DNS and Let's Encrypt), so port 80 is the only thing that can answer.
7. **Appends a `[[discourse]]` entry** to `dsc.toml`: `name`, `baseurl: https://<host>`, `ssh_host`, `ssh_user`, and `ssh_port` (omitted when 22, matching how existing entries work). `apikey`/`api_username` are left empty — create an API key on the new forum's admin panel and set them afterwards (`dsc api-key create`, then edit `dsc.toml` or `dsc setting`... there is no direct "set my own api key" setting command; edit the file).

## Flags

- `--host` (required) — hostname or IP the box is reachable at over SSH, **and** the value written to `DISCOURSE_HOSTNAME`. Use the real public hostname if you have it; a bare IP works for a first bootstrap but you'll need to re-render before going live with a real domain.
- `--email` (required) — one or more admin/developer emails for `DISCOURSE_DEVELOPER_EMAILS`. Comma-separated for more than one.
- `--smtp-host`, `--smtp-port`, `--smtp-user`, `--smtp-pass-stdin` — SMTP configuration. **Not just cosmetic**: Discourse's normal signup flow sends a confirmation email before any account can log in — including the very first admin account. Without a working SMTP relay, nobody can actually get into the new site at all, even though every other step (including the `about.json` poll) succeeds fine regardless. `dsc install` prints a loud warning (twice — once up front, once in the final summary) if `--smtp-host` is omitted, but doesn't refuse to proceed: there are legitimate reasons to defer (e.g. adding SMTP to `app.yml` by hand afterwards). A personal account (Gmail, Proton Mail, etc.) with an app-specific password is a fine temporary stopgap until a dedicated transactional provider is set up. `--smtp-pass-stdin` reads the password as one line from stdin (never as a bare argument, so it can't leak into shell history or `ps`) and requires `--smtp-host`.
- `--branch` — Discourse git revision to build. Omit to use `discourse_docker`'s own default (`tests-passed`); use `stable` for a slower-moving install.

## Dry run

`dsc install --dry-run …` prints every SSH command, the fully-rendered `app.yml`'s upload plan, the launcher invocation, and the `about.json` poll it *would* perform, and refuses every actual side effect — including the local `dsc.toml` write. No SSH connection or HTTP request happens at all under `--dry-run`.

## Errors and re-running

Every step is written to be safely re-run: cloning is skipped if `/var/discourse` already exists (pulled instead), the `app.yml` write is a plain overwrite (not a diff — this is a from-zero bootstrap, not a declarative sync), and `launcher bootstrap`/`start` are themselves idempotent on the Discourse side. If `dsc install` fails partway through, fix the underlying problem and run the same command again.

## Prerequisites

`dsc install` assumes a `dsc harden`-prepared box: a non-root sudo user with NOPASSWD sudo, and **rootless Docker** already installed and working for that user. Stage 3 of `dsc harden` (which will install Docker as part of hardening) is not yet shipped — see [harden](harden.md) — so for now, install Docker yourself first:

1. Rootless install, per the [official guide](https://github.com/discourse/discourse_docker#user-content-installation-notes): `curl -fsSL https://get.docker.com | sh`, `sudo apt install -y uidmap`, then as the SSH user (not root): `dockerd-rootless-setuptool.sh install`.
2. `sudo setcap cap_net_bind_service=ep $(which rootlesskit)` — without this, the rootless daemon cannot bind ports 80/443 and Discourse will never come up.
3. `loginctl enable-linger <user>` — without this, the rootless daemon (and later, the Discourse container) dies the moment your SSH session ends.

`dsc install` itself only needs `sudo` for one thing: creating `/var/discourse` (since `/var` isn't user-writable) and handing ownership to the SSH user with `chown`. Everything after that — the `git clone`, the `app.yml` write, and `launcher bootstrap`/`start` — runs as that plain user with `DOCKER_HOST` pointed at the rootless socket, never as root. A **rootful** Docker target (where `launcher` needs `sudo` to reach `/var/run/docker.sock`) is not supported by Phase 1 — it would need a different, mutually-exclusive command sequence; tracked as a Phase 2 candidate in the spec.

# dsc install

Provisions Discourse on a `dsc harden`-prepared box: clones `discourse_docker`, renders `containers/app.yml`, runs `launcher bootstrap && launcher start`, polls for a live site, and appends a `[[discourse]]` entry to `dsc.toml`.

**Phase 1** — the default single-container `standalone.yml` stack at the default path. Not yet implemented: `--image` (base image override) and `--bootstrap-admin` (create the first admin account non-interactively). See [spec/commands/install.md](https://github.com/koloki-co/dsc/blob/main/spec/commands/install.md) for the full design and remaining phases.

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
3. **Renders `containers/app.yml`** from a template based directly on `discourse_docker`'s own `samples/standalone.yml`: Postgres + Redis + rate-limited web, ports 80 and 443 exposed, `docker_manager` pre-installed. `DISCOURSE_HOSTNAME` and `DISCOURSE_DEVELOPER_EMAILS` are always set; the SMTP block is included only if `--smtp-host` is given (otherwise the rendered file carries a comment noting mail is unconfigured); `params.version` is set only if `--branch` is given (omitting it keeps `discourse_docker`'s own default, `tests-passed`). Every substituted value is YAML single-quote-escaped, and the whole file is transported to the remote host base64-encoded — so a password or hostname containing `$`, backticks, or quotes can't break the shell command or the YAML structure.
4. **Uploads it** to `/var/discourse/containers/app.yml` (`sudo install -m 0644`).
5. **Runs `launcher bootstrap app && launcher start app`**, streaming a live spinner with the latest output line (this step routinely takes 5-20 minutes); on failure, the last 20 lines of stdout and stderr are included in the error.
6. **Polls `http://<host>/about.json`** every 5 seconds for up to 5 minutes. Deliberately plain HTTP, not HTTPS — a freshly-bootstrapped `standalone.yml` has no SSL template enabled yet (that's a separate later step involving DNS and Let's Encrypt), so port 80 is the only thing that can answer.
7. **Appends a `[[discourse]]` entry** to `dsc.toml`: `name`, `baseurl: https://<host>`, `ssh_host`, `ssh_user`, and `ssh_port` (omitted when 22, matching how existing entries work). `apikey`/`api_username` are left empty — create an API key on the new forum's admin panel and set them afterwards (`dsc api-key create`, then edit `dsc.toml` or `dsc setting`... there is no direct "set my own api key" setting command; edit the file).

## Flags

- `--host` (required) — hostname or IP the box is reachable at over SSH, **and** the value written to `DISCOURSE_HOSTNAME`. Use the real public hostname if you have it; a bare IP works for a first bootstrap but you'll need to re-render before going live with a real domain.
- `--email` (required) — one or more admin/developer emails for `DISCOURSE_DEVELOPER_EMAILS`. Comma-separated for more than one.
- `--smtp-host`, `--smtp-port`, `--smtp-user`, `--smtp-pass-stdin` — SMTP configuration. `--smtp-pass-stdin` reads the password as one line from stdin (never as a bare argument, so it can't leak into shell history or `ps`) and requires `--smtp-host`. Omit the whole group to leave mail unconfigured for now.
- `--branch` — Discourse git revision to build. Omit to use `discourse_docker`'s own default (`tests-passed`); use `stable` for a slower-moving install.

## Dry run

`dsc install --dry-run …` prints every SSH command, the fully-rendered `app.yml`'s upload plan, the launcher invocation, and the `about.json` poll it *would* perform, and refuses every actual side effect — including the local `dsc.toml` write. No SSH connection or HTTP request happens at all under `--dry-run`.

## Errors and re-running

Every step is written to be safely re-run: cloning is skipped if `/var/discourse` already exists (pulled instead), the `app.yml` write is a plain overwrite (not a diff — this is a from-zero bootstrap, not a declarative sync), and `launcher bootstrap`/`start` are themselves idempotent on the Discourse side. If `dsc install` fails partway through, fix the underlying problem and run the same command again.

## Prerequisites

`dsc install` assumes a `dsc harden`-prepared box: a non-root sudo user with NOPASSWD sudo (for `git clone`, `install`, and `launcher`), and Docker already installed. Stage 3 of `dsc harden` (which will install Docker as part of hardening) is not yet shipped — see [harden](harden.md) — so for now, install Docker yourself first (rootless, per the [official guide](https://github.com/discourse/discourse_docker#user-content-installation-notes), remembering the `setcap cap_net_bind_service=ep` step and `loginctl enable-linger` so Discourse can bind ports 80/443 and survives SSH disconnect).

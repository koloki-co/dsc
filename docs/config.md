# dsc config

Inspect and validate your `dsc.toml` configuration.

Running `dsc config` without a subcommand prints the env-var overrides (if any), the active config file, the source it came from, and (when discovery selected it) the full search order with markers:

```text
$ dsc config
$DSC_CONFIG: (unset)
$DSC_CONFIG_HOME: (unset)

Active config: /home/marcus/.config/dsc/dsc.toml (from search hierarchy)

Search order:
  1. dsc.toml
  2. /home/marcus/.config/dsc/dsc.toml <-- active
  3. /etc/xdg/dsc/dsc.toml
  4. /etc/dsc/dsc.toml
  5. /etc/dsc.toml
  6. /usr/local/etc/dsc.toml
```

When `--config` or `$DSC_CONFIG` is in effect, the source line shows which selector won (e.g. `(via --config flag)`, `(via $DSC_CONFIG)`) and the search-order list is suppressed (it would be misleading - it was bypassed). See [configuration.md](configuration.md) for the full resolution chain and env-var reference.

## dsc config check

```text
dsc config check [--format text|json|yaml] [--skip-ssh] [--parallel] [--max <n>]
```

`dsc doctor` is a top-level alias with the same options and behaviour.

Probes each configured Discourse and reports two things per install:

- **API** — sends `GET /about.json` with the configured `apikey`/`api_username`. Reports `ok` on 2xx, flags 401/403 with a hint to check credentials, and surfaces other HTTP errors verbatim.
- **SSH** (only when `ssh_host` is set on the entry, and `--skip-ssh` is not passed) — runs `ssh -o BatchMode=yes -o ConnectTimeout=5 <host> true`. Reports the first stderr line on failure so problems are diagnosable at a glance.

It contacts every configured forum over the network (and SSH), so a large fleet can take a while - it prints a signpost up front and, in text mode, **streams each forum's result the moment it lands** (with a `N ok, M failed` summary at the end) rather than buffering the whole table to the end of the run. The signpost and summary go to stderr, so piping the text output (or `--format json`/`yaml`) gives clean, machine-readable results.

- `--parallel` / `-p` probes forums concurrently across a worker pool (default 8, `--max`/`-m` to change), streaming results **fastest-first**. On a large fleet this turns a ~30s sequential sweep into a few seconds.
- `--skip-ssh` skips the (often slowest) SSH probes.

Exits non-zero if any install fails any check, making it suitable for CI or pre-deploy gates.

Examples:

```bash
dsc config check
dsc config check -p               # probe all forums in parallel (fastest-first)
dsc config check --format json
dsc config check --skip-ssh       # API-only check, much faster
dsc doctor --parallel              # alias, with concurrent probes
```

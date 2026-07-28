# dsc app

Inspect Docker-level configuration for self-hosted Discourse installs over SSH.

## dsc app env list

```text
dsc app env list <discourse> [--format text|json|yaml]
```

Lists sorted, non-secret keys in the `env:` mapping of the configured forum's `app.yml`. The default remote path is `/var/discourse/containers/app.yml`; set `app_yml_path` in that forum's `dsc.toml` entry for a nonstandard layout. Values are never shown by `list`.

```bash
dsc app env list myforum
dsc app env list myforum --format json
```

## dsc app env get

```text
dsc app env get <discourse> <key> [--show-secret] [--format text|json|yaml]
```

Reads one scalar environment variable. Likely secret names, including password, token, API key, access key, private key, SMTP, and database settings, are redacted unless `--show-secret` is explicitly supplied for that one forum.

```bash
dsc app env get myforum DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE
dsc app env get myforum DISCOURSE_SMTP_PASSWORD --show-secret
```

## dsc app env audit

```text
dsc app env audit <key> [--tags <tag1,tag2,...>] [--format text|json|yaml]
```

Checks one key across every configured forum, optionally narrowed by tags. Unset keys, SSH/configuration errors, and non-secret values are shown per forum. Secret values are always redacted in audit output, even if they exist on the host.

```bash
dsc app env audit DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE
dsc app env audit DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE --tags production --format json
```

## dsc app env set and unset

```text
dsc app env set <discourse> <key> <value> [--rebuild] [--no-backup] [--dry-run] --yes
dsc app env unset <discourse> <key> [--rebuild] [--no-backup] [--dry-run] --yes
```

Changes are limited to one scalar entry in a plain `env:` mapping. `dsc` preserves unrelated file content, rejects complex YAML forms rather than rewriting the document, creates a timestamped remote `.bak` copy by default, writes atomically, and re-reads the file to verify the intended result.

Use `--dry-run` to inspect the before/after plan. A live edit requires `--yes`. The setting does not take effect until a rebuild; add `--rebuild` to run `./launcher rebuild app` after a successful edit. `dsc` refuses to start that rebuild if another launcher rebuild is already running.

```bash
dsc app env set myforum DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE 120 --dry-run
dsc app env set myforum DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE 120 --rebuild --yes
dsc app env unset myforum DISCOURSE_CDN_URL --yes
```

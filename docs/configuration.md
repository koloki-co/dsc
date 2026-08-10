# Configuration

If `--config <path>` is not provided, `dsc` resolves the active config in this order (first match wins):

1. `--config <path>` / `-c` flag (explicit)
2. `$DSC_CONFIG` env var (explicit)
3. `./dsc.toml` (current working directory)
4. `$DSC_CONFIG_HOME/dsc.toml` (defaults to `$XDG_CONFIG_HOME/dsc/dsc.toml`, which itself defaults to `~/.config/dsc/dsc.toml`)
5. `$XDG_CONFIG_DIRS` entries as `<dir>/dsc/dsc.toml` (or `/etc/xdg/dsc/dsc.toml` when `XDG_CONFIG_DIRS` is unset)
6. `/etc/dsc/dsc.toml`
7. `/etc/dsc.toml`
8. `/usr/local/etc/dsc.toml`

If none are found, it defaults to `./dsc.toml` (created on first write command).

**Explicit selectors error on missing files.** `-c <path>` and `$DSC_CONFIG` both name a specific file; if that file does not exist, `dsc` errors rather than silently falling through to a lower-precedence config. The discovered hierarchy (steps 3-8) is the only set of paths eligible for skip-if-missing.

If both `-c` and `$DSC_CONFIG` are set, the flag wins.

All CLI path arguments expand a bare `~` or leading `~/` themselves, including quoted paths and `--flag=~/path` forms that the shell leaves untouched. Path-valued configuration environment variables use the same expansion.

## Env-var reference

| Variable | Purpose |
|---|---|
| `DSC_CONFIG` | Absolute or relative path to a `dsc.toml`. Overrides discovery; errors if missing. |
| `DSC_CONFIG_HOME` | Directory containing `dsc.toml`. Defaults to `$XDG_CONFIG_HOME/dsc` -> `~/.config/dsc`. |
| `XDG_CONFIG_HOME` | Standard XDG base; used when `DSC_CONFIG_HOME` is unset. |
| `XDG_CONFIG_DIRS` | Standard XDG system list; each entry contributes a `<dir>/dsc/dsc.toml` candidate. |

Run `dsc config` to see env-var state, the active config, and (when discovery applies) the full search order with markers.

Each Discourse instance lives under a `[[discourse]]` table. See [dsc.example.toml](../dsc.example.toml) for a fuller template. Minimum useful fields are `name`, `baseurl`, `apikey`, and `api_username`.

```toml
[[discourse]]
name = "myforum"
fullname = "My Forum"
baseurl = "https://forum.example.com"
apikey = "your_api_key_here"
api_username = "system"
changelog_topic_id = 123
ssh_host = "forum.example.com"
tags = ["production", "client-a"]
```

## Field reference

| Field | Required | Description |
|---|---|---|
| `name` | yes | Short slugified identifier (no spaces). |
| `baseurl` | yes | Forum URL, no trailing slash. |
| `apikey` | for API commands | Discourse API key. |
| `api_username` | for API commands | User to act as (usually `system`). |
| `fullname` | no | Display name / site title. Auto-populated by `dsc add` and `dsc import` when fetchable. |
| `ssh_host` | for `update` | SSH config host name for remote updates. |
| `docker_rootless` | no | Set `true` when the instance uses rootless Docker (e.g. provisioned by `dsc harden`). Drops `sudo` from Docker/launcher commands during `dsc update`. Default: `false`. |
| `changelog_topic_id` | for changelog | Topic ID for update changelog posts. |
| `tags` | no | Labels for organising installs; used with `--tags` filtering. |
| `discourse_branch` | no | Discourse branch to compare against during `dsc update` (default: `latest`). Set to `stable` for sites that track the Discourse stable branch. The `--branch` CLI flag overrides this per-run. |
| `enabled` | no | Defaults to `true`. Set `false` to skip in bulk operations. |

## Notes

- `dsc add` without `--interactive` appends a full `[[discourse]]` template containing every supported config key, using placeholders like `""`, `[]`, and `0`.
- Empty strings and `0` values are treated as "unset" (most commands behave as if the key is missing).
- Most forum read/write commands require `apikey` and `api_username`. If they are missing, `dsc` will fail with a clear message.
- SSH credentials are not stored in `dsc.toml`; set up SSH keys and use an SSH config file.
- `apikey` is stored in plain text. Restrict the file to your own user (`chmod 600 dsc.toml`) and keep it out of version control.
- CLI inventory commands do not print stored API keys, and `dsc list` has no credential-reveal flag. Access the protected `dsc.toml` directly when credential administration is necessary.

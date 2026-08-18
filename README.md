# dsc

<p align="center">
  <img src="docs/assets/dsc-logo-black-on-orange.svg" alt="dsc" width="96">
  <br>
  <sub>Created by</sub>
  <br>
  <a href="https://koloki.co/"><img src="docs/assets/koloki-logo-master-2x.png" alt="Koloki" width="220"></a>
</p>

A Discourse CLI tool written in Rust. Manage multiple Discourse forums from your terminal — track installs, run upgrades over SSH, manage emojis, perform backups, and sync topics and categories as local Markdown.

Created by Marcus Baw and [Koloki Ltd](https://koloki.co/), an [official Discourse Partner](https://www.discourse.org/partners).

Most functionality uses the Discourse REST API. `dsc update` runs remote rebuilds via SSH.

## Features

- Track any number of Discourse installs via a single config file.
- Manage categories, topics, settings, and groups across installs.
- Run rebuilds over SSH and optionally post changelog updates.
- Import from text or CSV, or add installs ad-hoc.
- Pull/push individual topics or whole categories as Markdown.
- Upload custom emojis in bulk.
- List, install, and remove themes and plugins; configure outbound webhooks.
- Create, list, and restore backups.

## What works today

A glance at where `dsc` is, so you can tell whether it covers your use case before installing. Almost everything below works over the Discourse REST API; server lifecycle uses SSH.

| Area | Works now | On the roadmap |
|---|---|---|
| **Topics & posts** | pull / push / sync as Markdown, reply, create, rename (`topic title`), set tags, full-thread export; per-post edit / delete / move | — |
| **Categories** | list; pull / push a whole category as Markdown with durable `topic_id` binding, a reviewable dry-run plan, `--updates-only`, `--no-bump`, admonition/link conversion; declarative definition pull/push/diff/set/rename | definition assets and guarded prune |
| **Tags** | pull / push the taxonomy, rename, per-topic tag / untag | — |
| **Site settings** | get / set / list, pull / push snapshots, diff two sources, **audit one setting across every forum** | — |
| **Themes & palettes** | list / install / remove / pull / push / duplicate / show; component settings and fields; assets; enable / disable; attach / detach; colour palettes; remote update | — |
| **Users & access** | list / info / find-by-email, suspend / silence, promote / demote, group membership, activity export, create, password reset, email change; invites; private messages; API keys; **one-shot SAR / GDPR export** | scoped API keys |
| **Backups, emoji, uploads** | backup create / list / pull / push / restore, fleet backup creation and S3 health/setup, bulk emoji, file upload | S3 key rotation and retention |
| **Fleet (multi-install)** | one config for N forums, tag filtering, write/audit settings, update-all over SSH, cross-forum search and user lookup, backup fan-out | aggregate reports |
| **Server lifecycle** | `harden` a fresh box, stages 1-2 (new sudo user, pubkey auth, sshd lockdown to a non-standard port); `update` over SSH with skip-if-current | `harden` stage 3 - firewall, Docker, swap, fail2ban (config keys wired, SSH execution pending); one-shot `dsc install` provisioning |
| **Reporting** | analytics snapshot (growth / activity / health); saved Data Explorer query inspection/execution; `log staff` audit-trail inspection | dashboard reports, notifications |

Exploratory (not committed): `dsc chat`, a TUI, and an MCP server mode. See [spec/roadmap.md](spec/roadmap.md) for the full picture.

## Installation

### Shell installer — Linux and macOS

```bash
curl -LsSf https://dsc.koloki.co/install.sh | sh
```

Downloads a prebuilt binary for your platform and installs it to `~/.cargo/bin` (or `$CARGO_HOME/bin` if set). Supports `x86_64` and `aarch64` on both Linux and macOS.

This short URL proxies to cargo-dist's real installer on the [latest GitHub release](https://github.com/koloki-co/dsc/releases/latest) — fine for most purposes, but if you'd rather pin to a specific version or audit the script you can fetch it directly from the release assets.

### PowerShell installer — Windows

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://dsc.koloki.co/install.ps1 | iex"
```

Downloads the Windows `x86_64` binary and installs it to `%CARGO_HOME%\bin`.

### Homebrew — Linux and macOS

```bash
brew tap pacharanero/tap
brew install dsc-rs
```

The formula name matches the crate name (`dsc-rs`); the installed binary is still `dsc`.

### Windows installer (MSI)

Download `dsc-rs-x86_64-pc-windows-msvc.msi` from the [latest release](https://github.com/koloki-co/dsc/releases/latest) and double-click. The installer is unsigned, so Windows will show a SmartScreen warning the first time — click "More info" → "Run anyway".

### From crates.io

If you already have cargo-binstall, use the prebuilt release archive:

```bash
cargo binstall dsc-rs
```

Otherwise, build from crates.io with a Rust toolchain:

```bash
cargo install dsc-rs
```

The crate is published as `dsc-rs` (the `dsc` name was taken), but the installed binary is still `dsc`.

### Direct download

Prebuilt archives for Linux, macOS, and Windows are attached to every [GitHub release](https://github.com/koloki-co/dsc/releases/latest). Download, extract, and drop `dsc` (or `dsc.exe`) anywhere on your `PATH`. Each archive has a sibling `.sha256` file, and `sha256.sum` contains the unified archive, MSI, and source checksum set in `sha256sum -c` compatible format.

### From source

Requires Rust 1.95.0 or newer (install via [rustup](https://rustup.rs)).

```bash
git clone https://github.com/koloki-co/dsc.git
cd dsc
cargo install --path .
```

## Quick start

```bash
# Create a config file
cat > dsc.toml <<'EOF'
[[discourse]]
name = "myforum"
baseurl = "https://forum.example.com"
apikey = "<api key>"
api_username = "system"
ssh_host = "forum.example.com"
changelog_topic_id = 123
EOF

# dsc.toml holds a live API key in plain text - keep it readable only by you
chmod 600 dsc.toml

# Verify API (and SSH, if configured) connectivity before doing anything else
dsc doctor

# List configured forums (read-only)
dsc list

# Pull a topic into Markdown for editing (read-only)
dsc topic pull myforum 42

# Preview a push before it touches the live forum
dsc --dry-run topic push myforum 42 ./topic-title.md

# Push the edited topic back up for real
dsc topic push myforum 42 ./topic-title.md

# Update a forum over SSH - a real remote rebuild, no dry-run preview for this one
dsc update myforum
```

## Documentation

- [Configuration](docs/configuration.md) — config file format, search order, field reference
- [Compatibility](docs/compatibility.md) — the 1.x CLI, Rust, and Discourse support contract
- **Commands:**
  - [list](docs/list.md) — list and filter installs
  - [open](docs/open.md) — open a Discourse in the browser
  - [add](docs/add.md) — add installs to config
  - [import](docs/import.md) — import installs from file or stdin
  - [update](docs/update.md) — run OS and Discourse updates over SSH
  - [search](docs/search.md) — search topics on a Discourse
  - [analytics](docs/analytics.md) — community-health snapshot (growth, activity, health)
  - [explorer](docs/explorer.md) — inspect and run saved Data Explorer queries
  - [upload](docs/upload.md) — upload a file and return its short URL
  - [emoji](docs/emoji.md) — upload and list custom emoji
  - [topic](docs/topic.md) — pull, push, and sync topics as Markdown
  - [post](docs/post.md) — edit, delete, and move individual posts
  - [category](docs/category.md) — list, pull, push, and copy categories
  - [palette](docs/palette.md) — list, pull, and push colour palettes
  - [plugin](docs/plugin.md) — list, install, and remove plugins
  - [theme](docs/theme.md) — install (git/bundle), delete, list, pull, push, duplicate; edit settings, fields (SCSS), and assets; enable/attach components; update remotes
  - [group](docs/group.md) — list, inspect, copy, and bulk-add members
  - [user](docs/user.md) — list, inspect, suspend, archive activity, and manage group memberships
  - [sar](docs/sar.md) — export everything a forum holds about one person as a GDPR Subject Access Request bundle
  - [invite](docs/invite.md) — send invites, single or bulk from a file
  - [pm](docs/pm.md) — send and list private messages
  - [api-key](docs/api-key.md) — manage Discourse API keys
  - [webhook](docs/webhook.md) — list, create, delete, and ping outbound webhooks
  - [backup](docs/backup.md) — create, list, restore, and provision S3 off-site backups
  - [setting](docs/setting.md) — get, set, pull, push, and diff site settings
  - [tag](docs/tag.md) — list, pull, push, and rename the tag taxonomy (per-topic tagging lives under `dsc topic tag`/`untag`)
  - [config](docs/config.md) — inspect and validate the dsc config itself
  - [version](docs/version.md) — dsc's own version, or a forum's live Discourse version + commit
  - [harden](docs/harden.md) — provision a fresh Ubuntu server: new sudo user + pubkey auth, sshd lockdown (stages 1-2, shipped); firewall/Docker/swap/fail2ban is stage 3, still WIP
- [Shell completions](docs/completions.md) — bash, zsh, and fish
- [Man pages](docs/manpages.md) — generate Unix man pages for `dsc` and every subcommand
- [Development](docs/development.md) — building, testing, releasing, project layout
- [Contributing](CONTRIBUTING.md) — bug vs spec request, code conventions, support stance
- [For LLMs and agents](AGENTS.md) — using `dsc` from another AI session, and how to file a useful feature spec

## Support

For questions about administering Discourse, ask on [Discourse Meta](https://meta.discourse.org). For `dsc` bugs, feature requests, or proposed command surfaces, follow the routes in [Contributing](CONTRIBUTING.md). Security vulnerabilities belong in a private [GitHub Security Advisory](https://github.com/koloki-co/dsc/security/advisories/new) or email, never a public issue.

## License

`dsc` source code and documentation are licensed under [GPL-2.0-or-later](LICENSE). Third-party assets keep their own licences and attribution; see [NOTICE.md](NOTICE.md) and [REUSE.toml](REUSE.toml).

Koloki Ltd is an official Discourse Partner. `dsc` itself is an independent open-source project and is not endorsed by Civilized Discourse Construction Kit, Inc. (CDCK). Discourse is a trademark of CDCK.

The selected Font Awesome Free 6.6.0 icons used by the Discourse UI are copyright 2024 Fonticons, Inc. and licensed under [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/). Font Awesome brand icons remain trademarks of their respective owners and are used only to identify the represented products or services.

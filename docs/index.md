# dsc

A Discourse CLI written in Rust. Manage multiple Discourse forums from your terminal — track installs, run upgrades over SSH, manage users and groups, sync topics and categories as local Markdown, upload files, search, archive activity, and more.

Created by Marcus Baw and [Koloki Ltd](https://koloki.co/), an [official Discourse Partner](https://www.discourse.org/partners).

Most functionality uses the Discourse REST API; `dsc update` runs remote rebuilds via SSH.

## Install

=== ":simple-linux: Linux / :simple-apple: macOS"

    One-liner shell installer:

    ```bash
    curl -LsSf https://koloki-co.github.io/dsc/install.sh | sh
    ```

    Drops a prebuilt binary into `~/.cargo/bin` (or `$CARGO_HOME/bin`).
    Supports `x86_64` and `aarch64`.

=== ":simple-homebrew: Homebrew"

    Linux or macOS, no Rust toolchain needed:

    ```bash
    brew tap pacharanero/tap
    brew install dsc-rs
    ```

=== ":material-microsoft-windows: Windows (PowerShell)"

    One-liner:

    ```powershell
    powershell -ExecutionPolicy Bypass -c "irm https://koloki-co.github.io/dsc/install.ps1 | iex"
    ```

    Drops `dsc.exe` into `%CARGO_HOME%\bin`.

=== ":material-microsoft-windows: Windows (MSI)"

    Prefer a graphical installer? Download the `.msi` for
    `x86_64-pc-windows-msvc` from the
    [latest release](https://github.com/koloki-co/dsc/releases/latest)
    and double-click. Unsigned, so SmartScreen will warn the first
    time — click "More info" → "Run anyway".

=== ":simple-rust: Cargo"

    Install the prebuilt archive with cargo-binstall:

    ```bash
    cargo binstall dsc-rs
    ```

    Or build from crates.io with Rust 1.95.0 or newer:

    ```bash
    cargo install dsc-rs
    ```

    The crate is named `dsc-rs` (the `dsc` name on crates.io was
    taken); the installed binary is always `dsc`.

See the [project README](https://github.com/koloki-co/dsc#readme) for
direct-download archives and other paths in.

## A minimal config

`dsc` reads `dsc.toml`. The only required fields are `name` and `baseurl`:

```toml
[[discourse]]
name = "myforum"
baseurl = "https://forum.example.com"
apikey = "<admin api key>"       # required for anything that isn't public read
api_username = "system"
ssh_host = "forum.example.com"   # only needed for `dsc update`
```

Full field reference: [Configuration](configuration.md).

## What `dsc` does well

### 1. Multi-install operations

One config describes every forum you run. Act on all of them at once:

```bash
dsc list                         # tag-filterable overview
dsc update all --parallel        # rebuilds over SSH across every forum
dsc setting set --tags production site_name "New Name"
dsc doctor                       # alias for config check: verify API + SSH reachability
```

### 2. Topics and categories as Markdown

Edit content in your editor, commit it to git, run it through AI drafting — whatever your workflow demands:

```bash
dsc topic pull myforum 1234 ./drafts/
dsc topic push myforum 1234 ./drafts/edited.md
dsc category pull myforum support ./support-category/
```

### 3. Composable read/write primitives

Every write command reads from stdin or a file, so shell pipelines just work:

```bash
git log --since=yesterday --oneline | dsc topic reply myforum 1525
df -h | dsc topic new myforum 42 --title "Disk report $(date -I)"
dsc user activity meta pacharanero --since 7d --format markdown \
  | dsc topic reply myjournalforum 1234
```

## Command index

Browse by area:

- **Content** — [`topic`](topic.md), [`post`](post.md), [`category`](category.md), [`search`](search.md), [`upload`](upload.md), [`tag`](tag.md), [`emoji`](emoji.md)
- **Users & access** — [`user`](user.md), [`group`](group.md), [`invite`](invite.md), [`pm`](pm.md), [`api-key`](api-key.md)
- **Install management** — [`list`](list.md), [`add`](add.md), [`import`](import.md), [`open`](open.md), [`update`](update.md), [`config`](config.md)
- **Site admin** — [`setting`](setting.md), [`log`](log.md), [`backup`](backup.md), [`theme`](theme.md), [`plugin`](plugin.md), [`palette`](palette.md)
- **Meta** — [Shell completions](completions.md), [Compatibility](compatibility.md), [Development](development.md)

## Safe by default

- Side-effecting operations (`backup restore`, `setting set`, `topic push`, plugin/theme install/remove, and more) honour a global `--dry-run` / `-n` flag. Commands without a complete plan refuse rather than executing.
- HTTP 429 responses are retried automatically with the Retry-After header the server returns.
- Read-only commands like `user activity` work without an API key on forums that allow public reads.

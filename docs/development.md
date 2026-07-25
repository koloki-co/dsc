# Development

The supported Rust toolchain and 1.x CLI, library, and Discourse compatibility commitments are recorded in [Compatibility](compatibility.md).

## Building

```bash
# Fast feedback build
cargo build

# Optimized build
cargo build --release

# Run locally
cargo run -- --help
```

## Linting

```bash
cargo fmt
cargo clippy
```

## Testing

Standard test suite:

```bash
cargo test
```

The full CI-mirroring gate (fmt + clippy `-D warnings` + the complete test suite, with the live e2e tests skipped) is one script - a green run here means a green run in CI, and `s/version++` runs it before cutting a release:

```bash
s/test-fmt-clippy
```

Live compatibility tests require an explicit opt-in and an absolute path to a credential file. They run serially and may mutate their configured disposable resources, so point them only at an isolated test forum:

```bash
DSC_LIVE_TESTS=1 TEST_DSC_CONFIG=/absolute/path/to/live-test.toml s/test-live
```

Add `DSC_TEST_VERBOSE=1` before that command for test diagnostics. Ordinary `cargo test`, CI, and `s/test-fmt-clippy` never contact Discourse.

The live-test configuration uses the shape below:

```toml
[[discourse]]
name = "myforum"
baseurl = "https://forum.example.com"
apikey = "<admin api key>"
api_username = "system"
changelog_topic_id = 123        # optional unless testing update changelog posting
test_topic_id = 456             # topic used by e2e topic tests
test_category_id = 789          # category used by e2e category tests
test_color_scheme_id = 321      # palette used by e2e palette tests
emoji_path = "./smile.png"     # optional; enables emoji add test
emoji_name = "smile"
test_plugin_url = "https://github.com/discourse/discourse-reactions"
test_plugin_name = "discourse-reactions"
test_theme_url = "https://github.com/discourse/discourse-brand-header"
test_theme_name = "discourse-brand-header"
```

## Shell completions

Completion scripts are generated on demand by the binary itself — they are not committed to the repo. Regenerate them for any supported shell with:

```bash
cargo run -- completions zsh  --dir completions/
cargo run -- completions bash --dir completions/
cargo run -- completions fish --dir completions/
```

The `completions/` directory is gitignored. See [docs/completions.md](completions.md) for user-facing installation instructions.

## Documentation site

The docs you're reading are built with [Zensical](https://zensical.org) and deployed to GitHub Pages by `.github/workflows/deploy-docs-to-ghpages.yml` on every push to `main` that touches `docs/`, `mkdocs.yml`, or `requirements.txt`.

To preview locally:

```bash
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
s/docs                 # serves at http://localhost:8000 with hot reload
```

`s/docs` is a thin wrapper around `zensical serve`.

### Gotcha: inotify instances on Linux

Zensical's file watcher consumes inotify *instances*. The kernel default on most Linux distros is `fs.inotify.max_user_instances=128`, which runs out fast once you've got editors, file syncers and the like open. Symptoms:

```text
thread 'zrx/monitor' panicked … Too many open files
Build started
```

Fix (one-liner to try the current session, then the persistent form):

```bash
sudo sysctl fs.inotify.max_user_instances=512
echo 'fs.inotify.max_user_instances=512' | sudo tee /etc/sysctl.d/99-inotify.conf
sudo sysctl --system
```

The `s/docs` script warns if the limit is at its stock 128 value.

## Release

Releasing is **one action**: `s/version++`.

1. Commit your feature work first, with a conventional-commit message (`feat(...)`, `fix:`, …) - git-cliff builds the changelog from committed history.
2. Run `s/version++ [patch|minor|major]` (default `patch`). It refuses a dirty or unsynchronised `main`, runs the full local gate, bumps `Cargo.toml`, regenerates `CHANGELOG.md`, and creates `chore(release): vX.Y.Z`. With protected `main` it opens a `release/vX.Y.Z` PR; otherwise it pushes the release commit directly. `--pr` and `--direct` override automatic protection detection.
3. `auto-tag.yml` creates `vX.Y.Z` only after the release commit reaches `main`, then invokes the cargo-dist `Release` and `Publish to crates.io` workflows. The latter obtains a short-lived token through crates.io Trusted Publishing; it does not require a long-lived `CARGO_REGISTRY_TOKEN` secret.

There is no separate `s/release` step. If creating the release PR fails, the release commit remains only on the local `release/vX.Y.Z` branch; fix the failure and retry the push/PR creation. No tag or public release exists until that PR merges.

If auto-tag fails before pushing its tag, repair the workflow in a normal PR, then use its manual dispatch with the exact tag and the merged release commit. Recovery refuses a mismatched version, a pre-existing tag, or a commit outside `main`, and invokes the same release and publication workflows as the ordinary cascade.

## Project layout

- CLI entrypoint and commands: [src/main.rs](../src/main.rs)
- API client and forum interactions: [src/discourse.rs](../src/discourse.rs)
- Config structures and helpers: [src/config.rs](../src/config.rs)
- Utility helpers (slugify, I/O): [src/utils.rs](../src/utils.rs)
- Example configuration: [dsc.example.toml](../dsc.example.toml)
- CLI design standards: [spec/cli-design.md](../spec/cli-design.md); internals/release: [spec/spec.md](../spec/spec.md)
- Project scripts: [s/](../s) (house style - see below)
- Windows installer sources: [wix/](../wix) - MSI build artefacts consumed by the cargo-dist Windows target, not invoked directly during development

### `s/` scripts

Repo-local dev scripts live under `s/` (not `scripts/`) and are run as `s/<name>`, e.g. `s/lint`, `s/docs`, `s/version++`. This is a deliberate house-style convention, not a typo: `s/` keeps the invocation short and greppable. See the scripts themselves for what each does; the ones referenced elsewhere in this doc are `s/test-fmt-clippy` (CI-mirroring gate), `s/docs` (docs preview server), and `s/version++` (release).

# `dsc` config-path resolution spec

> **Status: Implemented in v0.10.9.** `$DSC_CONFIG` and `$DSC_CONFIG_HOME` are
> live; `dsc config` surfaces them and labels the active source. Resolution
> chain matches the spec below.

Spec for how `dsc` locates its `dsc.toml` when `--config` is not given. Goal: a standard, predictable hierarchy that an average Unix user expects, mirroring the author's other tool [`sct`](https://pacharanero.github.io/sct/path-resolution/#config-file-resolution) so the two share one mental model. Backward-compatible: every path that resolves today still resolves after this change.

## Motivation

`dsc` already walks a search hierarchy (`src/config.rs::config_search_paths`), but it offers no way to point at a config that lives outside those slots without passing `-c` on every invocation. A user whose config sits in, say, `~/code/discourse/dsc/dsc.toml` gets "discourse not found" from any other directory. `sct` solves the same problem with a `$SCT_CONFIG` env-var override and a `$SCT_CONFIG_HOME` knob. This spec brings the same two knobs to `dsc` and writes the precedence down as a contract.

## Current state (as of 2026-06-09)

`resolve_default_config_path()` returns the first existing path from `config_search_paths()`, defaulting to `./dsc.toml` if none exist. The `-c/--config` flag, when present, bypasses the search entirely. Order today:

1. `./dsc.toml`
2. `$XDG_CONFIG_HOME/dsc/dsc.toml` (or `~/.config/dsc/dsc.toml` when `XDG_CONFIG_HOME` is unset)
3. `$XDG_CONFIG_DIRS` entries as `<dir>/dsc/dsc.toml` (or `/etc/xdg/dsc/dsc.toml` when unset) `[unix]`
4. `/etc/dsc/dsc.toml` `[unix]`
5. `/etc/dsc.toml` `[unix]`
6. `/usr/local/etc/dsc.toml` `[unix]`

Gaps vs `sct`: no `$DSC_CONFIG` file override, no `$DSC_CONFIG_HOME` directory override, and the precedence is documented as an implementation detail rather than a stable contract.

## Resolution chain (first match wins)

This is the proposed order. Bold entries are new; the rest are today's behaviour, renumbered.

1. **`--config <path>` / `-c` flag** - explicit selection. (Exists today.)
2. **`$DSC_CONFIG` env var** - explicit file path. *(new)*
3. `./dsc.toml` - project-local, current working directory.
4. **`$DSC_CONFIG_HOME/dsc.toml`** - user config dir. *(new env var; the default value reproduces today's step 2.)*
5. `$XDG_CONFIG_DIRS` entries as `<dir>/dsc/dsc.toml` (or `/etc/xdg/dsc/dsc.toml` when unset) `[unix]`
6. `/etc/dsc/dsc.toml` `[unix]`
7. `/etc/dsc.toml` `[unix]`
8. `/usr/local/etc/dsc.toml` `[unix]`

If none exist, fall back to `./dsc.toml` (created on first write command), unchanged from today.

### `$DSC_CONFIG_HOME` defaulting

`$DSC_CONFIG_HOME` defaults to `$XDG_CONFIG_HOME/dsc`, and `$XDG_CONFIG_HOME` itself defaults to `~/.config`. So with nothing set, step 4 resolves to `~/.config/dsc/dsc.toml` - byte-for-byte today's step 2. The env var only changes behaviour when a user deliberately sets it. This is exactly `sct`'s `$SCT_CONFIG_HOME -> $XDG_CONFIG_HOME/sct -> ~/.config/sct` chain, with `dsc` substituted.

Note on filename: `sct` stores `config.toml` inside its config-home dir; `dsc` keeps `dsc.toml` for backward compatibility (existing installs and `dsc add`/`dsc import` all write `dsc.toml`). This is the one intentional divergence from `sct`.

## Mapping to `sct` (for retention)

| Concept | `sct` | `dsc` |
|---|---|---|
| Explicit flag | `--config` | `-c` / `--config` |
| Env override (full file path) | `$SCT_CONFIG` | `$DSC_CONFIG` |
| Project-local file | `./sct.toml` | `./dsc.toml` |
| User config-home dir env | `$SCT_CONFIG_HOME` | `$DSC_CONFIG_HOME` |
| Config-home default | `$XDG_CONFIG_HOME/sct` -> `~/.config/sct` | `$XDG_CONFIG_HOME/dsc` -> `~/.config/dsc` |
| Filename inside config-home | `config.toml` | `dsc.toml` |
| System-wide fallbacks | (not documented) | `$XDG_CONFIG_DIRS`, `/etc/...`, `/usr/local/etc` |
| No config found | empty defaults | default to `./dsc.toml` |

## Explicit-selector semantics (safety)

`-c <path>` and `$DSC_CONFIG` are *explicit selectors*: the user has named a specific file. If that file does not exist, a command that uses configuration must **error**, not silently fall through to a lower-precedence config. Rationale: a typo in a path must not cause `dsc` to quietly act against a different forum using whichever other config it happens to find. The discovered paths (steps 3-8) are the only ones eligible for silent skip-if-missing. Config-free commands (`dsc version` without a forum, `dsc completions`, and `dsc man`) deliberately do not resolve or validate configuration, even when a selector is present.

Precedence between the two explicit selectors: the `-c` flag wins over `$DSC_CONFIG` if both are set (an explicit per-invocation argument is more specific than an environment default).

## `dsc config` output

The `dsc config` (no subcommand) printout, which lists the search order and marks the active file, must reflect the new chain:

- Show whether `$DSC_CONFIG` and `$DSC_CONFIG_HOME` are set, and their values, near the top.
- Include steps 2 and 4 in the numbered search-order list, with `(exists)` / ` <-- active` markers as today.
- When the active config came from `-c` or `$DSC_CONFIG`, label the source explicitly (e.g. `via --config flag` / `via $DSC_CONFIG`) so the user can see why a path outside the standard hierarchy is active.

## Platform notes

- **macOS** is covered by the `[unix]` XDG path, same as `sct`. Native `~/Library/Application Support/dsc/` is a non-goal (keeps parity with `sct`).
- **Windows**: the `[unix]`-gated system paths do not apply. Today Windows users get the flag, `$DSC_CONFIG` (new), `./dsc.toml`, and `$DSC_CONFIG_HOME`/XDG-or-`~/.config`. A native `%APPDATA%\dsc\dsc.toml` slot is a reasonable follow-up but out of scope for this change.

## Implementation touch points

- `src/config.rs`
  - `config_search_paths()`: insert `$DSC_CONFIG_HOME` resolution in place of the hard-coded `~/.config/dsc` step (with the documented default so behaviour is unchanged when unset).
  - `resolve_default_config_path()` / its caller: consult `$DSC_CONFIG` before the discovered list; keep the explicit-selector error semantics above.
- `src/main.rs` (~line 782): update the `dsc config` printout to surface the env vars and label the active source.
- `-c/--config` handling: confirm a missing explicit path errors rather than falls through (apply the same to `$DSC_CONFIG`).
- Docs: `docs/configuration.md` (resolution list + env-var reference), `README.md` if it restates the order, `src/cli.rs` help text for `--config`.
- `dsc.example.toml`: a comment pointing at `$DSC_CONFIG` / `$DSC_CONFIG_HOME` for users who keep config outside the standard dirs.

## Tests

- `$DSC_CONFIG` set to an existing file wins over `./dsc.toml`.
- `$DSC_CONFIG` set to a missing file errors (does not fall through).
- `-c` flag wins over `$DSC_CONFIG` when both are set.
- `$DSC_CONFIG_HOME` set redirects step 4; unset reproduces `~/.config/dsc/dsc.toml`.
- Unset-everything resolution order is byte-for-byte identical to today (regression guard).
- `$XDG_CONFIG_HOME` interaction with `$DSC_CONFIG_HOME` default is correct.

## Non-goals

- Ancestor-directory walk (git-style upward search for `./dsc.toml`). Could be a later extension but is not part of matching `sct`.
- Accepting `config.toml` as an alternate filename inside the config-home dir.
- Merging multiple config files. First match wins; no layering.

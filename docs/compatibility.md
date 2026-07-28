# Compatibility

This is the compatibility contract for the `dsc` 1.x series. It takes effect with `v1.0.0`; pre-1.0 releases remain subject to change.

## CLI

The commands, subcommands, positional arguments, flags, and documented aliases shown by `dsc --help` are stable throughout 1.x. New commands and optional flags may be added in a minor release. Existing elements are not renamed, removed, or given incompatible meanings during 1.x.

Command results are written to stdout. Hints, progress, deprecation notices, and errors are written to stderr. A successful command exits with status 0; any failure exits non-zero. Exact non-zero values are not part of the 1.x contract.

Where a command offers `--format json` or `--format yaml`, its object or collection shape is stable during 1.x. Fields may be added, but existing fields do not change meaning or type. Text output is for people and may change unless a command page explicitly calls a line machine-readable.

## Deprecation

Deprecated CLI elements print a stderr warning naming the replacement. They remain available for the whole 1.x series and will not be removed before 2.0.0, except where retaining an element would create a security issue.

`dsc palette` is the current deprecated alias; use `dsc theme palette` instead.

## Command Decisions

`dsc open` remains a supported interactive helper. It opens the configured forum URL through the platform browser opener (or `DSC_BROWSER_OPENER`) and is intentionally refused by `--dry-run`, because opening a browser has no meaningful plan-only equivalent.

`dsc import` remains the supported bulk-onboarding path. It reads one URL per line or CSV (`name,url,tags`) from a file or stdin, appends entries to `dsc.toml`, and looks up each site title on a best-effort basis. A failed title lookup does not stop the import and is reported on stderr.

## Rust API

`dsc-rs` is a binary distribution, not a supported Rust library. Its public Rust modules exist to share implementation between the binary and the repository's tests; they are not a stable API and may change in any release. Depend on the `dsc` executable rather than linking `dsc-rs` from another Rust project.

## Rust Toolchain

The minimum supported Rust version (MSRV) is Rust 1.95.0. It is declared in `Cargo.toml` and tested in CI alongside the current stable toolchain. Source installs require Rust 1.95.0 or newer.

## Discourse

`dsc` supports the current upstream stable Discourse release on a best-effort, no-SLA basis. At the time this contract was written (July 2026), the supported release is `2026.7.0-latest`; older Discourse releases may work but are not supported. The Discourse admin API is not formally versioned, so report failures with `dsc version <forum>` output and the command used.

Live endpoint observations exist for Discourse 3.x (March 2026), `2026.6.0-latest`, and `2026.7.0-latest`. They are recorded in the relevant command specs and are not a complete compatibility matrix. The isolated `s/test-live` compatibility runner was validated twice against `2026.7.0-latest` (`216dc56395c9c678c36e772a8bbf3ed04b1e7fcb`) on 2026-07-27, including concurrent-run refusal and forced-failure cleanup.

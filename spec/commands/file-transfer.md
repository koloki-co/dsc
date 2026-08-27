# `dsc file` - safe file transfer over SSH

Spec for checksum-driven file transfer between the operator's machine and configured Discourse hosts. Goal: make a repeated, safety-critical fleet operation reviewable and reproducible without turning `dsc` into a general remote-command runner. Driver: deploying the canonical fallback update script from the administrative workspace to 16 managed standard `/var/discourse` hosts.

## Motivation

The fallback updater lives at `scripts/update.sh` in the administrative workspace and is installed as `/var/discourse/scripts/update.sh` on each managed host. It was deployed to all 16 hosts on 2026-08-26, then changed again on 2026-08-27 after a real update showed that plain `docker image prune` did not remove old, still-tagged `discourse/base` images. Today the operator must generate a host inventory with `dsc list --format json`, combine it with `jq`, `scp`, `ssh`, `sudo install`, timestamped backups, remote `bash -n`, and SHA-256 comparisons, then manually reconcile partial failures. The individual tools work, but the composition repeats fleet selection and SSH policy already owned by `dsc` and is easy to get subtly wrong.

## Current state (as of 2026-08-27)

`dsc` has no generic `file`, `copy`, `scp`, `ssh`, or `exec` command. `dsc update all` has SSH reachability and fleet fan-out machinery, but it performs a complete OS/Discourse update and cannot safely be repurposed for file deployment. `dsc app env set` demonstrates backup, atomic replacement, re-read verification, dry-run planning, and rebuild-lock checks for one constrained section of `app.yml`, but it is not a regular-file transfer primitive: it does not provide binary streaming or no-follow filesystem operations.

The current per-host workaround is:

```console
scp scripts/update.sh <ssh-host>:/tmp/discourse-update.sh
ssh <ssh-host> 'bash -n /tmp/discourse-update.sh'
ssh -t <ssh-host> 'sudo -n cp -a /var/discourse/scripts/update.sh /var/backups/discourse-update.sh-<timestamp> && sudo -n install -o root -g root -m 0755 /tmp/discourse-update.sh /var/discourse/scripts/update.sh && rm /tmp/discourse-update.sh'
ssh <ssh-host> '/var/discourse/scripts/update.sh --check'
ssh <ssh-host> 'sudo -n sha256sum /var/discourse/scripts/update.sh'
```

The fleet inventory currently comes from the secret-safe public representation rather than direct parsing of credential-bearing `dsc.toml`:

```console
dsc list --format json | jq -r '.[] | select(.ssh_host != null and .ssh_host != "") | [.name, .ssh_host] | @tsv'
```

## Proposed CLI surface

```text
dsc file audit <DISCOURSE|all> <LOCAL_PATH> <REMOTE_PATH> [--tags <TAGS>] [--parallel[=<N>]] [--format text|json|yaml]
dsc file push  <DISCOURSE|all> <LOCAL_PATH> <REMOTE_PATH> [--tags <TAGS>] [--owner <OWNER>] [--group <GROUP>] [--mode <MODE>] [--backup] [--sudo] [--parallel[=<N>]] [--yes]
dsc file pull  <DISCOURSE|all> <REMOTE_PATH> <LOCAL_PATH> [--tags <TAGS>] [--parallel[=<N>]] [--overwrite]
```

The target is the first positional argument so a fleet mutation is visually prominent: `dsc file push all ...`, not a trailing `--all` that is easy to overlook. Final selector behaviour must align with R48's shared fleet selector rather than introducing another interpretation of `all` or `--tags`. An explicitly empty tag selector must fail rather than matching the fleet.

### `file audit`

- Read-only. Compare the local SHA-256 and size with the remote regular file on each selected host without transferring remote contents.
- Report `same`, `different`, `missing`, or `failed` per forum. Text, JSON, and YAML include the forum name, paths, checksums where available, sizes, and status.
- Continue across fleet failures, print a complete summary, and exit non-zero if any target failed. A missing or different file is a successful audit result, not a transport failure.
- Do not print file contents. This keeps fleet parity checks useful for files that may contain sensitive configuration.

### `file push`

- Require a regular local file and an absolute remote destination. Directories, recursive transfer, globs, devices, sockets, and standard input are out of scope.
- Resolve and print the complete target list under `--dry-run`, along with local checksum and size, destination, whether the remote file is missing/same/different, requested ownership and mode, privilege escalation, and backup path. Dry-run performs read-only remote inspection but no upload, backup, chmod/chown, or replacement.
- Treat a matching remote checksum as an idempotent no-op.
- Upload to a uniquely named temporary regular file in the destination directory, verify its SHA-256, optionally preserve the existing destination with a UTC timestamp, then atomically rename the staged file over the destination on the same filesystem. Remove the staged file after any failure where possible.
- Refuse an existing symlink destination. Do not follow it, even with `--yes`.
- Use the configured SSH account by default. Require explicit `--sudo` before invoking non-interactive `sudo -n`; `--owner root` or another privileged ownership request must not silently imply escalation.
- Apply `--owner`, `--group`, and octal `--mode` to the staged file before replacement. Without those flags, preserve an existing destination's metadata; for a new destination, use a conservative regular-file mode subject to the remote account's umask.
- `--backup` creates a timestamped sibling or operator-visible backup before replacement. Phase 1 may require `--backup` whenever the destination already exists; the implementation must settle one consistent default and expose it in dry-run output.
- Require normal mutating confirmation for one host and explicit `--yes` for non-interactive use. A fleet push always requires `--yes` after a reviewed dry-run.
- Use the shared bounded fleet executor. Continue across host failures, report every outcome, and exit non-zero for a partial result. Do not attempt fleet-wide rollback: retained per-host backups and checksums are the reconciliation evidence.

Example:

```console
dsc -n file push all scripts/update.sh /var/discourse/scripts/update.sh --owner root --group root --mode 0755 --backup --sudo
dsc file push all scripts/update.sh /var/discourse/scripts/update.sh --owner root --group root --mode 0755 --backup --sudo --yes
```

### `file pull`

- Require an absolute remote regular-file path. Refuse symlinks and non-regular files.
- For one forum, treat `LOCAL_PATH` as the exact destination file.
- For `all` or a tag-selected fleet, require `LOCAL_PATH` to be a destination directory and write one flat file per forum. Name each file `<discourse>--<remote-basename>`; the double hyphen visibly separates the configured forum name from the original filename while avoiding an unexplained nested directory hierarchy.

Example fleet result:

```text
output/rcgp--update.sh
output/yorkmusic--update.sh
```

- Refuse collisions and existing files by default. `--overwrite` permits atomic replacement of an existing local regular file but never a symlink.
- Stream each remote file to a same-directory local temporary file, verify the transmitted checksum, set a conservative local mode, then rename atomically.
- Do not write remote file contents to stdout. Text output prints stable local paths; JSON/YAML report forum, remote path, local path, checksum, size, and status.

## Phases

### Phase 0 - transport and identity foundations

- [x] Centralise SSH process construction and timeout/liveness options so every SSH caller follows one reviewed policy. `update` and `config check` now use `commands::ssh`.
- [ ] Consolidate SSH diagnostics behind the shared transport without buffering transferred bytes.
- [ ] Add bounded binary stdin/stdout streaming suitable for transfers, with diagnostics that never include transferred bytes.
- [ ] Define host identity policy before deployment. The current `accept-new` default is convenient for existing maintenance commands but is insufficient evidence for a security-sensitive file deployment; Phase 1 requires a pre-existing known-host key or an explicit, documented override.
- [ ] Establish isolated SSH/process fixtures that can emulate regular files, symlinks, permissions, interrupted streams, checksums, and `sudo -n` failures without a live host.
- [ ] Define a remote no-follow replacement protocol. It must protect both the final path and parent-directory traversal, avoid check-then-use races, stage on the destination filesystem when possible, and clean up an interrupted staged file.

### Phase 1 - safe audit and single-host push

- [ ] Add `file audit` for one forum and `file push` for one forum, after Phase 0 is complete.
- [ ] Reuse the Phase 0 SSH transport, shell quoting, dry-run, and structured-output conventions.
- [ ] Implement checksum comparison, same-directory staging, symlink refusal, optional non-interactive sudo, backup, metadata handling, atomic replacement, and post-write verification.
- [ ] Require a timestamped backup whenever replacing an existing destination in Phase 1. A later phase may add an explicit opt-out only after field use demonstrates it is safe and useful.
- [ ] Cover missing, matching, differing, permission-denied, symlink, interrupted-transfer, checksum-mismatch, and failed-replacement cases with isolated SSH/process fixtures.

### Phase 2 - fleet push and audit

- [ ] Add `all`, `--tags`, and bounded `--parallel` through the shared R48 selector and fleet executor.
- [ ] Require a complete dry-run target plan and `--yes` for fleet push.
- [ ] Return structured per-forum outcomes, continue after individual failures, and exit non-zero on partial failure.
- [ ] Verify the real driver by auditing, piloting, and then deploying the fallback updater across authorised managed hosts, retaining redacted output and checksums but no host credentials.

### Phase 3 - pull

- [ ] Add single-forum pull with atomic local writes and checksum verification.
- [ ] Add fleet pull using flat `<discourse>--<remote-basename>` filenames.
- [ ] Add collision, overwrite, local symlink, partial-failure, and filename-sanitisation tests.

## Backward compatibility

This is a new command surface and changes no existing command. It must reuse the configured `ssh_host` and established SSH option handling. The positional `all` selector is proposed before v1.0, but its final spelling must follow R48's fleet-selector decision so `dsc file` does not create a new compatibility exception.

## Security and safety

- The feature operates only against hosts explicitly selected from `dsc.toml`; it does not accept an ad hoc hostname.
- File transfers require established host-key verification by default. A first-use or changed-key override, if retained, must be explicit in the command invocation and visible in dry-run output.
- Fleet dry-run must enumerate resolved targets before mutation.
- Remote and local paths are data, not interpolated shell fragments. Quote every generated command argument with the existing single-pass shell-quoting helper.
- Never print transferred contents. Checksums, sizes, paths, metadata, and status are sufficient operational evidence.
- Refuse symlink endpoints and non-regular files on both sides.
- Use `sudo -n` only when the operator explicitly requests `--sudo`; never open a password prompt.
- Pulled files receive a conservative local mode so pulling sensitive configuration does not make it broadly readable.
- Document that `file push all` is appropriate only when the same artefact genuinely belongs on every selected host. Per-forum configuration differences remain the operator's responsibility.

## Out of scope

- Arbitrary remote command execution, hooks, or post-deployment commands.
- Recursive directory synchronisation, deletion, pruning, glob expansion, or rsync semantics.
- Editing or templating file contents. `dsc render` remains the explicit templating surface.
- Secret management, encryption, key distribution, or printing remote file contents.
- Automatically rebuilding Discourse or restarting services after a transfer.
- Replacing Ansible or another declarative configuration-management system.

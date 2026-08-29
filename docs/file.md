# dsc file

Checksum-driven regular-file transfer over SSH to the hosts already configured in `dsc.toml`. One file per invocation, no recursion, no remote command execution - a narrow operational primitive for deploying things like the fallback update script, not an rsync or Ansible replacement.

Every target must be a configured forum with an `ssh_host`. The shared SSH connection policy applies (`DSC_SSH_STRICT_HOST_KEY_CHECKING`, `DSC_SSH_OPTIONS`).

## dsc file audit

```text
dsc file audit <DISCOURSE|all> <LOCAL_PATH> <REMOTE_PATH> [--tags <TAGS>] [-p] [-m <N>] [--format text|json|yaml]
```

Compares the local file's SHA-256 and size with the remote file on each selected host without transferring remote contents. Reports `same`, `different`, `missing`, `symlink`, `present`, or a per-forum failure. Missing/different is a successful audit result, not an error; transport failures exit non-zero after all forums are reported.

```bash
dsc file audit myforum scripts/update.sh /var/discourse/scripts/update.sh
dsc file audit all scripts/update.sh /var/discourse/scripts/update.sh --tags production -p -f json
```

## dsc file push

```text
dsc file push <DISCOURSE|all> <LOCAL_PATH> <REMOTE_PATH> [--tags <TAGS>] [-p] [-m <N>] [--owner <OWNER>] [--group <GROUP>] [--mode <MODE>] [--no-backup] [--sudo] [--yes]
```

Uploads the local file with a checksum-verified, atomically renamed replacement:

- A destination whose checksum already matches is an idempotent no-op.
- Bytes are staged into a same-directory temp file, checksum-verified **before** any change, then renamed over the destination in one shell invocation. A corrupted transfer aborts with the destination untouched.
- A symlink destination is refused and never followed, even with `--yes`.
- A timestamped backup of an existing destination is taken by default; `--no-backup` skips it.
- `--owner`/`--group`/`--mode` apply to the staged file only. Ownership changes generally need `--sudo` (non-interactive `sudo -n`; no password prompt).
- A fleet push (`all`, with optional `--tags`) always requires `--yes`; run `--dry-run` first for the full per-forum plan including each forum's current remote state.
- Serial by default; `-p` uploads concurrently (default 3 workers, `-m` overrides, ceiling 32).

```bash
dsc -n file push all scripts/update.sh /var/discourse/scripts/update.sh --owner root --group root --mode 0755 --sudo
dsc file push all scripts/update.sh /var/discourse/scripts/update.sh --owner root --group root --mode 0755 --sudo --yes
dsc file push myforum scripts/update.sh /var/discourse/scripts/update.sh
```

Only use `file push all` when the same artefact genuinely belongs on every selected host; per-forum differences remain the operator's responsibility.
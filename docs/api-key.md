# dsc api-key

Manage Discourse API keys (admin scope).

There's a chicken-and-egg here: you need an existing admin key in your `dsc.toml` to use any of these commands. Bootstrapping the very first key still has to happen via the Admin UI.

## dsc api-key list

```text
dsc api-key list <discourse> [--format text|json|yaml]
```

Lists all keys with their id, description, the user they act as (or `(all-users)` for global keys), the last-used timestamp, and whether they're active or revoked.

## dsc api-key create

```text
dsc api-key create <discourse> <description> [--username <user>] [--format text|json|yaml]
```

Creates a new key. **The secret value is only displayed at creation time** — capture it from the output and store it somewhere safe (your `dsc.toml`, a password manager). Re-fetching the key later is impossible by design.

`--username` makes the key act as that specific user; omit for a global all-users key. Honours `--dry-run` (prints the intended action without creating the key).

```bash
dsc api-key create myforum "ops-runbook bot" --username system
dsc api-key create myforum "alice's read-only key" -u alice -f json
```

Scoped keys (e.g. write-only-to-topics) are not yet supported — those need to be created via the Admin UI for now.

## dsc api-key revoke

```text
dsc api-key revoke <discourse> <key-id>
```

Revokes the key by ID (soft: sets `revoked_at`, the key stops authenticating but the record is kept). Use `dsc api-key list` to find the ID. Honours `--dry-run`. Reversible with `dsc api-key undo-revoke`.

## dsc api-key undo-revoke

```text
dsc api-key undo-revoke <discourse> <key-id>
```

Undoes a previous `revoke`, reactivating the key. Alias: `unrevoke`. Honours `--dry-run`.

## dsc api-key delete

```text
dsc api-key delete <discourse> <key-id>
```

**Permanently** removes the key record via `DELETE /admin/api/keys/:id.json` — unlike `revoke`, this cannot be undone. Alias: `rm`. Honours `--dry-run`.

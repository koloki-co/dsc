# `dsc list rm` - remove a configured Discourse

Roadmap: R79. Status: implemented on main. Goal: safely remove a retired or mistakenly added forum from the active local configuration without contacting it.

## Motivation

Forum operations are named through `[[discourse]]` entries in `dsc.toml`. Operators need to remove an obsolete entry, including its locally stored API credential, after a migration, retirement, or accidental addition. Editing TOML directly is error-prone when configuration is selected through environment variables or a symlinked dotfiles repository.

## CLI surface

```text
dsc list rm <discourse> [--yes]
```

- Resolves the active configuration through the ordinary `dsc` selection rules.
- Selects one exact configured name only; missing names fail without writing.
- `--dry-run` prints the selected name, base URL, and configuration path without modifying local or remote state.
- A live removal requires `--yes` after dry-run review, removes precisely one `[[discourse]]` entry, and writes through the existing atomic, symlink-preserving configuration save path.
- The command never contacts Discourse, deletes remote data, or accepts `--all`/tag selection.

## Backward compatibility

Additive. Existing configuration discovery and every remote command remain unchanged.

## Out of scope

- Deleting a remote Discourse installation or its data.
- Removing local snapshots, update logs, backups, or operator-managed files associated with the forum.
- Bulk removal. Explicit one-at-a-time removal keeps locally retained credentials reviewable.

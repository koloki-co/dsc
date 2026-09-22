# Command target selection

Cross-cutting specification for how commands select configured Discourse installations. Goal: make fleet support deliberate and discoverable instead of adding `all` inconsistently when an operator first needs it. Driver: `dsc version --all` was a natural operator attempt on 2026-09-22, but the command supported only one forum and clap suggested `-- --all`, which treated `--all` as a forum name.

## Selector standard

New read-only fleet surfaces use an optional positional forum plus mutually exclusive `--all` and `--tags <tag1,tag2,...>` selectors:

```text
dsc <command> <forum>
dsc <command> --all
dsc <command> --tags production,managed
```

`--tags` implies fleet selection and matches any tag through the shared `selected_discourses` helper. An explicitly empty tag filter is an error. Fleet commands continue across per-forum failures, retain one result row per selected forum, preserve configuration order in structured output, and return non-zero after rendering if any forum failed. Read-only remote work uses the bounded shared fleet executor.

Existing positional `all`, implicit-all commands, and command-specific meanings of `--all` remain compatibility exceptions until a concrete migration is justified. A nested resource flag such as `notification read --all` means all matching resources on one forum, not all configured forums.

## Capability matrix

The matrix classifies command families by useful targeting semantics. It is a design check, not a promise to add fleet mode everywhere immediately.

| Command family | Current selection | Appropriate target model | Follow-up |
|---|---|---|---|
| `list` | implicit fleet, `--tags` | fleet | complete |
| `config check`, `doctor` | implicit fleet | fleet, optionally scoped | consider tags when demanded |
| `open` | forum/glob, `--all`, `--tags` | forum or fleet | complete |
| `version` | forum, `--all`, `--tags` | local tool, forum, or fleet | R63 |
| `update` | forum or positional `all` | forum or guarded fleet | retain compatibility; tags are a useful gap |
| `search` | forum or positional `all`, optional `--tags` | forum or fleet | retain compatibility |
| `user find`, `setting audit`, `app env audit` | implicit fleet, optional `--tags` | fleet comparison | complete |
| `backup create`, `backup setup-s3` | forum, `--all`, `--tags` | forum or guarded fleet | complete |
| `backup health` | optional forum, omission means fleet, `--tags` | forum or fleet | retain compatibility |
| `file audit|push|pull` | forum or positional `all`, optional `--tags` | forum or guarded fleet | retain compatibility |
| `setting get|list|pull`, `app env list|get`, `analytics`, `report`, `log staff` | forum only | forum or fleet read | candidates when a concrete fleet workflow arises; R20 covers reports |
| `setting set|push`, `app env set|unset`, plugin and portable taxonomy/theme mutations | forum only or command-specific tags | guarded selected fleet | require complete dry-run and per-forum verification before expansion |
| Resource inventory commands such as `category list`, `group list`, `emoji list`, `tag list`, `plugin list`, `theme list`, `api-key list`, `webhook list` | forum only | forum or fleet read | candidates when comparison output is defined |
| `sar`, `render`, `upload`, `backup pull|push` | forum only | forum only unless output/destination semantics are designed | no generic `--all` |
| Pairwise operations such as category/group copy, category/setting diff | explicit source/target pair | pairwise | no `--all` |
| Topic, post, invite, PM, notification, and user-account mutations | forum only | forum-local | IDs, identities, permissions, and effects are forum-specific |

## Review rule

For every new command or subcommand, decide explicitly whether it is local, forum-local, pairwise, fleet-readable, or a guarded fleet mutation. If fleet selection is appropriate, use the standard selector and bounded executor unless backward compatibility requires an exception. If it is not appropriate, state why in the command spec when the omission would otherwise be surprising.

## R63 behavior

`dsc version` keeps three distinct modes:

- No selector reports the local `dsc` version without resolving configuration.
- A positional forum reports the existing single-forum text line or structured object.
- `--all` or `--tags` reports an ordered fleet result. JSON and YAML are arrays; text emits one line per forum. A failed forum has an `error` instead of `version` and `commit`, does not suppress successful rows, and makes the command exit non-zero after all rows are rendered.

The live version comes primarily from `/about.json`; the running commit comes from the homepage generator metadata, with fallback parsing between the responses. Fleet lookup therefore uses bounded concurrency rather than serial requests.

## Backward compatibility

The new flags are additive. Bare and single-forum `dsc version` output remains unchanged. Existing selector exceptions are documented rather than changed by R63.

## Out of scope

- Adding fleet support to every candidate in the matrix.
- Replacing compatibility-preserving positional `all` selectors.
- Aggregating versions into release-policy decisions or automatically updating forums.

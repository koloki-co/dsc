# dsc upcoming-change

List and inspect Discourse's hidden Upcoming Changes - features gated behind
`/admin/config/upcoming-changes.json` rather than the ordinary site-settings
catalogue, so `dsc setting get` cannot see them. This phase is read-only;
explicit `enable`/`disable` and `dsc setting upload` are a later phase. See
[the implementation spec](https://github.com/koloki-co/dsc/blob/main/spec/commands/upcoming-changes-and-setting-upload.md)
for the full design and the driver (core Discourse's `enable_generated_llms_txt`).

The visible alias `upcoming` works everywhere `upcoming-change` does.

## List every Upcoming Change

```text
dsc upcoming-change list <discourse> [--format text|json|yaml]
```

One row per Upcoming Change the forum exposes, in the server's own stable
setting-name order: the setting name, its current effective value, and (when
present) the change's `status` and `enabled_for` scope.

```bash
dsc upcoming-change list myforum
dsc upcoming-change list myforum --format json
```

A 404 usually means the endpoint is unavailable on this Discourse version.

## Show one Upcoming Change

```text
dsc upcoming-change show <discourse> <setting-name> [--format text|json|yaml]
```

Selects one entry by its exact setting name (there is no single-item
endpoint, so this performs the same list request as `list` and filters
client-side) and prints its full detail: humanized name, description,
value, status, impact, `enabled_for` scope, dependency state
(`depends_on_met`), and whether it overrides separate site-setting defaults
(`overriding_defaults`).

```bash
dsc upcoming-change show myforum enable_generated_llms_txt
dsc upcoming-change show myforum enable_generated_llms_txt --format yaml
```

Fails clearly if the name is absent - it may not exist on this Discourse
version, may already be a promoted ordinary setting, or the name may be
misspelled; run `list` to see what is available.

## Notes

- Auth is the standard configured `apikey`/`api_username`; the acting user
  needs administrator access.
- Both commands are read-only and permit global `--dry-run`.
- `enable`/`disable` and `dsc setting upload` are planned but not yet
  implemented - see the linked spec for phasing.

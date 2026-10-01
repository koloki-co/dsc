# dsc upcoming-change

List, inspect, enable and disable Discourse's hidden Upcoming Changes - features gated behind
`/admin/config/upcoming-changes.json` rather than the ordinary site-settings
catalogue, so `dsc setting get` cannot see them. `dsc setting upload` is a later
phase. See
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

## Enable or disable one Upcoming Change

```text
dsc upcoming-change enable  <discourse> <setting-name> [--format text|json|yaml]
dsc upcoming-change disable <discourse> <setting-name> [--format text|json|yaml]
```

Sends the explicit target state to `PUT /admin/config/upcoming-changes/toggle.json`
(there is deliberately no relative `toggle` verb). The command:

- fails if the name does not exist;
- refuses to enable when `depends_on_met` is false (dependencies are never
  enabled automatically);
- makes no request and prints `no change` if the forum is already in the
  target state;
- under global `--dry-run`, reads current state and prints the plan without
  sending the PUT;
- re-fetches afterwards and exits non-zero if the forum does not report the
  requested value.

Existing group scope (`enabled_for`) is preserved by the server and reported
in the output, along with the previous value for rollback.

```bash
dsc --dry-run upcoming-change enable myforum enable_generated_llms_txt
dsc upcoming-change enable myforum enable_generated_llms_txt
```

Toggling creates durable audit records and affects live behaviour; try it on
a canary forum first. Fleet mutation is intentionally not offered.

## Notes

- Auth is the standard configured `apikey`/`api_username`; the acting user
  needs administrator access.
- `list` and `show` are read-only; `enable`/`disable` honour global
  `--dry-run`.
- `dsc setting upload` is planned but not yet implemented - see the linked
  spec for phasing.

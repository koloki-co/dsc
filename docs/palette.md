# dsc theme palette

List, pull, and push colour palettes (color schemes).

> **Renamed.** These commands now live under `dsc theme palette`. The old
> top-level `dsc palette …` form still works as a deprecated alias (it prints
> a one-line notice) and will be removed in a future release. Substitute
> `dsc theme palette` for `dsc palette` in the examples below.

## dsc theme palette list

```
dsc theme palette list <discourse> [--format text|json|yaml] [--verbose]
```

Lists available colour palettes on the specified Discourse, including built-in palettes with negative IDs. `-v`/`--verbose` includes additional fields where supported.

## dsc theme palette pull

```
dsc theme palette pull <discourse> <palette-id> [<local-path>]
```

Exports the specified palette to a local JSON file. If `<local-path>` is omitted, writes `palette-<id>.json` in the current directory. Discourse returns effective resolved colours rather than identifying which values are stored overrides, so the file is an editable effective-colour snapshot. The `pulled` block records the original name and effective colours used to identify intentional edits; do not edit it. Built-in negative-ID palettes can be pulled but not updated.

## dsc theme palette push

```
dsc theme palette push <discourse> <local-path> [<palette-id>]
```

Updates the specified custom palette with the colours in the local file. Before updating, `dsc` re-fetches the palette, rejects conflicting remote edits, and sends only values changed from the recorded `pulled` baseline that are not already effective remotely. It then refreshes both the effective snapshot and baseline from Discourse, so repeated pushes do not materialise recalculated inherited colours as overrides. Omitting a colour leaves it unchanged because Discourse has no API operation for deleting an existing colour override. Existing-palette updates from legacy files without a `pulled` block are refused; re-pull before editing. A baseline-free file can still create a new palette when neither the file nor command supplies an ID; the ID is persisted immediately and the refreshed baseline follows.

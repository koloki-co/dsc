# dsc theme

Install (from git or a local bundle), delete, list, pull, push, and duplicate themes; read/write a theme's settings, fields (SCSS/HTML), and upload assets; enable/disable and attach/detach components; and update git-backed remote components.

## dsc theme list

```
dsc theme list <discourse> [--format text|json|yaml] [--verbose]
```

Lists installed themes on the specified Discourse. `-v`/`--verbose` includes additional fields where supported.

## dsc theme install

```
dsc theme install <discourse> <source> [--branch <branch>]
```

Installs a theme/component via the admin import API (`POST /admin/themes/import.json`). `<source>` is either:

- a **git repo URL** - imported by the server. Private repos work by embedding credentials in the URL (`https://user:TOKEN@github.com/org/repo.git`); the token is redacted in `--dry-run` output. `--branch`/`-b` selects a branch (defaults to the repo's default).
- a **local bundle file** - a `.tar.gz`/zip theme export, uploaded and imported.

```bash
dsc theme install myforum https://github.com/org/theme
dsc theme install myforum https://user:TOKEN@github.com/org/private-theme.git -b main
dsc theme install myforum ./my-theme.tar.gz
```

## dsc theme delete

```
dsc theme delete <discourse> <theme-id>
```

Deletes a theme/component by id via the API (`DELETE /admin/themes/:id.json`). Refuses to delete the site default theme (reassign the default first). Honours `-n` / `--dry-run`. Prefer this over `theme remove` - it needs no SSH setup.

## dsc theme remove

```
dsc theme remove <discourse> <name>
```

Legacy SSH removal by name, using the command template in `DSC_SSH_THEME_REMOVE_CMD` (supports `{name}` and `{url}` placeholders). Prefer `dsc theme delete <id>`, which uses the API and needs no SSH template.

## dsc theme pull

```
dsc theme pull <discourse> <theme-id> [<local-path>]
```

Pulls the specified theme into a local JSON file. `<theme-id>` can be found using `dsc theme list`.

If `<local-path>` is omitted, the file is written to the current directory named from the theme name (slugified). The path to the written file is printed to stdout.

## dsc theme push

```
dsc theme push <discourse> <json-path> [<theme-id>] [--dry-run] [--yes]
```

Pushes a local JSON theme file to a Discourse instance.

- If `<theme-id>` is supplied, updates that existing theme. Updating a live theme is destructive (it upserts the theme's fields), so it requires `--yes`; review the plan with `--dry-run` (`-n`) first. Prints the ID on success.
- If the JSON file contains an `id` field, `dsc` refuses to guess: pass that ID explicitly as `<theme-id>` to update, or remove the `id` to create a new theme. This prevents a file pulled from one forum from silently overwriting a same-numbered theme on another.
- Otherwise (no `<theme-id>`, no `id` in the file) creates a new theme and prints the new ID.

## dsc theme duplicate

```
dsc theme duplicate <discourse> <theme-id> [--format text|json|yaml]
```

Duplicates the specified theme and prints the new theme ID. The copy is named `Copy of <original name>` and is not set as the default theme. `--format json` emits `{"id": ...}`.

## dsc theme show

```
dsc theme show <discourse> <theme-id> [--format text|json|yaml]
```

Shows a richer view of one theme/component than `list`: whether it's a theme or component, its enabled/default/user-selectable flags, colour scheme, parent themes, attached child components, settings count, and the editable field inventory (e.g. `common/scss`). Read-only.

```bash
dsc theme show accm 11
dsc theme show accm 11 --format json
```

## dsc theme setting

Read and write a single theme or component's **settings** - the typed key/value pairs a component declares (e.g. a nav component's menu items). Distinct from `dsc setting`, which manages site-wide settings.

```
dsc theme setting list <discourse> <theme-id> [--format text|json|yaml]
dsc theme setting get  <discourse> <theme-id> <key> [--format text|json|yaml]
dsc theme setting set  <discourse> <theme-id> <key> <value>   [--dry-run]
```

- `list` / `get` read from the theme JSON (`GET /admin/themes/:id.json`). `list` prints `key = value` per line in text mode; `json`/`yaml` include each setting's `type` and `default`.
- `set` writes via `PUT /admin/themes/:id/setting.json`. The `<value>` is sent **verbatim** - for a JSON-schema list setting (e.g. a header-links component), pass the JSON array text directly, quoted for your shell. Honours global `-n` / `--dry-run`.

```bash
dsc theme setting get accm 11 links_position
dsc theme setting set accm 14 links_position left
dsc theme setting set accm 14 header_links '[{"id":1,"title":"Education","url":"https://..."}]'
```

### dsc theme setting pull / push

For a whole component - and especially for the big JSON-list settings like a nav component's `header_links` / `dropdown_links` - snapshot the settings to a file, edit, and push back. The same pull → edit → push loop `dsc setting pull/push` gives site settings.

```
dsc theme setting pull <discourse> <theme-id> [<local-path>]
dsc theme setting push <discourse> <theme-id> <local-path>   [--dry-run]
```

- `pull` writes every setting to a file (YAML by default; a `.json` path writes JSON). **JSON-list settings are expanded to real arrays** - `header_links` becomes an editable list of entries, not one escaped string on a line. If `<local-path>` is omitted it writes `<theme-name>-settings.yml` in the current directory.
- `push` re-serialises each list back to the JSON-string form Discourse expects and **PUTs only the settings that changed** (compared semantically, so reformatting and key-order don't count as edits - an untouched pull → push is a clean no-op). Settings in the file that no longer exist on the component are skipped with a warning. `--dry-run` prints the plan, summarising long list values by length so the terminal isn't flooded.

This replaces the read-whole-array → change-one-field → PUT-it-back scripting that editing a header menu by hand otherwise needs.

```bash
# Snapshot the Dropdown Header component (id 17), edit, preview, apply
dsc theme setting pull accm 17 header.yml
$EDITOR header.yml                                  # rename "Conference 2026" -> "2027" in the list
dsc theme setting push accm 17 header.yml --dry-run # header_links: changed (864 -> 868 chars)
dsc theme setting push accm 17 header.yml
```

## dsc theme field

Read and write a theme's individual **fields** - the raw source entries a theme stores: `common/scss`, `desktop/scss`, `mobile/scss`, `common/head_tag`, etc. This is the pull -> edit -> push loop for a theme's SCSS/HTML, without a whole-theme JSON round-trip.

```
dsc theme field list <discourse> <theme-id> [--format text|json|yaml]
dsc theme field pull <discourse> <theme-id> <target/name> [<local-path>]
dsc theme field push <discourse> <theme-id> <target/name> <local-path>   [--dry-run]
```

- `list` shows each field as `target/name` with its type (`scss`/`html`/`js`/`yaml`/`upload`) and size.
- `pull` writes one field's body to a file (default name derived from the field, e.g. `common-scss.scss`). Upload-var fields have no text body - use `dsc theme asset` for those.
- `push` PUTs just that one field back (a single-entry `theme_fields` upsert; other fields are untouched). Unchanged content is a no-op; `--dry-run` shows the byte delta.
- **`push` refuses a git-backed remote component** - its fields are owned by the upstream repo, not the site. Edit upstream and `dsc theme update`, or `dsc theme duplicate` it first for an editable copy.

```bash
dsc theme field list accm 11
dsc theme field pull accm 11 common/scss common.scss
$EDITOR common.scss
dsc theme field push accm 11 common/scss common.scss --dry-run
dsc theme field push accm 11 common/scss common.scss
```

## dsc theme asset

Upload an image or font and bind it to a theme upload variable (`$name`) in one step, so the theme's SCSS/settings can reference it.

```
dsc theme asset list  <discourse> <theme-id> [--format text|json|yaml]
dsc theme asset set   <discourse> <theme-id> <name> <file>   [--dry-run]
dsc theme asset unset <discourse> <theme-id> <name>          [--dry-run]
```

- `set` uploads `<file>`, then binds it as a `theme_upload_var` field named `<name>` on the `common` target - referenceable as `$name` in the theme's SCSS.
- `list` shows the theme's bound upload assets (name, filename, URL).
- `unset` removes a binding (deletes the `theme_upload_var` field).
- Site-wide header logos (`logo`, `logo_small`, `mobile_logo`) are **site settings**, not theme assets - set those with `dsc setting set` + `dsc upload`. `theme asset` is for theme-scoped `$var` images/fonts.

```bash
dsc theme asset set accm 11 centred_logo ./logo.png
dsc theme asset list accm 11
dsc theme asset unset accm 11 centred_logo
```

## dsc theme enable / disable

```
dsc theme enable  <discourse> <theme-id>
dsc theme disable <discourse> <theme-id>
```

Toggles a theme or component's enabled state (`PUT /admin/themes/:id.json`). A disabled component stays attached to its parents (disabled is not the same as detached). Honours `-n` / `--dry-run`.

## dsc theme attach / detach

```
dsc theme attach <discourse> <parent-id> <component-id>
dsc theme detach <discourse> <parent-id> <component-id>
```

Attaches or detaches a component to/from a parent theme - this is what makes a component active on a given theme (and what `dsc theme install` leaves to you). `dsc` reads the parent's current children, adds/removes the component, and writes the full replacement set, so other attached components are preserved. Detaching an id that isn't attached (or attaching one that already is) is a reported no-op. Honours `-n` / `--dry-run`.

```bash
# Make component 14 active on the default theme (id 2)
dsc theme attach accm 2 14
# Retire it again
dsc theme detach accm 2 14
```

## dsc theme update

Pull a git-backed **remote** component to its latest upstream commit - the CLI equivalent of the Admin UI's "check for updates" / "update" on a remote theme. Distinct from `dsc update`, which rebuilds the OS/Discourse host.

```
dsc theme update <discourse> <theme-id> [--check]
```

- `--check` (and `--dry-run`) reports how many commits behind upstream the component is, without pulling.
- Without `--check`, it pulls the latest commit and reports the old -> new short hash (or "already up to date").
- Only git-backed remote components can be updated; a locally-authored theme reports a clear error (it has no upstream).

```bash
dsc theme update accm 17 --check   # e.g. "3 commits behind https://github.com/…"
dsc theme update accm 17           # pull the latest commit
```

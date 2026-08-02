# `dsc theme` - management gaps spec

> **Status: Phases 1-3 implemented (setting pull/push in v0.10.25; field/asset/update unreleased).**
> All specced commands now ship: `theme setting list/get/set/pull/push`,
> `theme field list/pull/push`, `theme asset list/set`, `theme enable/disable`,
> `theme attach/detach`, `theme show`, and `theme update` (git-backed remote
> refresh). The `theme_fields` shape flagged as uncaptured was confirmed live
> (2026-07-01, against koloki-demo + ACCM): `{ target, name, type_id, value,
> upload_id? }`, `type_id` 0=html/1=scss/2=upload_var. `theme install` (git
> or local bundle, via the API), `theme delete <id>`, and `theme asset unset`
> also now ship - see the completeness note at the end. Only a cross-instance
> theme settings diff remains deferred.

Spec for extending `dsc theme` to cover component configuration, enable/disable, per-field editing, and asset binding. Goal: drive a Discourse theme/component setup end-to-end from the CLI, without dropping into the Admin UI. Motivated by the ACCM (kitchen.culinarymedicine.org) header customisation work, where configuring header-nav components and iterating on theme SCSS from `dsc` is currently impossible.

## Context

Discourse distinguishes:

- **Themes** - top-level, user-selectable, can be the site default.
- **Components** - themes with `component: true`, attached to one or more parent themes via the parent's `child_theme_ids`. A component only takes effect while attached to an enabled parent.

A theme/component carries:

- **settings** - typed key/value pairs declared in the component's `settings.yaml`, edited per-install. This is how nav components (Custom Header Links, Dropdown Header, Header Submenus) store their menu items. Exposed in the theme JSON as `settings`; written via `PUT /admin/themes/:id/setting.json` with `name` + `value`.
- **theme_fields** - the source assets: `common/scss`, `desktop/scss`, `mobile/scss`, `extra_js`, `migrations`, translations, plus `theme_upload_var` fields that bind an uploaded image to an SCSS variable (e.g. `$logo`). Read in full by the theme JSON; written via `PUT /admin/themes/:id.json` with a `theme_fields` array.

(Exact endpoints above are the Admin UI's current behaviour and should be reconfirmed against the running Discourse version during implementation - the theme admin API is not formally versioned.)

## Current state (as of 2026-06-09)

`dsc theme` has: `list`, `install`, `remove`, `pull`, `push`, `duplicate`.

- `pull`/`push` operate on the **whole-theme JSON**. Good for backup/migration, clumsy for iterating one SCSS field.
- For locally-authored themes (e.g. ACCM's `kitchen-customisations`, id 11) `pull` returns real `theme_fields` values. For git-backed remote components (e.g. Brand Header, Header Submenus) the field values come back empty, since their source lives in the upstream repo - so `push` is not the edit path for those.
- There is **no** way to: read or write a component's **settings**; enable/disable a theme or attach/detach a component; edit a single field; or bind an uploaded image as a theme asset.

Key gaps, in priority order below.

## Phase 1 - blocking (component config + enable/disable)

### `dsc theme setting`

Read and write a single theme/component's settings (distinct from `dsc setting`, which is site settings).

```
dsc theme setting list <discourse> <theme-id> [--format text|json|yaml]
dsc theme setting get  <discourse> <theme-id> <key>
dsc theme setting set  <discourse> <theme-id> <key> <value>   [--dry-run]
```

- `list`/`get` read from the theme JSON `settings` array (`GET /admin/themes/:id.json`).
- `set` -> `PUT /admin/themes/:id/setting.json` with `name=<key>`, `value=<value>`. Honour `--dry-run`/`-n` like `dsc setting set`.
- Many nav components encode lists as a single delimited string (e.g. Header Submenus uses `|`-separated rows). `set` writes the raw string as given; documenting the per-component encoding is the user's job, not `dsc`'s.

`dsc theme setting pull/push` is **now field-required** (bumped 2026-06-28) - promoted from "optional later" to **Phase 2** below, after repeatedly hand-editing JSON-list settings via the raw admin API. `set` alone forces a read-modify-write of the entire JSON array just to change one field.

### `dsc theme enable` / `disable` (and component attachment)

```
dsc theme enable   <discourse> <theme-id>
dsc theme disable  <discourse> <theme-id>
dsc theme attach   <discourse> <parent-id> <component-id>     [--dry-run]
dsc theme detach   <discourse> <parent-id> <component-id>     [--dry-run]
```

- `enable`/`disable` -> `PUT /admin/themes/:id.json` toggling the theme's enabled state.
- `attach`/`detach` -> `PUT /admin/themes/:parent-id.json` adding/removing `component-id` in the parent's `child_theme_ids`. This is what actually makes a component active on a given theme, and what `dsc theme install` currently leaves to the user.
- Confirm during implementation whether "retiring" a component is best modelled as disable, or detach-from-parent. The ACCM case (Header Submenus showing unwanted demo content) is satisfied by either.

## Phase 2 - iteration ergonomics

### `dsc theme setting pull/push` (field-required) — implemented (unreleased)

Snapshot a component's settings to a local file, edit, push back - the pull -> edit -> push pattern `dsc setting pull/push` already gives site settings.

```
dsc theme setting pull <discourse> <theme-id> [file]
dsc theme setting push <discourse> <theme-id> <file>   [--dry-run]
```

- `pull` writes the `settings` array to YAML/JSON, **pretty-printing the JSON-list values** (`header_links`, `dropdown_links`) as real arrays - editable by hand, not one escaped string.
- `push` re-serialises each list back to its JSON-string form and `PUT`s only the changed settings (`PUT /admin/themes/:id/setting.json` per key); `--dry-run` shows the diff.
- **Why field-required:** editing Dropdown Header's `header_links` / `dropdown_links` on kitchen.culinarymedicine.org has repeatedly meant a hand-written *read-whole-array -> change one field -> PUT it back* script via the raw admin API - the My Account dropdown build, the parent-URL fix, and the Conference / Agenda "2026 -> 2027" renames (2026-06-28). `pull -> edit -> push` collapses each to a one-file edit.

**Confirmed shape (2026-06-29, against Dropdown Header id 17 on ACCM, Discourse 2026.6.0):** the JSON-list settings report `"type": "string"` - same as plain string settings like `main_link_color` - and carry their value as a string of escaped JSON (`"[{\"id\": 1, ...}]"`). So `type` can't flag them; detection is "parse the string; if it yields an array/object, expand it." Push serialises the edited array back to compact JSON text (NOT the site-settings pipe-join, which would corrupt it) and compares **semantically** (parsed JSON, not raw strings) so the compact-vs-spaced and key-order differences between file and server don't register as spurious edits - an untouched `pull -> push` is a verified no-op. `--dry-run` summarises long list values by length rather than dumping the whole array.

### `dsc theme field` — implemented (unreleased)

Edit one `theme_field` without a whole-theme round-trip.

```
dsc theme field list <discourse> <theme-id>
dsc theme field pull <discourse> <theme-id> <target/name> [local-path]
dsc theme field push <discourse> <theme-id> <target/name> <local-path>   [--dry-run]
```

- `<target/name>` e.g. `common/scss`, `desktop/scss`, `mobile/scss`.
- `pull` writes the raw field body (e.g. the SCSS) to a file with a sensible default name; `push` PUTs just that field back via `PUT /admin/themes/:id.json` with a single-entry `theme_fields` array.
- Refuse (with a clear message) to push to a field on a git-backed remote component, where the DB value is not the source of truth.

### `dsc theme asset` — implemented (unreleased)

Upload an image and bind it to a theme upload variable in one step, so SCSS/settings can reference `$name`. Confirmed live: `/uploads.json` accepts `type=theme`, and binding a `theme_upload_var` (`type_id: 2`) via a single-entry `theme_fields` PUT works.

```
dsc theme asset set <discourse> <theme-id> <name> <file>   [--dry-run]
dsc theme asset list <discourse> <theme-id>
```

- Uploads `<file>` (reusing the existing `dsc upload` path), then writes a `theme_upload_var` `theme_field` named `<name>` bound to the resulting upload. Needed for ACCM's centred-logo image and brand imagery.
- Note: the site-wide header logos (`logo`, `logo_small`, `mobile_logo`) are **site settings**, already settable via `dsc setting set` + `dsc upload`; `theme asset` is specifically for theme-scoped `$var` assets.

## Phase 3 - nice to have (implemented)

- [x] **`dsc theme show <discourse> <theme-id>`** - richer than `list`: component flag, enabled state, default flag, parent(s), attached children, settings count, field inventory. `list` today shows only `id - name - enabled/disabled`.
- [x] **`dsc theme update <discourse> <theme-id>`** - pull an installed *remote* component to its latest upstream commit (distinct from `dsc update`, which is the OS/Discourse rebuild). Maps to the Admin UI "check for updates" on a remote theme. Endpoint confirmed from Discourse source: `PUT /admin/themes/:id.json` with `theme.remote_check` (refresh `commits_behind`) or `theme.remote_update` (pull). `--check` reports how far behind; the default pulls and reports old->new short hash.

## Completeness (2026-07-01)

`dsc theme` now covers the full ACCM driver and the day-to-day CRUD end to end: install (git URL incl. private-via-URL creds, or a local `.tar.gz`/zip bundle - `POST /admin/themes/import.json` with `remote`+`branch` or a `bundle` multipart), delete-by-id (`DELETE /admin/themes/:id.json`, refuses the site default), list/show/pull/push/duplicate, settings (incl. pull/push), fields (SCSS/HTML), upload assets (set + unset), enable/disable, attach/detach, palettes, and remote update. Palette `push --dry-run` reads and validates an existing target, then prints the exact payload, request sequence, and assigned-ID and refreshed local snapshot writes without writing. All verified live on koloki-demo (git + local install, delete, asset unset).

The old SSH `theme install`/`theme remove` remain as legacy paths but are superseded by the API `install`/`delete`. Only genuinely-deferred item left: a cross-instance theme settings diff (the `dsc setting diff` analogue), out of scope below.

## Reference: API calls observed in the field

These are the exact Discourse admin API calls used to do this work by hand on kitchen.culinarymedicine.org while `dsc` lacked the commands. Tested against **Discourse 2026.6.0-latest** (the new glimmer header). All requests carry `Api-Key: <redacted>` and `Api-Username: <admin>` headers. They are the ground truth the proposed subcommands should wrap.

**List themes (find default, components, relationships)** - backs `theme show` / a richer `theme list`:

```
GET /admin/themes.json
```

Response: `{ "themes": [ { "id", "name", "component": bool, "default": bool, "enabled": bool, "child_themes": [{id,name}], "parent_themes": [{id,name}] }, ... ] }`. Components attach to a parent via the parent's children, not a flag on the child.

**Read one theme: settings schema + fields** - backs `theme setting list/get`, `theme field`:

```
GET /admin/themes/:id.json
```

Response `theme.settings[]` entries look like `{ "setting": "links_position", "type": "enum", "default": "right", "value": "right", "choices": [...] }`. Note: JSON-schema list settings (e.g. Dropdown Header's `header_links`) report **`"type": "string"`** here - the `json_schema` lives in the component's `settings.yml`, not the API response. The stored `value` is the JSON array serialised as a string.

**Set a theme/component setting** - backs `theme setting set`:

```
PUT /admin/themes/:id/setting.json
Content-Type: application/x-www-form-urlencoded
name=links_position&value=left
```

For a JSON-schema string setting, `value` is the JSON text, urlencoded, e.g.:

```
name=header_links&value=[{"id":1,"title":"Education","icon":"","url":"https://...","newTab":true}]
```

Returns 200 on success; 422 with a validation message if the value violates the setting's `json_schema`. No response body needed.

**Import a component from a git repo** - backs `theme install` over the API (today's `dsc theme install` is SSH-only):

```
POST /admin/themes/import.json
Content-Type: application/x-www-form-urlencoded
remote=https://github.com/paviliondev/discourse-dropdown-header&branch=main
```

Response: `{ "theme": { "id": 14, "name": "Dropdown Header", "component": true, ... } }`. `branch` is required-ish; try the repo default (`main`, then `master`).

**Attach a component to a parent theme** - backs `theme attach/detach`:

```
PUT /admin/themes/:parent-id.json
Content-Type: application/json
{ "theme": { "child_theme_ids": [8,3,1,5,13,10,4,11,6,14] } }
```

The list **replaces** the parent's children, so read the current `child_themes` from `GET /admin/themes/:parent-id.json` first and send the full set plus the new id. Disabled components stay in the list (disabled != detached).

**Enable / disable a theme or component** - backs `theme enable/disable`:

```
PUT /admin/themes/:id.json
Content-Type: application/json
{ "theme": { "enabled": false } }
```

(Confirmed via the Admin UI toggle; the `enabled` boolean round-trips in `GET /admin/themes/:id.json`.)

## Out of scope

- Authoring component source (SCSS/JS) beyond editing existing fields - that belongs in the component's own repo, not `dsc`.
- A full theme settings-diff across instances (the `dsc setting diff` analogue for themes). Could follow if `theme setting pull/push` lands.

## ACCM driver (why now)

The kitchen.culinarymedicine.org header rework needs to, from the CLI: disable/retire the Header Submenus component, configure a header-nav component's menu items (its settings), iterate `kitchen-customisations` (id 11) Common SCSS for the centred-logo layout, and bind a centred-logo image asset. Phases 1-2 cover all of that; until they land the same actions are done via the Admin UI or direct API calls.

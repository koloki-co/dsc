# `dsc setting` - bulk pull/push spec

Spec for declarative site-settings management. Goal: snapshot an instance's site settings to a version-controlled file, diff across instances, and push changes through a staging→production workflow.

## Context

Discourse exposes ~800+ site settings via `GET /admin/site_settings.json`. Each entry includes:

```json
{
  "setting": "title",
  "value": "My Forum",
  "default": "Discourse",
  "description": "The name of this site, as used in the title tag.",
  "category": "required",
  "type": "string",
  "preview": null,
  "placeholder": null
}
```

The number of settings grows by ~5-10 per Discourse release. Settings are occasionally renamed or removed between versions.

## Current state (as of 2026-06-07)

`dsc setting` has three subcommands:

| Subcommand | Scope | Notes |
|---|---|---|
| `list` | single instance | Dumps all settings as `key = value` (text) or structured (json/yaml). Discards `default`, `description`, `type`. |
| `get` | single instance, single key | Fetches all settings then filters client-side (no per-setting endpoint). |
| `set` | single instance (or tag-filtered) | `PUT /admin/site_settings/{name}.json`. Multi-instance via `--tags` is implemented but unreachable from CLI. |

Key gaps: no file-based workflow, no cross-instance comparison, no metadata preservation, no bulk write.

## Implementation progress

- [x] **Phase 1** - `dsc setting pull` (read-only snapshot)
  - [x] `SiteSettingDetail` struct + `list_site_settings_detailed` API method
  - [x] CLI `Pull` subcommand wired through `main.rs`
  - [x] `pull_settings` command writes YAML/JSON snapshot
  - [x] `--changed-only` and `--category` filters
  - [x] Read-only skip-list applied
- [x] **Phase 2** - `dsc setting push` (write)
  - [x] CLI `Push` subcommand
  - [x] Idempotent diff + PUT only on change
  - [x] `--dry-run` plan output
  - [x] `--reset-unlisted` mode
  - [x] Skip unknown / read-only settings gracefully
- [x] **Phase 3** - `dsc setting diff`
  - [x] Live cross-instance diff (`dsc setting diff <a> <b>`)
  - [x] File-based diff (snapshot vs snapshot)
  - [x] `--changed-only` and `--category` filters
- [x] **Phase 4** - Fix `setting set --tags` CLI reachability

## New subcommands

### Phase 1: `dsc setting pull` (read-only snapshot)

```
dsc setting pull <discourse> [path] [--changed-only] [--category <cat>]
```

- Fetch all settings via `GET /admin/site_settings.json`.
- Serialize to a self-documenting YAML (default) or JSON file.
- Default path: `settings.yaml`.
- `--changed-only` (`-c`): only emit settings where `value != default`. This is the typical workflow file - manageable size (~50-100 entries instead of 800+).
- `--category <cat>`: filter to a single category (e.g. `required`, `email`, `security`).
- Sort by `category` then `name` for stable diffs.
- Exclude known read-only / computed settings (maintain a skip-list, e.g. `version`, `discourse_connect_url` when SSO is off).

#### File schema (version 1)

```yaml
version: 1
provenance:
  source: "https://forum.example.com"
  exporter_version: "0.24.0"
discourse_version: "2026.6.0"
pulled_at: "2026-06-07T14:30:00Z"

settings:
  # ── required ──────────────────
  - name: title
    value: "My Forum"
    default: "Discourse"
    type: string
    category: required
    description: "The name of this site, as used in the title tag."

  - name: site_description
    value: "A place to discuss things"
    default: ""
    type: string
    category: required
    description: "Describe this site in one sentence, used in meta description."

  # ── email ─────────────────────
  - name: notification_email
    value: "noreply@forum.example.com"
    default: "noreply@unconfigured.discourse.org"
    type: string
    category: email
    description: "The from: email address used when sending all essential system emails..."
```

Design notes:
- `default`, `type`, `description` are metadata pulled from the API. They make the file self-documenting and LLM-friendly. `push` ignores them (only `name` and `value` matter on write).
- `provenance`, `discourse_version`, and `pulled_at` are informational stamps - not enforced on push. `provenance.source` is credential-free and records the normalized source site; `provenance.exporter_version` records the dsc exporter version. Older v1 files may omit it.
- Category comment separators are cosmetic in YAML output for human scanning.

### Phase 2: `dsc setting push` (write)

```
dsc setting push <discourse> <path> [--reset-unlisted]
```

- Read the file, compare each `name`/`value` pair against the server's current value.
- **Only send PUTs where the value differs** (idempotent - no-op if file matches server).
- **Skip unknown settings**: if the file mentions a setting the server doesn't have, warn and skip (handles version drift gracefully).
- **Skip read-only settings** silently.
- `--reset-unlisted`: for settings present on the server but absent from the file, reset them to their `default` value. Off by default. This is the "full sync" mode.
- `--dry-run` (`-n`): print the diff/plan without applying.
- Settings with `value == default` in the file are still pushed (explicitly setting something to default is a valid intent).

#### Dry-run output format

```
[dry-run] Setting push plan for myforum:
  ~ title: "Discourse" → "My Forum"
  ~ notification_email: "noreply@unconfigured..." → "noreply@forum.example.com"
  = site_description: (unchanged)
  ? some_removed_setting: skipped (not found on server)
  - old_setting: would reset to default "true" (--reset-unlisted)
```

### Phase 3: `dsc setting diff` (cross-instance comparison)

```
dsc setting diff <discourse-a> <discourse-b> [--changed-only] [--category <cat>]
```

- Pull settings from both instances, compare value-by-value.
- Output a unified-diff-style view (or structured json/yaml with `--format`).
- `--changed-only`: only show settings where at least one instance differs from default.
- Useful for: "what's different between staging and production?"

Alternative file-based form:

```
dsc setting diff <file-a> <file-b>
```

Compare two previously-pulled snapshots without network access.

### Phase 4: Fix existing `set` multi-instance reachability

Make `discourse` optional in `dsc setting set` when `--tags` is provided:

```
dsc setting set --tags production title "New Title"
```

This is already implemented in the command layer but unreachable from the CLI because `discourse` is a required positional argument.

## API surface changes

### `src/api/settings.rs`

New method:

```rust
/// Fetch all site settings with full metadata.
/// Returns Vec<SiteSettingDetail> instead of raw JSON.
pub fn list_site_settings_detailed(&self) -> Result<Vec<SiteSettingDetail>>
```

Where:

```rust
pub struct SiteSettingDetail {
    pub setting: String,
    pub value: serde_json::Value,
    pub default: serde_json::Value,
    pub description: String,
    pub category: String,
    pub setting_type: String,    // "string", "integer", "bool", "enum", "list", etc.
}
```

Existing `list_site_settings()` (raw JSON) remains for backward compat with `setting list`.

### `src/commands/setting.rs`

New functions:

```rust
pub fn pull_settings(config, discourse_name, local_path, changed_only, category) -> Result<()>
pub fn push_settings(config, discourse_name, local_path, reset_unlisted, dry_run) -> Result<()>
pub fn diff_settings(config, source, target, changed_only, category, format) -> Result<()>
```

### `src/cli.rs`

Add to `SettingCommand`:

```rust
Pull { discourse, local_path, changed_only, category }
Push { discourse, local_path, reset_unlisted }
Diff { source, target, changed_only, category, format }
```

## Version drift handling

| Scenario | Behaviour |
|---|---|
| File has setting not on server | `push` warns and skips |
| Server has setting not in file | `push` leaves it alone (unless `--reset-unlisted`) |
| Setting renamed between versions | Old name skipped (warn), new name not in file (left alone). User edits file to use new name. |
| Setting type changed | `push` sends the string value; Discourse coerces. If coercion fails, the API returns 422 and `dsc` reports the error per-setting and continues. |
| `discourse_version` mismatch | Informational only - no enforcement |

## Value serialization

Settings values in the Discourse API can be strings, integers, booleans, or lists. In the YAML file:

- Strings: quoted YAML strings
- Integers/booleans: native YAML types
- Lists (pipe-separated in Discourse): stored as a YAML string matching Discourse's `"a|b|c"` format (round-trip safe; avoids lossy transformation)
- Empty values: empty string `""`

On push, all values are sent as strings to the `PUT` endpoint (Discourse coerces internally).

## Read-only settings skip-list

Some settings are computed or read-only. Maintain a list in code:

```rust
const READONLY_SETTINGS: &[&str] = &[
    "version",
    "discourse_version",
    // add more as discovered
];
```

On `pull`, these are excluded. On `push`, they are silently skipped. The list will be small - most settings are writable. Unknown read-only settings will produce a 422 from the API, which `push` handles gracefully (warn + continue).

## Phases and dependencies

| Phase | Deliverable | Depends on | Effort |
|---|---|---|---|
| 1 | `setting pull` + file schema + API metadata | None | Medium |
| 2 | `setting push` + dry-run + idempotent write | Phase 1 | Medium |
| 3 | `setting diff` (live + file-based) | Phase 1 | Small |
| 4 | Fix `setting set --tags` CLI reachability | None (independent) | Small |

## Edge cases / open questions

1. **Secrets in settings**: Some settings contain API keys, SMTP passwords, etc. The pull file will contain these in plaintext. The file should NOT be committed to a public repo. Consider a `--redact-secrets` flag that replaces known secret-type settings with a placeholder (or omits them). The `type` field from the API may help identify these (`type: "secret"`).

2. **Upload-type settings**: Settings like `logo` and `favicon` reference uploaded file paths. These are strings (URLs) and round-trip fine, but pushing a URL from instance A to instance B only works if the asset exists on B. Out of scope for this feature - document the limitation.

3. **Plugin settings**: Plugins add their own site settings. These appear in the API response with plugin-specific category names. `pull` captures them naturally; `push` skips unknown ones on a server without the plugin. No special handling needed.

4. **Rate limiting on push**: Bulk-setting hundreds of values will hit rate limits. The existing `send_retrying` wrapper handles 429s. For very large pushes with `--reset-unlisted`, consider batching or adding a progress bar.

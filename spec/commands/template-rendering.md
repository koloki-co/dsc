# `dsc render` - template placeholder rendering

Spec for filling placeholders in local Markdown template files using data from `dsc.toml`, so a finished post body is ready to push to a Discourse without manual find-and-replace. Goal: eliminate the per-forum manual substitution step in the `pull → edit → push` workflow when working from a shared content-template library. Driver: the `content-templates/` collection in the author's discourses workspace - 24 anonymised, reusable Markdown templates that use `FORUM_BASEURL`, `[COMMUNITY]`, `[ORGANISATION]`, and `[GROUP NAME]` placeholders, currently substituted by hand or ad-hoc `sed` before pushing into a client forum.

## Motivation

The author maintains a library of ~24 reusable forum content templates (onboarding guides, welcome posts, moderation canned replies, admin how-tos) that are anonymised - real forum names, URLs, and organisations are replaced with placeholders. Today, adapting a template for a specific client forum means a manual find-and-replace pass: `FORUM_BASEURL` → the forum's base URL, `[COMMUNITY]` → the community name, `[ORGANISATION]` → the org name, and so on. This is error-prone, tedious, and breaks the `dsc topic new` / `dsc category push` round-trip because the file on disk still contains placeholders until you edit it by hand.

`dsc.toml` already holds per-forum data that maps to most of these placeholders: `baseurl`, `fullname`, `name`. What is missing is (a) a way to declare additional per-forum template variables (organisation name, community name, group names) in `dsc.toml`, and (b) a rendering step that substitutes all of them into a template file, producing a ready-to-push Markdown body. With both in place, the workflow becomes: `dsc render myforum template.md | dsc topic new myforum 42 --title "..."` - no manual editing.

Discourse itself has no composer-level placeholder substitution (the `discourse-placeholder-theme-component` embeds input boxes in the post, which is a different use case). This feature fills that gap on the `dsc` side, where the config data already lives.

## Current state (as of 2026-07-17)

- `dsc topic new <discourse> <category> --title <title> <file>` reads a file and posts it as-is. No placeholder substitution.
- `dsc topic push <discourse> <topic-id> <file>` reads a file and pushes it as-is. No placeholder substitution.
- `dsc category push <discourse> <category> <dir>` reads files and pushes them as-is. No placeholder substitution.
- `dsc.toml` `[[discourse]]` blocks carry `name`, `fullname`, `baseurl`, `apikey`, `api_username`, `ssh_host`, `tags`, `changelog_topic_id`, `docker_rootless`. None of these are exposed as template variables.
- There is no `[template]` section in `dsc.toml` and no concept of per-forum custom variables.
- No template engine crate is in `Cargo.toml` dependencies.

## Proposed CLI surface

```text
dsc render <discourse> <file> [-o <output>] [--format text|json|yaml]
```

### `dsc render <discourse> <file>`

Reads a local Markdown (or text) file, substitutes all template placeholders using the named forum's variables, and writes the result to stdout (or to `-o <output>`).

- Reads `<file>` from disk. If `<file>` is `-`, reads from stdin.
- Loads template variables from three layers (later layers override earlier ones - see [Variable resolution](#variable-resolution) below):
  1. Built-in variables derived from the `[[discourse]]` block.
  2. `[template.vars]` global variables from `dsc.toml`.
  3. `[discourse.template]` per-forum variables from the matching `[[discourse]]` block.
- Substitutes all `{{ variable }}` occurrences in the file content using the resolved variable map.
- Writes the rendered text to stdout, or to the path given by `-o` / `--output`.
- `--format json` emits `{"rendered": "..."}` for scripting; `--format yaml` emits `rendered: |-\n  ...`. Default is `text` (raw rendered content to stdout).
- Honours `-n` / `--dry-run`: prints the resolved variable map to stderr and the rendered output to stdout, but does not write `-o` output. Useful for previewing what substitutions will be made.
- On unknown variable (a `{{ foo }}` with no `foo` in the variable map): prints a warning to stderr naming the variable, substitutes an empty string, and continues. This matches the "drop the placeholder" expectation rather than failing the whole render. A `--strict` flag (Phase 2) can make unknown variables a hard error.
- Does **not** touch Discourse's own `%{...}` placeholders (e.g. `%{reply_to_username,fallback:there}`, `%{my_name}`, `%{reply_key}`). These are Discourse server-side substitution tokens that pass through `dsc render` untouched. The `{{ }}` syntax is chosen specifically to avoid collision with `%{ }`.

### Integration flag: `--render` on push commands

Applies the same rendering as `dsc render` inline, so you do not need a separate step:

```text
dsc topic new     <discourse> <category> --title <title> <file> --render
dsc topic push    <discourse> <topic-id> <file> --render
dsc topic reply   <discourse> <topic-id> <file> --render
dsc category push <discourse> <category> <dir> --render
```

- `--render` is a boolean flag (no value). When present, the file(s) are rendered against the target forum's variables before being sent.
- On `category push --render`, every `.md` file in the directory is rendered individually before pushing.
- Without `--render`, all commands behave exactly as today (no substitution). This is the backward-compatible default.
- `--render` composes with `--dry-run`: the dry-run output shows the rendered content that would be sent.

## Variable resolution

Variables are resolved from three layers. Later layers override earlier ones, so per-forum values win over globals.

### Layer 1: built-in variables (from the `[[discourse]]` block)

These are derived automatically from the matched forum's config entry. No user configuration needed.

| Template variable | Source field | Example |
|---|---|---|
| `forum_baseurl` | `baseurl` | `https://forum.example.org` |
| `forum_name` | `name` | `myforum` |
| `forum_fullname` | `fullname` | `My Forum` |

`forum_baseurl` is the primary substitute for the `FORUM_BASEURL` placeholder used in the content-templates library. `forum_fullname` is the display title. `forum_name` is the short CLI name (rarely needed in post bodies but available).

### Layer 2: global template variables (`[template.vars]`)

A new top-level `dsc.toml` section for variables shared across all forums:

```toml
[template.vars]
organisation = "Koloki Ltd"
community = "OpenEHR International"
```

These apply to every forum unless overridden by a per-forum value. Useful for variables that are constant across your fleet (e.g. your company name in a footer or signature).

### Layer 3: per-forum template variables (`[discourse.template]`)

A new optional sub-table inside a `[[discourse]]` block, for forum-specific overrides and additions:

```toml
[[discourse]]
name = "openehr"
fullname = "openEHR International"
baseurl = "https://discourse.openehr.org"
apikey = "..."
api_username = "system"

[discourse.template]
organisation = "openEHR International"
community = "openEHR International"
support_email = "admin@openehr.org"
```

Variables here override globals of the same name and can introduce new ones. This is where most per-forum substitution data lives.

### Resolution example

Given the config above and a template file containing:

```markdown
Welcome to {{ community }}! Visit your preferences at {{ forum_baseurl }}/my/preferences/emails.
If you need help, email {{ support_email }}.
```

`dsc render openehr template.md` produces:

```markdown
Welcome to openEHR International! Visit your preferences at https://discourse.openehr.org/my/preferences/emails.
If you need help, email admin@openehr.org.
```

## Template syntax

The engine uses `{{ variable }}` interpolation - the Jinja2/Handlebars/Tera family of syntax that is widely recognised. The initial scope is simple variable substitution only:

| Syntax | Meaning | Phase |
|---|---|---|
| `{{ variable }}` | Substitute the variable's value | 1 |
| `{{ variable \| default("fallback") }}` | Substitute with a fallback if the variable is unset | 2 |
| `{% if condition %}...{% endif %}` | Conditional blocks | 3 |
| `{% for item in list %}...{% endfor %}` | Loops | 3 |

Phase 1 ships only `{{ variable }}`. The engine is chosen so that conditionals and loops are available later without a syntax migration (see [Engine choice](#engine-choice)).

### What is NOT substituted

- Discourse's own `%{...}` placeholders (e.g. `%{reply_to_username,fallback:there}`, `%{my_name}`, `%{reply_key}`) pass through untouched. These are server-side Discourse substitution tokens, not `dsc`'s concern.
- YAML front matter (the `---` block at the top of a file) is passed through as-is. `dsc render` operates on the full file content; `dsc topic push` already strips front matter before sending. Rendering happens before stripping, so front matter is available for future metadata uses but its values are not used as variables in Phase 1.
- Code blocks (```` ``` ````) are not protected in Phase 1. If a code block contains `{{ }}` that should not be substituted, use `--strict` and ensure the variable is defined, or wait for Phase 2 raw-block handling. In practice, Discourse template content rarely contains literal `{{ }}` in code blocks.

## Engine choice

Two mature, actively maintained Rust template engines fit the `{{ }}` syntax requirement:

| | [Tera](https://crates.io/crates/tera) | [Handlebars](https://crates.io/crates/handlebars) |
|---|---|---|
| Latest stable | 2.0.0 (2026-06-26) | 6.4.3 (2026-07-12) |
| Syntax family | Jinja2 / Django | Handlebars JS |
| Conditionals / loops | Yes (`{% if %}`, `{% for %}`) | Yes (`{{#if}}`, `{{#each}}`) |
| Filters / helpers | Rich built-in filter set | Helper system (extensible) |
| Logic philosophy | Full Jinja2 power | Logic-less (by design) |
| Download volume | ~1.7M/month | ~3.4M/month |
| SLoC | ~12K | ~9K |
| License | MIT | MIT |

**Recommendation: Tera.** The content-templates use case is simple variable substitution today, but the natural evolution (conditionals for forum-type-specific content, loops for repeating sections, filters for title-casing community names) maps directly to Tera's Jinja2 feature set. Tera's syntax (`{{ var }}`, `{% if %}`) is the most widely recognised template syntax outside of web-specific ecosystems. Handlebars' logic-less philosophy would impose constraints that matter for HTML views but not for Markdown prose templates. Tera 2.0 is a fresh release on a maintained codebase.

If the author prefers the logic-less constraint as a guardrail against template creep, Handlebars 6.4.3 is the fallback - the `{{ }}` interpolation syntax is identical for Phase 1, so no template migration would be needed if the choice is reversed later.

### Dependency footprint

Tera 2.0.0 pulls in `serde`, `serde_json` (both already in `dsc`'s dependency tree), and a handful of small utility crates. No heavy transitive dependencies. The `glob_fs` and `fast_hash` features are not needed and should be left off to keep the build lean.

## Config schema additions

### `[template.vars]` (top-level, optional)

A flat string-keyed map of global template variables. All values must be strings (or integers that stringify cleanly). No nested tables.

```toml
[template.vars]
organisation = "Koloki Ltd"
community = "Koloki Community"
```

### `[discourse.template]` (per-forum, optional)

A sub-table inside a `[[discourse]]` block. Same shape as `[template.vars]` - flat string-keyed map. Overrides globals of the same name and can introduce new variables.

```toml
[[discourse]]
name = "openehr"
baseurl = "https://discourse.openehr.org"
# ... other fields ...

[discourse.template]
organisation = "openEHR International"
community = "openEHR International"
support_email = "admin@openehr.org"
```

Both sections are optional. If neither is present, only the three built-in variables (`forum_baseurl`, `forum_name`, `forum_fullname`) are available for rendering.

## Migration path for existing content-templates

The content-templates library (in the discourses workspace, not in this repo) currently uses two placeholder styles:

| Current placeholder | New template variable |
|---|---|
| `FORUM_BASEURL` | `{{ forum_baseurl }}` |
| `[COMMUNITY]` | `{{ community }}` |
| `[ORGANISATION]` | `{{ organisation }}` |
| `[GROUP NAME]` | `{{ group_name }}` |

A one-time find-and-replace pass over the 24 template files converts them. The `README.md` in that directory documents the placeholder convention and would be updated to match. This migration is out of scope for `dsc` itself (it is content, not code) but is noted here for completeness. The `dsc render` feature is what makes the new `{{ }}` syntax useful.

## Phases

### Phase 1 - blocking

- [x] Add `tera` 2.0.0 to `Cargo.toml` dependencies (no optional features).
- [x] Implement `[template.vars]` and `[discourse.template]` parsing in the config loader.
- [x] Implement built-in variable derivation (`forum_baseurl`, `forum_name`, `forum_fullname`) from the `[[discourse]]` block.
- [x] Implement `dsc render <discourse> <file> [-o <output>] [--format text|json|yaml]` with `{{ variable }}` substitution only.
- [x] Unknown variable: warn to stderr, substitute empty string, continue.
- [x] `--dry-run`: print resolved variable map to stderr, rendered output to stdout, skip `-o` write.
- [x] End-to-end test: render a template file against a test config, verify output.
- [x] Update `dsc.example.toml` with commented-out `[template.vars]` and `[discourse.template]` examples.
- [x] Add `docs/render.md` with usage and examples.

### Phase 2 - iteration ergonomics

- [ ] `--render` flag on `dsc topic new`, `dsc topic push`, `dsc topic reply`, `dsc category push`.
- [x] `--strict` flag: unknown variables are a hard error (exit non-zero with a message naming every unknown variable).
- [x] `dsc render --list-vars <discourse>`: print the full resolved variable map for a forum (useful for debugging and for seeing what is available before writing a template).
- [ ] Raw block protection: content inside ```` ```raw ```` / ```` ``` ```` code fences is not substituted. (Alternatively, Tera's `{% raw %}...{% endraw %}` blocks, but those inject non-Markdown syntax into the file. A Markdown-aware code-fence skip is cleaner for this use case.)

### Phase 3 - nice to have

- [ ] Tera conditionals and loops (`{% if %}`, `{% for %}`) available in templates. No new `dsc` code needed beyond not disabling them, but documented and tested.
- [ ] Custom filters relevant to forum content: `{{ community \| titlecase }}`, `{{ forum_baseurl \| trim_end("/") }}`.
- [ ] `dsc render` accepts multiple files: `dsc render <discourse> <file> [<file>...]` renders each and writes to `<stem>.rendered.md` alongside the original (or to `-o <dir>`).
- [ ] Template validation: `dsc render --check <file>` parses the template and reports syntax errors and unknown variables without rendering.

## Backward compatibility

- No existing command behaviour changes. `--render` is opt-in; without it, `topic new` / `topic push` / `category push` send files as-is, exactly as today.
- `dsc.toml` additions (`[template.vars]`, `[discourse.template]`) are optional. Configs without them load and work exactly as before.
- `dsc render` is a new top-level command; it does not shadow or alias any existing command.
- The three built-in variable names (`forum_baseurl`, `forum_name`, `forum_fullname`) are reserved. If a user defines a `[template.vars]` or `[discourse.template]` key with the same name, the per-forum value wins (per the resolution order) but a warning is printed on `dsc render` and on `dsc config` output.

## Out of scope

- **Discourse-side placeholder substitution.** Discourse's `%{...}` tokens are the server's responsibility. `dsc` passes them through untouched.
- **Interactive variable prompting.** No "enter the value for COMMUNITY:" prompts. All variables come from `dsc.toml`. (A future `dsc render --interactive` could prompt for missing variables, but that is not Phase 1.)
- **Template libraries / registries.** `dsc` does not host, fetch, or index template collections. The content-templates directory is just files on disk.
- **HTML rendering.** `dsc render` operates on text (Markdown). No HTML escaping or sanitisation.
- **Secrets in templates.** API keys and passwords from `dsc.toml` are not exposed as template variables. Only `baseurl`, `name`, `fullname`, and user-declared `[template.vars]` / `[discourse.template]` string values are available.
- **Per-category or per-topic variables.** Variables are per-forum only. Different categories within the same forum share the same variable map.
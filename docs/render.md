# dsc render

Fill `{{ variable }}` placeholders in a local Markdown (or text) template file using variables resolved from a forum's configuration, so a shared content-template library can be adapted for a specific Discourse without manual find-and-replace.

## dsc render

```text
dsc render <discourse> <file> [-o <output>] [--format text|json|yaml]
```

Reads `<file>` (or stdin, when `<file>` is `-`), substitutes every `{{ variable }}` placeholder it finds using the named forum's resolved template variables, and writes the result to stdout or to the path given by `-o`/`--output`.

`--format json` emits `{"rendered": "..."}`, and `--format yaml` emits `rendered: |-\n  ...`, for scripting. Default is `text`, which prints the raw rendered content. `--format` is ignored when `-o` is given: the file always receives the raw rendered text.

An unknown variable (a `{{ foo }}` with no `foo` in the resolved map) is not a hard error: `dsc render` prints a warning to stderr naming the variable, substitutes an empty string, and keeps rendering the rest of the file.

`dsc render` does not touch Discourse's own `%{...}` placeholders (e.g. `%{reply_to_username,fallback:there}`). Those are server-side substitution tokens and pass through untouched — `dsc`'s `{{ }}` syntax is chosen specifically to avoid colliding with them.

Honours global `-n`/`--dry-run`: prints the resolved variable map to stderr and the rendered output to stdout, without writing to `-o`.

```bash
dsc render myforum welcome.md
dsc render myforum welcome.md -o welcome.rendered.md
dsc render myforum welcome.md --dry-run   # preview the resolved variables
```

## Variable resolution

Variables resolve from three layers; later layers override earlier ones on a same-name key.

1. **Built-ins**, derived automatically from the matched `[[discourse]]` block: `forum_baseurl` (`baseurl`), `forum_name` (`name`), `forum_fullname` (`fullname`).
2. **`[template.vars]`**, a top-level `dsc.toml` table of flat string variables shared across every forum.
3. **`[discourse.template]`**, an optional sub-table inside a `[[discourse]]` block for forum-specific overrides and additions.

```toml
[template.vars]
organisation = "Koloki Ltd"
community = "Koloki Community"

[[discourse]]
name = "openehr"
baseurl = "https://discourse.openehr.org"
fullname = "openEHR International"

[discourse.template]
organisation = "openEHR International"
support_email = "admin@openehr.org"
```

Given the config above, `dsc render openehr welcome.md` on a file containing:

```markdown
Welcome to {{ community }}! Brought to you by {{ organisation }}.
Visit {{ forum_baseurl }} or email {{ support_email }}.
```

produces:

```markdown
Welcome to Koloki Community! Brought to you by openEHR International.
Visit https://discourse.openehr.org or email admin@openehr.org.
```

`organisation` resolves from `[discourse.template]`, since a per-forum value wins over the `[template.vars]` global of the same name.

Both `[template.vars]` and `[discourse.template]` are optional. A config without them still renders successfully — only the three built-in variables are available.

## Template syntax

Phase 1 supports plain `{{ variable }}` interpolation only (backed by the [Tera](https://crates.io/crates/tera) engine). Filters, conditionals (`{% if %}`), and loops (`{% for %}`) are not part of this phase — a template using only bare `{{ name }}` placeholders is the supported surface.

YAML front matter at the top of a file is rendered like the rest of the content; `dsc topic push`/`category push` strip it separately after any rendering step.

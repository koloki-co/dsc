# `dsc update` - theme-derived label colour

Spec for optional per-Discourse terminal label colours during `dsc update`. Goal: make multi-forum output recognisably match each forum without making update dependent on live theme queries. Driver: operators want the update stream to use each forum's visual identity rather than an arbitrary-looking terminal colour.

## Motivation

`color_discourse_label()` currently hashes the Discourse key into one of twelve ANSI colours. The output is deterministic, not random, but it has no relationship to a forum's theme. A stored theme-derived colour would make concurrent update output easier to scan while preserving a useful fallback for incomplete configurations and offline commands.

## Current state (as of 2026-08-02)

`dsc theme show` already exposes a theme's `color_scheme_id`, and `dsc theme palette` reads colour schemes through the Admin API. What identifies the active site-default theme and an appropriate accessible key colour has not been verified against a live Discourse response. `dsc update` has no theme API dependency and must remain able to run from existing configurations.

## Proposed configuration and behaviour

```toml
[[discourse]]
name = "myforum"
update_colour = "#3f8f77"
```

- `update_colour` is optional, accepts exactly a six-digit `#RRGGBB` value, and is used only when colour output is enabled.
- The stored value is a cache of a user-selected key colour from the active theme/palette, not an authority that must be refreshed before every update.
- A future explicit refresh surface discovers the active default theme, resolves its palette, presents candidate colours, and writes the selected value to `dsc.toml`. `dsc update` itself never rewrites configuration.
- With no configured value, invalid cached value, disabled colour, or a non-terminal destination, retain current behaviour: respect `NO_COLOR` and `DSC_COLOR`, then use the deterministic ANSI hash fallback when colour is allowed.
- Use 24-bit ANSI colour only where the existing output-colour policy allows it. Do not add escape sequences to JSON, YAML, logs, or redirected output.

## Reference: API discovery required

Before implementation, capture redacted responses from a real supported Discourse version for:

```text
GET /admin/themes.json
GET /admin/themes/:id.json
GET /admin/color_schemes.json
```

Confirm how the default active theme is identified, how it relates to `color_scheme_id`, whether the scheme exposes semantic roles such as `primary`, and how an administrator should choose a legible label colour. The theme admin API is not formally versioned, so the recorded evidence must include the Discourse version.

## Phases

### Phase 1 - discovery

- [ ] Capture the active-theme and colour-scheme response shapes from a live supported forum.
- [ ] Decide the explicit refresh command and whether candidate selection is automatic or user-confirmed.

### Phase 2 - configuration and rendering

- [x] Add and validate optional `update_colour` configuration, docs, and schema output. Implemented on `main`: `DiscourseConfig.update_colour` accepts a strict `#RRGGBB` string (empty string treated as unset, like other optional string fields); an invalid value warns on config load and falls back rather than erroring.
- [x] Render configured truecolour labels while preserving existing `NO_COLOR`, `DSC_COLOR`, non-TTY, and hash-fallback behaviour. `color_discourse_label` takes an optional `update_colour` override and emits 24-bit ANSI (`\x1b[1;38;2;r;g;bm`) when it parses, otherwise falls back to the existing twelve-colour hash.
- [x] Add configuration parsing and terminal-output regression coverage. See `src/config.rs` (`update_colour_parses_from_toml`, `update_colour_empty_string_is_none`, `warn_on_invalid_update_colour_does_not_panic_on_malformed_or_valid_values`) and `src/utils.rs` (`parse_hex_color_*`, `discourse_label_ansi_*`).

### Phase 3 - refresh ergonomics

- [ ] Implement the explicit read/choose/write refresh flow with dry-run support.
- [ ] Verify it against at least one light and one dark Discourse palette.

## Backward compatibility

This is additive. Existing `dsc.toml` files and update output keep their current deterministic ANSI fallback unless an operator opts into `update_colour`.

## Out of scope

- Changing Discourse theme or palette settings.
- Querying themes or rewriting `dsc.toml` as a side effect of `dsc update`.
- Altering machine-readable output, logs, or the global colour-disable controls.

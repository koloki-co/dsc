# dsc user-field

List, inspect, export, compare, and safely create Discourse custom user-field definitions. The `ucf` alias is available for interactive use.

## dsc user-field list

```text
dsc user-field list <discourse> [--format text|json|yaml]
```

Lists custom field definitions. This concerns the forum-wide definitions only, never individual users' responses.

## dsc user-field show

```text
dsc user-field show <discourse> <field-id> [--format text|json|yaml]
```

Shows one definition by its forum-local ID.

## dsc user-field pull

```text
dsc user-field pull <discourse> <local-path> [--force]
dsc user-field pull <discourse> <field-id> [local-path] [--force]
```

Exports every definition as a versioned YAML snapshot, or JSON when `<local-path>` ends in `.json`. A numeric `<field-id>` pulls only that definition. Without `[local-path]`, a single-field pull writes `user-field-<id>-<field-name>-<YYYY-MM-DD>.yaml` in the current directory, where the field name is slugified. Pulled entries retain their forum-local IDs and source URL. The command refuses to overwrite an existing file without `--force`.

## dsc user-field diff

```text
dsc user-field diff <discourse> <local-path> [--format text|json|yaml]
```

Compares a curated snapshot with the live forum. This initial implementation is create-only: entries with IDs, or names that already exist remotely, are refused rather than updating a definition. To copy definitions to another forum, remove their IDs and set `source` to the target URL deliberately.

## dsc user-field push

```text
dsc user-field push <discourse> <local-path>
```

Creates every new field in a valid curated snapshot after checking the live forum for exact-name collisions. It sends no update, delete, option replacement, or type-change request. Each created field is checked against the API response before the command continues.

`--dry-run` fetches the required live state and prints the create plan but sends no POST request. A failed read or an unsupported update aborts before any write.

## Snapshot shape

```yaml
version: 1
source: https://forum.example
user_fields:
  - name: Countries
    description: Which countries do you work in? Select all that apply.
    field_type: multiselect
    editable: true
    requirement: on_signup
    show_on_profile: false
    show_on_signup: true
    show_on_user_card: false
    searchable: false
    options:
      - Example country A
      - Example country B
```

Supported field types are `text`, `textarea`, `date`, `confirm`, `dropdown`, and `multiselect`. Dropdown and multiselect fields require a nonempty ordered option list with no blank or duplicate values. Other types must omit `options`.

Existing-definition updates remain deliberately unsupported because Discourse replaces all options when an options array is submitted. This can affect stored user answers. Pull the target and reconcile it manually until an evidence-backed guarded update workflow is added.

# dsc list

Lists all Discourse installs known to dsc, optionally filtered by tags.

```text
dsc list [--format <format>] [--tags <tag1,tag2,...>] [--open] [--verbose]
```

## Formats

`--format` (or `-f`) accepts:

- `text` (default)
- `markdown`
- `markdown-table`
- `json`
- `yaml`
- `csv`
- `urls` — one base URL per line, useful for piping

Output never includes API keys or other secret credential values in any format. There is deliberately no flag to reveal them; operators who need credentials must access the protected `dsc.toml` directly.

## Flags

- `--tags` — comma or semicolon separated, matches any tag (case-insensitive).
- `--open` (or `-o`) — open each listed Discourse base URL in a browser tab/window.
- `--verbose` (or `-v`) — include empty results and verbose listing details.

`--open` launches an opener for each selected URL without waiting for earlier openers to exit. Success means the openers were launched, not that the pages loaded: missing executables and other launch failures are reported, but later opener failures cannot be reported. Openers run noninteractively with stdin, stdout, and stderr disconnected from `dsc`, so they cannot prompt or hold output pipes open. `DSC_BROWSER_OPENER` overrides the platform opener. For a single forum with inherited stdio and a checked opener exit status, use [`dsc open`](open.md).

## Examples

```bash
# List all installs as a markdown table
dsc list --format markdown-table

# Open all installs tagged "client" in the browser
dsc list --open --tags client

# Pipe URLs to another command
dsc list -f urls --tags alpha | xargs -n1 xdg-open
```

## dsc list tidy

Orders the `dsc.toml` file entries alphabetically by name. Collects any missing full names by querying the Discourse URLs.

```bash
dsc list tidy
```

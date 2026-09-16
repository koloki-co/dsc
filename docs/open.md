# dsc open

Opens a Discourse install, or a whole fleet selection, in the default browser.

```
dsc open <discourse>
dsc open --all
dsc open --tags <tag1,tag2>
dsc open <pattern>
```

`<discourse>` is a configured forum's exact name. It may instead contain `*` (any run of characters) or `?` (exactly one character), in which case every forum whose name matches the glob opens. `--all` opens every configured forum, and `--tags` opens every forum carrying any of the given tags (comma/semicolon separated) - the same fleet selector the other fan-out commands share.

A fleet selection (`--all`, `--tags`, or a name glob) launches one detached opener per selected forum: it does not wait for earlier openers to exit, so one slow or hung opener cannot hold up the rest. Success means the openers were launched, not that the pages loaded; missing executables and other launch failures are reported, but later opener failures cannot be. Openers run noninteractively (stdin/stdout/stderr disconnected from `dsc`). A single exact name keeps the interactive contract: `dsc open` waits for the opener to exit, inherits stdin/stdout/stderr, and reports a nonzero opener exit status as an error. Neither form waits for the page to load. In contrast, [`dsc list --open`](list.md) combines listing with the same detached fleet-open behavior.

The browser opener can be overridden with the `DSC_BROWSER_OPENER` environment variable.

## Examples

```bash
# Open a forum in the browser
dsc open koloki-demo

# Open every configured forum, each in its own tab
dsc open --all

# Open forums tagged production
dsc open --tags production

# Open forums whose names match a glob
dsc open 'forum.rc*'
```

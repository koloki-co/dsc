# dsc open

Opens a Discourse install in the default browser.

```
dsc open <discourse>
```

Opens the `baseurl` for the named Discourse. The browser opener can be overridden with the `DSC_BROWSER_OPENER` environment variable.

This command waits for the opener to exit, inherits stdin/stdout/stderr, and reports a nonzero opener exit status as an error. It does not wait for the page to load if the opener itself returns earlier. In contrast, [`dsc list --open`](list.md) launches fleet openers noninteractively without waiting for their exit statuses.

## Examples

```bash
# Open a forum in the browser
dsc open koloki-demo
```

# dsc version

```
dsc version [<discourse> | --all | --tags <tags>] [--format text|json|yaml]
```

With no target selector, prints `dsc`'s own version without loading configuration.

With a forum name, prints that forum's **live Discourse version and git commit** using the configured API key, so it works even on login-required forums where an anonymous request is rejected. The version comes primarily from `/about.json`; the commit comes from the homepage's generator metadata.

`--all` checks every configured forum. `--tags` checks forums matching any comma-separated tag. Fleet requests use bounded concurrency and preserve configuration order in JSON/YAML output. An unavailable or unconfigured forum is retained as an error row; other forums still complete, and the command exits non-zero after printing the complete result.

```bash
dsc version                # → 0.10.20  (dsc itself)
dsc version accm           # → accm: Discourse 2026.6.0-latest (70aacf7…)
dsc version --all
dsc version --tags production --format json
```

This is useful for checking which build each forum in your fleet is on, such as whether every forum includes a recently bundled core plugin.

# dsc emoji

Manage custom emoji on a Discourse install.

Requires an admin API key and username.

## dsc emoji pull

```
dsc emoji pull <discourse> <output-dir>
```

Downloads all custom emoji from the instance into `output-dir`. Images are saved as `<name>.<ext>` where the extension is derived from the remote URL. Files that already exist locally are skipped.

## dsc emoji push

```
dsc emoji push <discourse> <emoji-path> [emoji-name]
```

Adds a new emoji from a local image file (alias: `dsc emoji add`). If `emoji-name` is omitted, the filename stem is used, slugified to Discourse's accepted shape (lowercased, hyphens preserved) - so `google-drive.svg` becomes `google-drive`.

If `emoji-path` is a directory, uploads all `.png`, `.jpg`, `.jpeg`, `.gif`, `.svg` files using the filename stem as the emoji name.

If your instance requires a `client_id` query parameter for admin emoji endpoints, set `DSC_EMOJI_CLIENT_ID` to append it automatically.

Bulk uploads retry automatically on HTTP 429 responses, reading the wait time from the `Retry-After` header, the `extras.wait_seconds` JSON field, or the message body.

If you consistently hit rate limits on large batches, raise `DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE` (default 60). This is a Discourse global setting, not a site setting — it is not visible in the Admin UI. On a standard Docker install, set it under `env:` in `/var/discourse/containers/app.yml` and rebuild the container. Nginx-level 429s (HTML body, `nginx` in the response) come from the reverse proxy, not Discourse itself, and must be raised in the proxy config.

## dsc emoji list

```
dsc emoji list <discourse> [--format text|json|yaml] [--inline] [--verbose]
```

Lists custom emojis (name + URL).

Flags:

- `--inline` (or `-i`) — render emoji images inline in supported terminals.
  - Override detection with `DSC_EMOJI_INLINE_PROTOCOL=iterm2|kitty|off`.
- `-v`, `--verbose` — include additional fields where supported.

## dsc emoji delete

```
dsc emoji delete <discourse> <name>... [--dry-run]
```

Deletes one or more custom emojis by name (alias: `dsc emoji rm`). Names are matched case-insensitively against the forum's existing custom emoji; any requested name not found is reported as an error and nothing is deleted. `--dry-run` prints the plan without deleting.

If a custom emoji shadowed a built-in emoji of the same name (a common reason to delete one — e.g. replacing an accidentally-uploaded `heart`), references that resolve the name after deletion may fall back to the built-in emoji. Discourse rebakes ordinary posts after deletion, while other persisted content may refresh differently; the dry-run output states this consequence without promising that every stored rendering changes immediately.

Uses `DELETE /admin/config/emoji/:name.json`, falling back to the legacy `/admin/customize/emojis/:name.json` route on older Discourse versions. After Discourse accepts each deletion, `dsc` lists emoji again and refuses to report success while the name remains. This catches plugin-provided emoji, which can appear in the admin list but cannot be removed through the custom-emoji endpoint.

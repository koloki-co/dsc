# dsc upload

Upload a file (typically an image) to a Discourse install. Returns the short `upload://…` URL that can be embedded in topic and reply Markdown.

```text
dsc upload <discourse> <file> [--upload-type composer] [--format text|json|yaml]
```

In default text mode, prints just the short URL — designed to be captured into a variable or piped:

```bash
url=$(dsc upload myforum ./diagram.png)
echo "Posted ![diagram]($url)" | dsc topic reply myforum 1525
```

`--upload-type` controls Discourse's `type` field. `composer` (the default) is for embedding in posts. Others: `avatar`, `profile_background`, `card_background`, `custom_emoji`.

Use `--format json` for the full upload payload (id, full URL, filesize, dimensions if applicable).

## PNG-to-JPEG conversion

Discourse's `UploadCreator` converts a PNG to JPEG server-side when the image exceeds 1280x720 pixels and the resulting JPEG would be smaller, controlled by the `png_to_jpg_quality` site setting (set it to `100` to disable conversion site-wide). The converted file comes back with a `.jpeg` extension and, if the source PNG had a transparent background, that transparency is gone.

The uploaded file's extension can therefore differ from what you passed in. Text-mode output flags this when it happens:

```text
$ dsc upload myforum ./logo.png
note: Discourse returned this upload as "logo.jpeg" (.jpeg instead of .png). PNG-to-JPEG conversion loses transparency; use --upload-type custom_emoji to skip conversion, or set png_to_jpg_quality to 100 to disable it site-wide.
upload://a1B2c3D4e5F6.jpeg
```

(`--format json`/`yaml` already carry the returned `original_filename`, so no separate hint is printed there.)

`--upload-type custom_emoji` is a known conversion-exempt path if you need the PNG preserved as-is (e.g. a logo with transparency) without changing the site-wide setting.

## Examples

```bash
# Get the short URL for a screenshot
dsc upload myforum ./screenshot.png
# upload://a1B2c3D4e5F6.png

# Upload and open the resulting URL in the browser
dsc upload myforum ./diagram.png --format json | jq -r .url | xargs xdg-open

# Inline upload into a reply, all in one shell line
echo "Build output:\n\n![log]($(dsc upload myforum ./build.log))" \
  | dsc topic reply myforum 1525
```

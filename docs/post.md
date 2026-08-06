# dsc post

Operations on individual posts (not whole topics). Complements `dsc topic pull/push/sync`, which only work on the OP.

## dsc post pull

```text
dsc post pull <discourse> <post-id> [<local-path>]
```

Downloads the raw Markdown body of the specified post. Writes to `<local-path>` if given, otherwise to stdout.

```bash
dsc post pull myforum 98765 ./post-98765.md
dsc post pull myforum 98765 > post.md
```

## dsc post push

```text
dsc post push <discourse> <post-id> [<local-path>]
```

Replaces the body of the specified post with the contents of `<local-path>` (or stdin if omitted, or `-`). Requires either an admin API key or a key whose user is the post author. (Alias: `dsc post edit`.)

```bash
dsc post push myforum 98765 ./corrected.md
echo "Edited on the fly." | dsc post push myforum 98765
```

## dsc post delete

```text
dsc post delete <discourse> <post-id>
```

Soft-deletes the post. This is post-ID based; use it when you know the specific post ID. To delete a whole topic by topic ID, use `dsc topic delete`. Supports `--dry-run` (or `-n`).

## dsc post move

```text
dsc post move <discourse> <post-id> --to-topic <topic-id>
```

Moves the post to a different topic. `dsc` infers the source topic from the post itself, so you only supply the destination. Prints the URL of the target topic on success. Supports `--dry-run`.

```bash
dsc post move myforum 98765 --to-topic 1525
dsc -n post move myforum 98765 -t 1525   # dry run
```

## dsc post info

```text
dsc post info <discourse> <post-id> [--format text|json|yaml]
```

Read-only lookup that resolves a post ID to its canonical topic URL and minimal metadata: post ID, topic ID/title/slug/category ID, post number, URL, and deletion state. Supports staff-visible soft-deleted posts and topics. Never prints raw post content, author data, or credentials — use `dsc post pull` when you need the body. (Alias: `dsc post i`.)

```bash
dsc post info myforum 61695
dsc post info myforum 61695 --format json
```

## Notes

- Post edits and deletes require either admin scope or that the API user owns the post.
- Moving a post that is the first post of its topic will move the whole topic. Discourse's API enforces this.

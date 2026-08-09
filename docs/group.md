# dsc group

List, inspect, and copy groups.

## dsc group list

```
dsc group list <discourse> [--format text|json|yaml] [--verbose]
```

Lists all groups with their IDs, names, and full names. `-v`/`--verbose` includes additional fields where supported.

## dsc group info

```
dsc group info <discourse> <group-id> [--format json|yaml] [--with-defaults]
```

Shows details for a specific group. `--with-defaults` additionally includes the group's category and tag notification-level defaults (`watching_category_ids`, `tracking_tags`, etc.) as returned by the Discourse API for admins/owners; omitted by default.

## dsc group members

```
dsc group members <discourse> <group-id> [--format text|json|yaml]
```

Lists members of the specified group.

## dsc group copy

```text
dsc group copy <source-discourse> <group-id> [--target <target-discourse>]
```

Copies the specified group. If `--target` is omitted, copies within the same Discourse.

- The copied group name is slugified and suffixed with `-copy` (e.g., `staff` -> `staff-copy`).
- The copied group full name is set to `Copy of <original full name>`.
- All other fields match the source, except the ID which is assigned by Discourse.

`<group-id>` can be found using `dsc group list`.

## dsc group add

```text
dsc group add <discourse> <group-id> [<local-path>] [--notify]
```

Bulk-add members to a group by email. The input is one email per line; blank lines and `#` comments are ignored; duplicates and case differences are collapsed. Reads stdin when `<local-path>` is omitted or `-`. With `--notify`, Discourse sends each added user a notification.

Emails must resolve to existing users on the Discourse — the endpoint does not auto-invite unknown addresses. To invite new people, use Discourse's invites UI (or Phase 2's planned `dsc invite`).

Honours `--dry-run` (`-n`), which prints the cleaned email list without sending.

```bash
# From a file
dsc group add myforum 42 ./new-members.txt

# From stdin with a pipeline
printf 'alice@example.com\nbob@example.com\n' | dsc group add myforum 42

# Preview first
dsc -n group add myforum 42 ./new-members.txt
```

The symmetric per-user view is [`dsc user groups`](user.md).

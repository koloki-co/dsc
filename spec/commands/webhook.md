# `dsc webhook list|create|delete|ping` - basic webhook administration

> **Status: implemented on main (unreleased).** Basic admin-scope CRUD-plus-ping over Discourse's
> outbound webhook admin API (`/admin/api/web_hooks.json`). Roadmap item R17.

Driver: `dsc` already administers API keys (`dsc api-key`) via the same
class of small admin-scope CRUD endpoint; webhooks are the natural next
"basic administration" surface, following the same pattern.

## Command surface

```
dsc webhook list <discourse> [--format text|json|yaml]

dsc webhook create <discourse> <payload_url> [--content-type json|form]
                                               [--secret <secret>]
                                               [--inactive]
                                               [--no-verify-certificate]
                                               [--format text|json|yaml]

dsc webhook delete <discourse> <webhook_id> [--format text|json|yaml]

dsc webhook ping <discourse> <webhook_id> [--format text|json|yaml]
```

`webhook` has the short alias `wh`; `list`/`create`/`delete` alias to
`ls`/`cr`/`rm`, matching `dsc api-key`.

- `list` - all configured webhooks.
- `create` - a new wildcard webhook. Per-event-type selection is out of scope for this first cut. The command fetches and attaches Discourse's current default event types because an empty event-type list would save successfully but receive no normal deliveries.
  - `--content-type` - `json` (default, `application/json`) or `form`
    (`application/x-www-form-urlencoded`). Sent to Discourse as its integer
    encoding (`1`/`2`).
  - `--secret` - shared secret Discourse signs deliveries with, returned in
    the `X-Discourse-Event-Signature` request header on each delivery.
  - `--inactive` and `--no-verify-certificate` negate the two boolean fields that otherwise default on (`active`, `verify_certificate`).
  - A supplied secret must be at least 12 characters and is never printed, including under `--dry-run`.
  - Payload URL userinfo is redacted in every output format.
- `delete` - remove a webhook by ID.
- `ping` - enqueue a one-off test delivery (Discourse's own "Ping" button
  in the admin UI). Any 2xx response counts as success; Discourse's ping
  route isn't documented to return meaningful body content.

`create`, `delete`, and `ping` all honour `--dry-run` - each enqueues a real
side effect on the forum (a webhook row, its deletion, or a live test
delivery job), so all three print a `[dry-run] ...` plan and return without
making a request. `list` is read-only and ignores `--dry-run`.

## Endpoints used

| Subcommand | Method | Path |
|---|---|---|
| `list`   | `GET`    | `/admin/api/web_hooks.json` |
| `create` | `POST`   | `/admin/api/web_hooks.json` |
| `delete` | `DELETE` | `/admin/api/web_hooks/{id}.json` |
| `ping`   | `POST`   | `/admin/api/web_hooks/{id}/ping.json` |

`list` follows Discourse's offset pagination (`?offset=N`, 50 rows per page) until its `total_rows_web_hooks` count is reached. `create` sends permitted `web_hook[...]` form fields (`payload_url`, `content_type`, `secret`, `wildcard_web_hook`, `active`, `verify_certificate`, and `web_hook_event_type_ids[]`).

## Output

- **text** (default): `list` prints one line per webhook - ID, payload URL,
  `events:all|selected`, `active|inactive`. `create` prints the new
  webhook's ID plus the fields it was created with.
- **json** / **yaml**: safe webhook records with ID, redacted payload URL, delivery and scope fields, categories, groups, tags, and event types.

Discourse's admin serializer includes webhook signing secrets. `dsc` deliberately converts server responses to an explicit public type that omits those secrets, so later server fields cannot accidentally appear in JSON or YAML output.

## Notes

- Admin-scope only - the configured `api_username` must be a staff member,
  same as `dsc api-key`.
- No per-event-type selection yet - add a `--event <name>` picker on demand.
- No `update`/`edit` subcommand yet - only list/create/delete/ping, matching
  the roadmap item's scope. Add `dsc webhook update` separately if a
  concrete need for editing an existing webhook's URL/secret/flags arises.

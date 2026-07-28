# dsc webhook

Manage outbound Discourse webhooks with an administrator API key.

## dsc webhook list

```text
dsc webhook list <discourse> [--format text|json|yaml]
```

Lists every configured webhook, following Discourse pagination. Output includes the ID, URL, state, delivery status, scopes, tags, and event types. Webhook signing secrets are never printed, and credentials embedded in a payload URL are redacted.

## dsc webhook create

```text
dsc webhook create <discourse> <payload_url> [--content-type json|form] [--secret <secret>] [--inactive] [--no-verify-certificate] [--format text|json|yaml]
```

Creates an active wildcard webhook by default. `dsc` attaches Discourse's default event types, so the new webhook receives normal event deliveries. Per-event selection is not yet supported.

The payload URL must be an absolute HTTP or HTTPS URL. A supplied signing secret must contain at least 12 characters. Neither the secret nor URL credentials appear in normal or dry-run output.

## dsc webhook delete

```text
dsc webhook delete <discourse> <webhook_id> [--format text|json|yaml]
```

Deletes a webhook by positive numeric ID.

## dsc webhook ping

```text
dsc webhook ping <discourse> <webhook_id> [--format text|json|yaml]
```

Enqueues Discourse's test `ping` event for a webhook. It is a real delivery and therefore honours global `--dry-run`.

All mutating subcommands print a complete `[dry-run]` plan without contacting the forum when invoked with `-n` or `--dry-run`.

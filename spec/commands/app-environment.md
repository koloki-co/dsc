# `dsc app env` - inspect and safely manage Docker `app.yml` environment

> **Status: Phases 1 and 2 implemented (unreleased).** `dsc app env list|get|audit|set|unset` reads and safely edits a constrained scalar `env:` mapping over SSH, uses an optional per-forum `app_yml_path`, and applies conservative secret redaction.

Spec for reading, auditing, and eventually changing the `env:` section of a self-hosted Discourse Docker install's `containers/app.yml` over SSH. Goal: make fleet configuration observable and controlled without manually logging into every host. Driver: Digital Health Discourse hit an admin API rate limit while `dsc tag pull` ran; the immediate setting to inspect or raise is `DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE`.

## Motivation

A fleet operator needs to know which Docker-level configuration is present across owned forums, beginning with environment variables such as `DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE`. Today the workflow is SSH to each host, inspect or edit `/var/discourse/containers/app.yml`, then run `./launcher rebuild app`. `dsc setting audit` cannot help because Docker environment variables are outside Discourse site settings and are not exposed by the admin API.

## Current state (as of 2026-07-14)

- `dsc` can SSH for `update`, but has no command to read or alter `containers/app.yml`.
- `dsc update` assumes the standard Docker checkout at `/var/discourse`; `DiscourseConfig` has no configurable `app.yml` path.
- `DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE` defaults to 60 in standard Discourse Docker and is not visible in the Admin UI. It is an environment setting, so a rebuild is needed before a changed value takes effect.

## Proposed CLI surface

```text
dsc app env list <discourse> [--format text|json|yaml]
dsc app env get <discourse> <key> [--show-secret] [--format text|json|yaml]
dsc app env audit <key> [--tags <tags>] [--format text|json|yaml]
dsc app env set <discourse> <key> <value> [--rebuild] [--backup] [--dry-run]
dsc app env unset <discourse> <key> [--rebuild] [--backup] [--dry-run]
```

- `list` returns environment-variable names only by default, sorted. It never prints values that look secret.
- `get` returns one value, redacting a secret-looking key unless the operator explicitly supplies `--show-secret`.
- `audit` fans out across matching configured forums, showing whether the key is unset, redacted, or has a non-secret value. It must never aggregate or print secrets, including with `--show-secret`.
- `set` and `unset` change one scalar `env:` entry only. They create a timestamped remote backup by default, show a before/after plan under `--dry-run`, write atomically, re-read and verify the result, and do not rebuild unless `--rebuild` is explicit. `--rebuild` uses the same rebuild-lock guard as `dsc update`; the normal destructive confirmation applies unless `--yes` is supplied.
- The default remote file is `/var/discourse/containers/app.yml`. Add an optional per-forum `app_yml_path` config field for nonstandard/rootless layouts; do not guess arbitrary home-directory paths.

## Reference: field workflow observed

No Discourse API exists for this state. The manual SSH/Docker workflow is:

```text
# inspect
ssh <host> 'grep -n DISCOURSE_MAX_ADMIN_API_REQS_PER_MINUTE /var/discourse/containers/app.yml'

# edit /var/discourse/containers/app.yml, then apply the changed environment
ssh <host> 'cd /var/discourse && ./launcher rebuild app'
```

The request surfaced on 2026-07-14 while a read-only `dsc tag pull dhi-discourse` received 429 responses and waited for 53 seconds between retries. The live retry was stopped after the rate-limit wait; no forum state was changed.

## Phases

### Phase 1 - blocking: inspect and audit

- [x] `app env list`, `get`, and fleet `audit` over SSH.
- [x] `app_yml_path` config field with the standard path as its documented default.
- [x] Conservative secret-key detection and redaction; structured output and empty-value behaviour.
- [x] Unit tests for parsing representative `env:` blocks and redaction. A disposable SSH-host integration fixture remains a live-test infrastructure follow-up.

### Phase 2 - controlled environment edits

- [x] Constrained text-preserving edit of a scalar `env:` mapping; reject unsupported YAML forms rather than reserialising and losing comments, anchors, or unrelated sections.
- [x] Atomic write, timestamped backup, post-write re-read verification, dry-run plan, and `--rebuild` integration with the rebuild lock.
- [x] Unit coverage for set/unset edits and rejection of unsupported syntax. A disposable SSH-host integration fixture remains a live-test infrastructure follow-up.

### Phase 3 - deliberately separate follow-up

- [ ] Consider a read-only inventory of selected non-`env:` top-level keys (`templates`, `hooks`, `volumes`) if a concrete fleet need arises.
- [ ] Consider a guarded pull/edit/push workflow for a managed subset only, with an explicit secret-handling design.

## Backward compatibility

No existing command changes. The new `app_yml_path` field is optional. Existing `dsc update` behaviour and the default `/var/discourse` Docker layout remain unchanged.

## Out of scope

- Editing arbitrary YAML or reformatting an entire `app.yml` document.
- Exporting or comparing SMTP/API/database credentials across forums.
- Proxy, Cloudflare, load-balancer, or cloud-firewall configuration; those are not in Discourse Docker's `app.yml`.
- Disabling rate limiting globally or bypassing Discourse's safety controls.
- Multi-container layouts (`data.yml`, `web_only.yml`) until a real deployment needs them.

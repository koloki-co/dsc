# `dsc` User API authentication

> **Status: Exploratory specification for R27. Not scheduled for implementation.**
>
> Phase 0 is a compatibility spike against disposable regular-user, moderator, group-manager, and admin accounts. No User API command should be advertised until its live role/endpoint behavior is confirmed.

Spec for adding Discourse User API key authentication to `dsc`. Goal: let a person authorize `dsc` to act as their own Discourse account without copying an Admin API key, while preserving the existing administrator and SSH workflows. The useful first surface is read/search/pull, notifications, private messages, uploads, invites, and content the authorizing user may create or edit.

Driver: `dsc` currently requires `Api-Key` plus `Api-Username` for almost every authenticated HTTP command. Discourse's User API key protocol provides user-approved, revocable, optionally expiring credentials with separate rate limits and an authorization device flow designed for CLI pollers. This widens both the audience and the safe least-privilege surface; it does not bypass the authorizing user's normal Discourse permissions.

## Terminology

- **Server API authentication**: Discourse authentication using `Api-Key`. A global key also needs an actor selector such as `Api-Username`; a user-bound server key does not. `dsc` currently requires and sends both fields and calls this profile `admin-api`.
- **User API authentication**: a user-approved key sent in `User-Api-Key`, with optional `User-Api-Client-Id`. The key is permanently bound to one user.
- **User API authorization flow**: the OAuth-like RSA/device-code exchange that creates a User API key. It is not standard OAuth and does not issue OAuth access or refresh tokens.
- **Scope**: a route-method gate on a User API key. A scope never grants a permission that the authorizing user does not already have.
- **Role permission**: the ordinary Guardian authorization applied by Discourse after scope checking, including visibility, ownership, trust level, group management, category moderation, staff, or admin status.
- **Server-API-only behavior**: an endpoint or behavior explicitly conditional on Discourse's `is_api?`. User API authentication deliberately sets `is_user_api?`, not `is_api?`, even when the user is an admin.

Use **User API key**, not "OAuth token", in CLI copy, config fields, docs, and code identifiers.

## Current state (as of 2026-07-27)

`DiscourseClient::new` only configures `Api-Key` and `Api-Username` (`src/api/client.rs`). `ensure_api_credentials` rejects a Discourse entry unless both `apikey` and `api_username` are present (`src/commands/common.rs`). The config schema has no authentication discriminator or User API fields (`src/config.rs`).

Consequences today:

- No API-backed `dsc` command sends `User-Api-Key`.
- Commands cannot distinguish "missing credential" from "this command requires Admin API authentication".
- A global Admin API key can select an actor with `Api-Username`; a User API key cannot impersonate another actor.
- SSH/local commands do not conceptually require Admin API authentication, but some hybrid workflows currently require `apikey` and `api_username` for HTTP preflight calls.
- `DiscourseConfig` derives `Debug` and `Serialize`, so direct structured serialization would expose `apikey`. `dsc list` now uses an explicit non-secret DTO rather than deferring that security fix to unscheduled R27. R27 must preserve that boundary and add redacted config debugging before introducing another credential field.

R27 adds an authentication backend and user login flow. It does not fork `dsc` into a separate user edition and does not duplicate command implementations.

## Discourse scope model

Current built-in scopes in `UserApiKeyScope`:

| Scope | Route gate | Relevance to `dsc` |
|---|---|---|
| `read` | All `GET` requests | Main read/search/pull scope |
| `write` | `GET`, `POST`, `PATCH`, `PUT`, `DELETE` | Includes read; needed for content writes |
| `notifications` | Notification list/totals/mark-read and notification message bus | Narrow notification profile |
| `session_info` | Current session and topic-tracking state | Identity/status without broad read |
| `message_bus` | Message bus `POST` | Deferred; no current streaming command |
| `push` | No ordinary route matcher | Deferred; an allowlisted push callback can be active with either `push` or `notifications` |
| `one_time_password` | Special one-time-login endpoint | Out of scope; disallowed in device authorization |
| `bookmarks_calendar` | User bookmarks in ICS format | No current command |
| `user_status` | Get/set/clear user status | Possible later command |

Plugins may register additional scopes. The site setting `allow_user_api_key_scopes` determines which scopes may be requested. A registered User API client can impose a narrower allowed set. The user then approves the requested set.

`write` already includes `GET`; requesting both `read` and `write` is redundant. `dsc` should normalize that combination and explain it in dry-run/login output rather than request duplicate scopes.

Scope authorization is only the first gate. Effective access is:

```text
site-allowed scopes
INTERSECT client-allowed scopes
INTERSECT scopes requested by dsc and approved by the user
INTERSECT the user's Guardian permissions for the resource/action
INTERSECT endpoint-specific authentication rules
```

## Proposed CLI surface

```text
dsc login  <discourse> [-s|--scope <scope>]... [-e|--expires-in <duration>] [-r|--replace-scopes] [-u|--replace-account] [-o|--no-open] [-f|--format text|json|yaml]
dsc logout <discourse> [-l|--local-only] [-f|--format text|json|yaml]
dsc auth status  <discourse> [-k|--check] [-f|--format text|json|yaml]
dsc auth explain [-f|--format text|json|yaml] -- <command> [<args>...]

dsc --auth admin-api|user-api|anonymous <command> ...
```

Examples:

```bash
dsc login community
dsc login community --scope write
dsc login community --scope notifications --scope session_info
dsc login community --scope read --expires-in 30d
dsc login community --scope read --no-open
dsc auth status community --check
dsc auth explain -- topic pull community 123 --full
dsc --auth admin-api api-key create community "CI integration key"
dsc logout community
```

`login` requires an existing Discourse entry with `baseurl`; it does not create the entry. `dsc add` remains the config-entry command. A later convenience may accept a URL plus `--name`, but it is out of the first phase.

Default first-login scope is `read`. Reauthorization without `--replace-scopes` requests the union of the recorded existing scopes and all new `--scope` values because authorizing a replacement key invalidates the old key. If the existing profile has no recorded scopes, `login` refuses additive reauthorization and requires `--replace-scopes`. `--replace-scopes` deliberately replaces the set and is the way to reduce privilege. Normalization removes `read` when `write` is present. `dsc` MUST NOT silently request `write` because a later command needs it.

A successful `login` stores the User API profile and sets `auth = "user-api"`. The dry-run plan and final result must say when this changes the selected profile from `admin-api` or `anonymous`.

`--auth` is a long-only global Phase 1 selector and is accepted before or after the command. A short alias would conflict with inherited subcommand flags, including existing `-a/--convert-admonitions`. It selects the named profile for every HTTP leg of an ordinary single-forum command. Without an override, each Discourse entry uses its configured selection rules. Mixed-profile multi-forum mutations remain disabled for User API authentication until the command metadata can express independent source and target selectors; R27 does not guess stronger credentials for one leg.

Special authentication commands bypass ordinary selection: `login` uses an explicitly anonymous protocol client, `logout` uses the stored User API key unless `--local-only` is given, and `auth status` is local-only unless `--check` is given.

### Dry-run behavior

- `login --dry-run` validates the local entry, normalizes scopes, and prints the intended expiry, profile transition, and local fields without making an HTTP request, creating a device grant, opening a browser, or writing config. It labels server capability and scope acceptance as unverified.
- `logout --dry-run` prints whether remote revocation would be attempted and the exact local fields/profile selection that would change, without sending or writing anything.
- Read-only `auth status` and `auth explain` accept and ignore global `--dry-run`, consistent with the CLI contract.

## Device authorization flow

Prefer current Discourse's device flow over the original browser redirect callback flow. Routes called directly by `dsc`:

```text
HEAD /user-api-key/new
POST /user-api-key/device.json
POST /user-api-key/device/poll.json
POST /user-api-key/revoke
```

`dsc` opens the browser at `GET /user-api-key/activate`; the browser-side Discourse UI then uses the activation, authorize, and deny POST routes. Those browser requests are not made by the CLI client.

Flow:

1. Preflight config readability/writability and capture a digest of the selected entry before starting an authorization that may take ten minutes. Load a previously stored `user_api_client_id`, or generate and persist that non-secret stable ID before creating the device grant so denied, expired, and interrupted first logins reuse it. Require HTTPS for User API authorization and authenticated requests; tests may use an explicit loopback-only exception.
2. Probe `HEAD /user-api-key/new` with an anonymous client. Require a successful response and `Auth-Api-Device-Code: true`; parse `Auth-Api-Version` separately. Current source reports version 4, but version 4 alone does not advertise device-code support.
3. Generate an ephemeral RSA keypair of 2048-8192 bits and a cryptographic nonce.
4. Send `POST /user-api-key/device.json` with a JSON body and `Content-Type: application/json`. Include `nonce`, comma-separated `scopes`, `client_id`, `application_name`, public key, optional `expires_in_seconds`, and `padding=oaep`.
5. Receive `device_code`, `user_code`, `verification_uri`, `verification_uri_with_request`, `expires_in`, and `interval`. Reject malformed values, non-HTTPS verification URLs, and URLs whose origin differs from the configured Discourse origin. Anonymous and credential-bearing clients must not follow cross-origin redirects.
6. Open `verification_uri_with_request` unless `--no-open`; always show the URL and user code once for headless use. Reuse the existing `open_url` platform integration and `DSC_BROWSER_OPENER` test hook.
7. Wait one server-provided interval before the first poll, then send JSON `{ "device_code": "..." }` to `POST /user-api-key/device/poll.json` with `Content-Type: application/json`. Handle `authorization_pending`, `authorized`, `access_denied`, and `expired_token` explicitly.
8. Use a monotonic deadline. A zero interval or expiry is invalid; the client never polls more frequently than the advertised interval, never waits longer than the remaining grant lifetime, and applies any 429 wait inside the same deadline. Phase 0 must record acceptable upper bounds before Phase 1; those bounds become named, tested constants rather than unbounded server input.
9. RSA-decrypt an authorized payload requested with `padding=oaep` as RSAES-OAEP using SHA-1, MGF1-SHA-1, and an empty label, matching current Discourse/OpenSSL behavior. Verify the nonce before trusting the key, validate the API version, and parse optional `expires_at`.
10. Derive the new key owner's username before replacing an existing profile whenever its scopes permit a safe identity request. If the known account changes, require `--replace-account`; if an existing account cannot be compared safely, revoke the new key and refuse replacement. A first profile may retain an unknown username, but identity-dependent commands then refuse until identity can be established.
11. Keep the previous key in memory until transition checks complete. If server-side same-user/client replacement cannot be proven, self-revoke the previous key before overwriting its only local copy. On an inconclusive revocation failure, revoke the new key where possible and preserve the previous config rather than orphan a credential.
12. Re-read the config and merge only the selected entry's authentication fields. Detect a conflicting edit to that entry instead of replacing the whole stale config. Persist through a crash-durable writer that syncs the parent directory after rename on Unix, then discard the private RSA key, previous key, and decrypted payload.

Current defaults are a ten-minute device authorization lifetime and a five-second poll interval. Do not assume these constants; use validated response values.

Both device POST routes require an actual JSON request MIME type; the `.json` suffix alone is insufficient. Protocol JSON bodies are capped at 64 KiB and the encrypted payload field at 16 KiB unless Phase 0 evidence requires a different explicit bound.

### One-shot authorized payload

After approval, Discourse creates or replaces the key and retains the encrypted payload for at most 60 seconds or the remaining device-grant lifetime, whichever is shorter. The first poll returning `authorized` consumes and deletes the grant. A later poll returns `expired_token`.

An ambiguously lost authorized response is therefore not safely retryable: the server may have replaced the old key while the client cannot recover the new one. `dsc` must stop polling, explain that reauthorization is required, and never claim the previous local key remains valid. If the payload is received but config persistence fails, `dsc` attempts to revoke the new key, leaves the pre-existing config untouched, and reports that the prior same-client key may already have been invalidated.

### Authorized payload contents

The decrypted payload contains `key`, `nonce`, `push`, `api`, and optional `expires_at`. It does not contain the username or granted scopes. `user_api_username` therefore comes from a follow-up identity request, such as `GET /session/current.json` when a granted scope permits it, or an authenticated response's `X-Discourse-Username` header. It may remain unknown only for a first profile whose scopes expose no safe identity route. `user_api_scopes` records the normalized set requested and accepted by the authorization endpoint; it is local metadata, not data returned in the encrypted payload.

Phase 1 may require device-code support and fail with a compatibility message on older Discourse versions. Supporting the legacy callback/custom-scheme flow is a separate compatibility decision, not required for the first implementation.

## Login output

Interactive text example:

```text
stderr: Requesting User API access from https://community.example.com
stderr: Application: dsc CLI
stderr: Scopes: read
stderr: Expires: 30 days
stderr: Open: https://community.example.com/user-api-key/activate?request=AB12CD34
stderr: Code: WXYZ-2345
stderr: Waiting for approval...
stdout: community - user-api - marcus - read
```

Progress, browser instructions, and the one-time `user_code` go to stderr; only the final result goes to stdout. `login` and `logout` accept `--format`; structured output emits a non-secret result containing forum name, selected profile, optional username, scopes, expiry, and the completed local/server actions. Intentional one-time display of the verification URL and `user_code` is allowed. The polling `device_code` must never be displayed or logged.

Never print the User API key, encrypted payload, private key, or nonce. No existing global verbose/debug flag exists, but any future tracing facility is subject to the same rule.

## Authentication configuration

Proposed additive fields on each `[[discourse]]` entry:

```toml
[[discourse]]
name = "community"
baseurl = "https://community.example.com"

# Selected by default when both auth profiles exist.
auth = "user-api"

# Existing Admin API fields remain valid and unchanged.
apikey = "..."
api_username = "system"

# Written by `dsc login`.
user_api_key = "..."
user_api_client_id = "dsc-..."
user_api_username = "marcus"
user_api_scopes = ["read"]
user_api_expires_at = "2026-09-01T12:00:00Z"
```

Accepted `auth` values:

```text
admin-api
user-api
anonymous
```

A complete Admin API profile has both `apikey` and `api_username`. A complete User API profile has `user_api_key`; `user_api_client_id`, username, scopes, and expiry are metadata, although a key created by `dsc login` always has a stable client ID. Imported keys may omit that ID or scope metadata, but the next login persists a stable dsc client ID before authorization and requires `--replace-scopes` when the old scope set is unknown. Invalid expiry timestamps and unknown `auth` values are config errors. Partial profiles are surfaced by `auth status` and are never selected for an authenticated command.

Selection rules:

- If `auth` is absent and only a complete Admin API profile exists, select `admin-api`; this preserves existing configs.
- If `auth` is absent and only a complete User API profile exists, select `user-api`.
- If `auth` is absent and neither profile is complete, use anonymous HTTP where the command permits it; an authenticated command reports the incomplete/missing profile.
- If both profiles exist and `auth` is absent, fail with an ambiguity error rather than silently choose stronger credentials.
- If `auth` explicitly names an incomplete profile, fail even when another complete profile exists.
- A global `--auth` override selects a profile per invocation but does not alter config.
- Never retry a User API 401, 403, or hidden-route 404 using the configured Admin API key. Silent privilege escalation is forbidden.

`user_api_username`, scopes, and expiry are local informational metadata. The server is authoritative. The key itself determines the actor; `dsc` must never send `Api-Username` alongside `User-Api-Key`.

### Login and logout state transitions

| Event | Server state | Local state |
|---|---|---|
| First successful login | Creates a key | Merge User API fields into a freshly read entry and set `auth = "user-api"` |
| Successful same-account/client reauthorization | Replaces the same user's previous key for this client ID | Store the new key/scopes/expiry and keep the client ID |
| Successful imported-key or account replacement | Create the new key, verify account intent, then self-revoke the previous key | Store the new profile only after the previous key is no longer an orphan risk |
| Denied or expired device grant | No usable new key | Leave credential fields unchanged; retain a newly provisioned non-secret client ID for retry |
| Authorized response lost | New key may exist and a previous same-user/client key may be invalid | Leave credential fields unchanged, retain the client ID, report uncertain state, and require a fresh login |
| Different known account without `--replace-account` | Revoke the new key | Leave the previous profile unchanged |
| Payload received but config merge/save fails | Attempt self-revocation of the new key | Preserve the prior file; report that the old same-client key may be invalid |
| Normal logout succeeds | Revoke the key | Remove key, username, scopes, and expiry; retain client ID; if selected profile was `user-api`, write `auth = "anonymous"` rather than silently select Admin API |
| Logout receives a response proving the key is invalid, revoked, or expired | No usable key remains | Apply the same local cleanup as successful logout |
| Logout gets a network or server failure | Revocation is unknown | Keep local credentials by default so revocation can be retried; `--local-only` deliberately clears them with a warning |
| Remote revocation succeeds but local save fails | Key is revoked | Leave stale local fields; report that `auth status` will show an unusable profile and retrying logout will clean it |

Before starting browser authorization, `login` preflights config writability, ensures the stable client ID is stored, and records a digest of the target entry. Before saving, it re-reads the file, preserves unrelated changes, and rejects conflicting changes to that entry's auth fields. Atomic replacement alone does not prevent lost updates during a ten-minute approval window. On Unix, the config writer must also sync the parent directory after rename before claiming crash-durable persistence.

An expired key is refused before ordinary requests, but `logout` may still attempt revocation; if authentication rejects it, local cleanup proceeds. `--local-only` never implies server revocation.

### Config secrecy and platform behavior

- Replace direct structured serialization of `DiscourseConfig` with an explicit non-secret listing DTO before adding User API fields. That remediation must also stop the existing `apikey` leak from `dsc list --format json|yaml`.
- Use custom redacted `Debug` implementations or secret-aware field wrappers for all config and runtime types containing either key.
- `auth status`, `config check`, `doctor`, errors, tests, and structured output may report only profile presence and non-secret metadata.
- Continue using the private same-directory config write path. On Unix, preserve 0600 mode, symlink refusal, and atomic replacement. Do not claim Unix permissions or equivalent rename atomicity on Windows; Phase 1 must document and test the supported Windows behavior explicitly.
- `dsc list tidy`, `dsc add`, `dsc.example.toml`, and `docs/configuration.md` must understand optional auth profiles without inserting secret-looking placeholders or reporting every User API field as universally required.

## HTTP client architecture

Replace fixed header construction with an explicit authentication model. The secret type below is conceptual: Phase 1 must select or define a reviewed wrapper with redacted `Debug`, controlled exposure for header/TOML serialization, and zeroization where the underlying library can provide it.

```rust
enum DiscourseAuth {
    Anonymous,
    AdminApi {
        key: SecretValue,
        username: String,
    },
    UserApi {
        key: SecretValue,
        client_id: Option<String>,
        recorded_scopes: Option<BTreeSet<String>>,
        expires_at: Option<DateTime<Utc>>,
    },
}
```

Header behavior:

| Profile | Headers |
|---|---|
| Anonymous | none |
| Admin API | `Api-Key`, `Api-Username` |
| User API | `User-Api-Key`, and `User-Api-Client-Id` when present |

Mark secret `HeaderValue`s sensitive. Tests must assert that only the selected profile's headers are sent, redirects cannot forward them cross-origin, and no credential appears in debug/error/output paths.

User API requests require HTTPS except for an explicit loopback-only test/development path. The normal Admin API profile retains existing URL compatibility in R27; tightening it is a separate change. Authentication protocol requests use dedicated anonymous or User API clients rather than inheriting default headers from the selected ordinary profile.

### Composable command requirements

A single `AuthRequirement` enum cannot describe commands that combine HTTP, SSH, scopes, roles, or multiple source/target legs. Use one declarative source of truth consumed by enforcement, `auth explain`, diagnostics, docs-generation checks, and coverage tests:

```rust
struct CommandRequirements {
    legs: &'static [RequestRequirement],
}

struct RequestRequirement {
    name: &'static str,
    transport: TransportRequirement,
    accepted_profiles: &'static [AuthProfileKind],
    scopes_any_of: &'static [&'static str],
    scopes_all_of: &'static [&'static str],
    role_hint: Option<RoleHint>,
    method: Option<HttpMethod>,
    route: Option<&'static str>,
}
```

Requirements are resolved for the actual command variant and relevant flags, not just the top-level family. A multi-forum command has one leg per source/target. Role hints support explanation and fail-fast checks where known, but they are not a local authorization engine; Discourse remains authoritative.

Every HTTP command variant must have requirement metadata or an explicit test-approved exemption. This replaces ad hoc `ensure_api_credentials` calls and must also become the source for centralized dry-run policy so the two registries cannot drift.

## Candidate command compatibility matrix

This matrix is a Phase 0 hypothesis derived from current `dsc` endpoints and current Discourse source. It MUST be validated live before becoming user-facing documentation. Combined command families must be split into method/route-level metadata during implementation.

| `dsc` surface | Method class | Minimum scope/profile | Expected actor | Notes |
|---|---|---|---|---|
| `version [discourse]` | public `GET` | anonymous or any readable profile | any | `/about.json` is an implementation detail; success does not prove a credential is valid |
| `search` | `GET` | `read` or `write` | regular user | Results remain visibility-filtered |
| `topic pull --full`, `post pull` | `GET` | `read` or `write` | regular user | Visible topics/posts and the user's PMs |
| `category list`, topic-content `category pull` | `GET` | `read` or `write` | regular user | Visible categories/topics only |
| `tag list` | `GET` | `read` or `write` | regular user | Restricted tags remain filtered |
| `notification list` | `GET` | `notifications`, `read`, or `write` | regular user | Narrow notification profile is preferred |
| `notification read` | `PUT` | `notifications` or `write` | regular user | Mutation, not part of the read-only candidate |
| `pm list` | `GET` | `read` or `write` | regular user | Only the key owner's mailbox; current username positional must default to or equal the key owner |
| `pm send` | `POST` | `write` | regular user | Subject to PM/group-recipient rules |
| `topic new`, `topic reply` | `POST` | `write` | regular user | Subject to category/trust/rate limits |
| `topic push/title/tags`, soft `topic delete`, `post push/delete` | write/delete | `write` | owner/editor | Only while the actor may edit/delete that content; `post edit` is only an alias for `post push` |
| `topic delete --purge` | permanent delete | `write` | staff, subject to live validation | Must have separate option-aware requirements and destructive safeguards |
| `upload` | `POST` | `write` | regular user | Subject to type/size/extension limits |
| `invite send/bulk` | `POST` | `write` | permitted inviter | Group/topic options may require additional authority |
| `user info/activity`, `user groups list` | `GET` | `read` or `write` | regular user | Private fields remain filtered |
| visible `group list/info/members` | `GET` | `read` or `write` | regular user | Current admin-first probes need 403 fallback fixes |
| `post move`, deleted-topic list/restore, broad edits | mixed | `write` | moderator/category moderator/TL4 as applicable | Validate each Guardian path |
| `user groups add/remove`, `group add` | write | `write` | group owner/manager or staff | Not self-service group joining; there is no group-side remove command |
| `category def pull/push`, `category show/get/set/diff/rename` | mixed | `read`/`write` | likely admin | Permission fields may be filtered for non-admins |
| settings, tag groups, API-backed themes and `theme palette` | mixed | `read`/`write` | admin | Admin routes appear role-gated; validate live |
| `theme remove`, plugin install/remove | SSH | SSH | SSH operator | Separate from API-backed theme/plugin inspection |
| backups, analytics, staff logs, SAR, custom emoji | mixed | `read`/`write` | admin/staff as applicable | Validate endpoints and response completeness |
| admin users/groups, plugin list | mixed | `read`/`write` | admin/staff as applicable | Validate every mutation live |
| `api-key list/create/revoke` | Admin API by `dsc` policy | `admin-api` | admin | Do not let User API auth mint broader server credentials initially |
| privileged `user create` | `POST` | Server API key | Admin API actor | Current Discourse explicitly branches on `is_api?`; there is no separate `user activate` command |
| `update` | HTTP plus SSH, optional changelog write | per leg | SSH operator plus optional API actor | Not SSH-only; version probes and changelog posting need separate requirements |
| `harden` | SSH | SSH | SSH operator | User API does not replace SSH |
| `backup setup-s3` | Discourse HTTP plus local AWS CLI | per leg | admin plus local AWS credentials | No SSH leg; User API may cover only the HTTP settings portion |
| `add`, `import`, `list`, `open`, completions, man pages, `update log` | no required Discourse auth | none | any | Some perform public HTTP, browser, config, or file I/O; "local" is not one transport requirement |

### Regular-user read candidate

The first useful User API release should focus on:

```text
search
topic pull
post pull
category list/pull
tag list
pm list (self)
notification list
user info/activity
user groups list
visible group list/info/members
version [discourse]
```

### Regular-user write candidate

After read-only support is stable:

```text
topic new/reply
eligible own topic/post edits and deletes
topic title/tags where the user may edit
pm send
upload
invite send/bulk where permitted
notification read
```

`category push` is not in the initial candidate set. It mixes new-topic writes with edits to every existing topic represented by the snapshot and is only complete when the actor can edit them all. R27 must define each bulk command's preflight, partial-success, and rollback contract from actual endpoint behavior; it cannot promise all-or-nothing authorization where Guardian decisions are only known per request.

### Admin User API keys

Current Discourse User API authentication sets `current_user` to the key owner. `Admin::AdminController` checks `ensure_admin`; current source has no general "Admin API keys only" gate for admin controllers. Therefore an admin user with a `read`/`write` User API key is likely able to use many `dsc` admin HTTP commands.

This is not promised behavior until Phase 0 tests it. Known distinctions include:

- `is_api?` remains false, so privileged user creation/activation does not get the Server API fast path.
- Post creation receives ordinary first-post checks and draft behavior.
- The key cannot switch actor using `Api-Username`.
- User API key limits apply in addition to ordinary IP and endpoint/user limits.
- A `write` key for an admin is broad. User approval, expiry, revocation, fixed identity, and separate rate limits improve credential handling, but do not make it a narrowly resource-scoped admin credential.

## Authentication diagnostics

`auth status` is local by default. Text and structured output report the configured profiles, selected profile, completeness, optional username, recorded scopes, expiry, and whether the key is locally expired. It never prints either key. Exit zero means the local configuration is internally usable; ambiguous, partial, unknown, or expired selected profiles exit non-zero.

`auth status --check` performs a profile-aware network probe only when a safe route exists for the recorded scopes. A public `/about.json` success cannot prove credential validity. A `read`, `write`, or `session_info` profile may probe session identity; `notifications` may probe its own list route; a `push`-only key may be reported as locally configured but not safely probeable. The output distinguishes authenticated success, anonymous success, unprobeable scope, invalid credential, and inconclusive permission failure.

`auth explain` is local/advisory and consumes the same resolved command-requirement metadata as enforcement. `--` separates the nested command's flags from `auth explain`. Output includes every leg, method/route where known, selected profile, required any/all scopes, likely role, dry-run support, and known incompatibilities. It cannot prove that Guardian will authorize a particular object.

The nested command supplies its own Discourse argument or source/target arguments; `auth explain` has no separate forum positional that could disagree with it.

`config check` and `doctor` must use the same profile selection and safe probes rather than treating a public `/about.json` 200 as authenticated success. Their structured schemas gain selected-profile and probe-result fields without exposing credentials. Multi-forum checks resolve each entry independently; a global `--auth` override applies the same profile kind to each entry and errors where that profile is unavailable.

## Errors and rate limits

User API errors need auth-aware messages, but status alone is not enough to distinguish a revoked key from Guardian denial. Discourse may also hide protected/admin routes behind 404. Phase 0 must record stable response signatures before the CLI promises finer classification.

| Condition | `dsc` behavior |
|---|---|
| Unsupported device capability | Name the missing header/version and minimum supported flow |
| Invalid/revoked/expired key signature | Explain the evidence and suggest `dsc login <forum>` |
| Missing recorded route scope | Name the required scope and show a reauthorization command containing the union of retained and needed scopes |
| 401/403/hidden-route 404 without a decisive signature | Report the selected profile and explain that credentials, route scope, role, visibility, or object permission may be responsible |
| Device authorization denied | Exit cleanly without writing credentials |
| Device code expired | Offer to restart login; do not reuse key material |
| Authorized poll response lost | Explain one-shot delivery and require a new login |
| Per-minute User API 429 | Honor retry headers within one bounded policy and the enclosing operation deadline |
| Per-day User API 429 | Fail immediately; do not sleep/retry in a loop |

Valid User API requests are charged to additional per-key sliding limits configured by `max_user_api_reqs_per_minute` and `max_user_api_reqs_per_day`, currently defaulting to 20 and 2,880. Ordinary IP and endpoint/user limits may also apply; only authenticated Admin API requests roll back the default IP limiters.

Classify User API key-limit 429 responses using `Discourse-Rate-Limit-Error-Code`: `user_api_key_limiter_60_secs` and `user_api_key_limiter_1_day`. The shared retry layer must classify before sleeping, identify the selected profile in messages, and cover every HTTP method; current direct DELETE paths and the Admin-specific generic 429 hint must be fixed. Bulk pulls and syncs may exhaust User API limits sooner than Admin API operations.

## Key lifecycle and revocation

- Record requested scopes and returned expiry at login; mark them as local metadata.
- Refuse ordinary requests locally when the recorded key has expired.
- Reauthorization with the same client ID destroys the user's previous key for that client. Warn before replacement.
- There is no refresh token. Renewal means another user-approved login.
- Keys may be automatically revoked by Discourse's inactivity cleanup (`revoke_user_api_keys_unused_days`, currently 180 by default) or optional maximum-lifetime cleanup (`revoke_user_api_keys_maxlife_days`, currently disabled at 0).
- A logout 403 is not an authoritative "already revoked" result; it can mean invalid, revoked, expired, or otherwise rejected. Apply local cleanup only when the response proves the key cannot be used, otherwise retain it unless `--local-only` was explicit.

## Security requirements

- Generate RSA private keys locally with the OS CSPRNG; never use an online key generator or persist the private key after login.
- Request OAEP and pin SHA-1/MGF1-SHA-1/empty-label interoperability in fixtures while current Discourse uses those OpenSSL defaults.
- Verify the returned nonce before accepting the decrypted key.
- Enforce explicit protocol response and encrypted-payload size limits.
- Require HTTPS and same-origin verification/redirect handling for User API flows, with only an explicit loopback test exception.
- Never log keys, private key material, encrypted/decrypted payloads, nonces, `device_code`, or request tokens; intentional one-time display of the verification URL and `user_code` is the only exception.
- Store the User API key only through `dsc`'s private config-write path and preserve concurrent unrelated config edits.
- Keep `User-Api-Client-Id` non-secret but stable per `dsc` installation/profile.
- Prevent mixed Admin/User auth headers on every request and mark secret header values sensitive.
- Treat `write` for an admin user as a high-privilege credential in docs and CLI confirmation copy.
- Do not implement automatic Admin API fallback.
- Ensure every text, JSON, YAML, debug, error, diagnostic, and list path is tested for secret absence.

## Phases

### Phase 0 - compatibility and threat-model spike (blocking)

- [x] Complete R36's disposable, serial, cleanup-safe live-test isolation first.
- [ ] Enable User API keys/scopes on the disposable demo forum.
- [ ] Create disposable regular-user, moderator/category-moderator, group-manager, and admin accounts.
- [ ] Probe both capability headers and complete the device flow manually with JSON request bodies.
- [ ] Record actual endpoint results, methods, routes, response signatures, and role behavior for every candidate command family.
- [ ] Catalogue controller behavior explicitly conditional on `is_api?` or `is_user_api?`.
- [ ] Verify admin-route access under an admin User API key.
- [ ] Record minute/day rate-limit headers, bodies, ordinary-limit interaction, and safe probe routes per scope.
- [ ] Confirm protocol size/deadline bounds and same-origin/redirect behavior.
- [ ] Decide the minimum supported Discourse auth API version and whether legacy callback-flow support is warranted.
- [ ] Update the matrix from observed requests/responses and pin the tested Discourse version/commit before Phase 1.

### Phase 1 - authentication plus read-only surface

- [ ] Verify the independent structured `dsc list` credential-exposure fix remains in place before adding User API fields.
- [ ] Add additive User API config fields, profile validation, selection, and concurrent-safe targeted writes.
- [ ] Add `dsc login`, `logout`, `auth status`, and `auth explain`, including dry-run and structured output contracts.
- [ ] Implement device authorization, exact JSON/OAEP interoperability, nonce verification, one-shot recovery handling, expiry, and revocation.
- [ ] Refactor `DiscourseClient` to explicit Anonymous/Admin/User profiles with HTTPS/origin/redirect safeguards.
- [ ] Make config replacement crash-durable on Unix by syncing the parent directory after rename; specify equivalent supported Windows behavior.
- [ ] Replace fixed credential guards with composable per-leg requirement metadata shared by enforcement and diagnostics.
- [ ] Integrate profile-aware checks with `config check` and `doctor`.
- [ ] Enable and document the confirmed regular-user read candidate commands.
- [ ] Add auth-aware 401/403/404/429 errors and key-limit classification across all HTTP methods.
- [ ] Update add/tidy/example config, configuration docs, README command index, completions, and man pages.
- [ ] Add offline crypto/protocol fixtures and opt-in serialized live tests.

### Phase 2 - ordinary user writes

- [ ] Enable confirmed own-content writes, PM send, uploads, notifications, and permitted invites.
- [ ] Add tests for ownership, edit-window, visibility, and partial-failure behavior.
- [ ] Specify each bulk workflow's authority preflight, partial-success output, and rollback behavior from observed API capabilities.
- [ ] Document every command as "own/eligible content", not broadly writable.

### Phase 3 - elevated roles (demand-driven)

- [ ] Enable only the moderator, group-manager, and admin command families proven in Phase 0 live tests.
- [ ] Keep `dsc`-policy Admin-API-only commands explicit.
- [ ] Add mixed-profile source/target selectors only for a concrete multi-forum need.
- [ ] Add multi-forum tests with different auth profiles per Discourse.

### Phase 4 - specialized scopes (only on real demand)

- [ ] Message bus streaming.
- [ ] Push callback/daemon behavior.
- [ ] User-status commands.
- [ ] Calendar subscriptions.
- [ ] Legacy/one-time-password flow, if a concrete consumer requires it.

## Test plan

Offline unit and integration tests:

- Header selection for Anonymous/Admin/User profiles; assert no mixed or cross-origin-forwarded headers.
- Config backward compatibility, unknown/partial profile handling, ambiguity refusal, and per-invocation overrides.
- `dsc list`, `auth status`, diagnostics, debug, and errors never expose Admin or User API keys in text/JSON/YAML.
- Config mode/symlink behavior, concurrent unrelated edits, conflicting target edits, and save failure after authorization.
- Stable client-ID persistence across denied/interrupted first login, imported-key migration, unknown-scope replacement refusal, old-key revocation, and account-mismatch handling.
- Device request/response parsing, JSON MIME requirements, capability headers, every poll status, first-poll timing, monotonic deadline, and 429 handling with injected clock/sleeper behavior.
- RSA OAEP decryption with fixed SHA-1/MGF1-SHA-1/empty-label fixtures and nonce mismatch refusal.
- Response-size, invalid URL/origin, HTTPS, redirect, interval, and expiry-bound refusal.
- Login/logout state transitions, dry-run behavior, expiry, local-only logout, and one-shot authorized-response loss.
- Scope normalization and additive-versus-replacement reauthorization.
- Requirement/error messaging for every command variant, including source/target legs and option-dependent routes.
- User API per-minute and per-day 429 classification across GET/POST/PUT/PATCH/DELETE.
- Parser/help/examples, completion regeneration, man pages, and structured output schemas for all new commands.

Use the current test infrastructure: `run_dsc` with temporary config files, `DSC_LIVE_TESTS` and `TEST_DSC_CONFIG` for opt-in live tests, RAII cleanup guards, and parser/policy unit tests in `src/cli.rs`. Do not add a test-serialization dependency until R36 defines the repository-wide live-test mechanism.

Live tests (explicit opt-in, disposable demo, serial, cleanup-safe):

- Device authorization for each test role.
- Regular read visibility filtering.
- Own topic/post create/edit/delete and forbidden edits of another user's content.
- PM isolation and notification ownership.
- Moderator/category-moderator operations.
- Group-manager membership operations.
- Admin endpoint matrix and known `is_api?` exceptions.
- Minute/day limit signatures, revoked/expired credentials, and ambiguous failures.
- Cleanup all disposable content and revoke generated keys.

Do not run device-login tests in ordinary CI; authorization requires explicit human approval. Protocol and crypto behavior must have complete offline fixtures.

## Backward compatibility

- Existing `apikey`/`api_username` configs and commands retain current behavior.
- Existing Admin API authentication remains the default when it is the only complete profile.
- Existing anonymous-capable HTTP commands remain anonymous when no profile is complete and no explicit selector is present.
- Existing Admin API URL compatibility is not tightened by the User API HTTPS requirement.
- No existing command is renamed in the first phase.
- `dsc api-key` remains the Admin API-key-management command. A future visible alias `admin-key` may clarify terminology, but removing/renaming `api-key` is out of scope and would require a compatibility plan.
- User API support is additive and opt-in.
- A command that works with Admin API today must not silently become less privileged after adding User API fields.
- Logout never silently selects an existing Admin API profile after removing a selected User API key.

## Open questions

1. Should Phase 1 require device-code support or also implement the legacy redirect callback flow?
2. What expiry should `dsc login` request by default: a `dsc`-defined duration such as 30 days, or no explicit expiry? Omitting `expires_in_seconds` creates no explicit expiry; the site's maximum-expiry setting only caps requested durations.
3. Should one Discourse entry support multiple User API identities, or exactly one user profile plus one Admin API profile initially?
4. Which admin HTTP commands should `dsc` intentionally keep Admin-API-only even when Discourse technically permits an admin User API key?
5. Should command requirement metadata be exposed as machine-readable JSON for agents and wrappers?
6. Are User API rate limits sufficient for category/topic bulk pulls, or should those commands estimate/request a budget before starting?
7. What explicit upper bounds for device grant lifetime and poll interval are compatible with supported live Discourse versions?

## Out of scope

- Standard OAuth/OIDC support; Discourse User API keys are a separate protocol.
- Storing secrets in an OS keychain or external secret manager.
- Bypassing Guardian, trust levels, category visibility, edit windows, or site rate limits.
- Impersonating another user or honoring `Api-Username` under User API auth.
- Automatically upgrading from User API to Admin API credentials.
- Replacing SSH, AWS, or Docker authentication.
- Registering a push URL or running a background notification daemon in the initial phases.
- Implementing new content/admin commands solely to demonstrate User API auth.
- Mixed-profile multi-forum mutations until a concrete command requires independent source/target selectors.

## Reference: current Discourse protocol/source

Reviewed on 2026-07-27 against Discourse commit [`76e5cf8b709e2bd9d308648a35971e52d8248403`](https://github.com/discourse/discourse/tree/76e5cf8b709e2bd9d308648a35971e52d8248403):

- [User API keys specification](https://meta.discourse.org/t/user-api-keys-specification/48536)
- [`app/models/user_api_key_scope.rb`](https://github.com/discourse/discourse/blob/76e5cf8b709e2bd9d308648a35971e52d8248403/app/models/user_api_key_scope.rb) - built-in scope route matchers
- [`lib/auth/default_current_user_provider.rb`](https://github.com/discourse/discourse/blob/76e5cf8b709e2bd9d308648a35971e52d8248403/lib/auth/default_current_user_provider.rb) - key lookup, current-user binding, `is_api?` versus `is_user_api?`, and rate limits
- [`app/controllers/user_api_keys_controller.rb`](https://github.com/discourse/discourse/blob/76e5cf8b709e2bd9d308648a35971e52d8248403/app/controllers/user_api_keys_controller.rb) - capability headers and authorization/device endpoints
- [`app/services/user_api_key/device_auth.rb`](https://github.com/discourse/discourse/blob/76e5cf8b709e2bd9d308648a35971e52d8248403/app/services/user_api_key/device_auth.rb) - device flow constants and contract
- [`app/services/user_api_key/device_auth/crypto.rb`](https://github.com/discourse/discourse/blob/76e5cf8b709e2bd9d308648a35971e52d8248403/app/services/user_api_key/device_auth/crypto.rb) - RSA parsing, payload-size calculation, and OAEP behavior
- [`app/services/user_api_key/device_auth/request_validator.rb`](https://github.com/discourse/discourse/blob/76e5cf8b709e2bd9d308648a35971e52d8248403/app/services/user_api_key/device_auth/request_validator.rb) - RSA key-size and request validation
- [`app/services/user_api_key/device_auth/grant_store.rb`](https://github.com/discourse/discourse/blob/76e5cf8b709e2bd9d308648a35971e52d8248403/app/services/user_api_key/device_auth/grant_store.rb) - one-shot authorized-payload storage
- [`app/services/user_api_key/device_auth/payload_builder.rb`](https://github.com/discourse/discourse/blob/76e5cf8b709e2bd9d308648a35971e52d8248403/app/services/user_api_key/device_auth/payload_builder.rb) - encrypted payload fields
- [`app/services/user_api_key/expiry.rb`](https://github.com/discourse/discourse/blob/76e5cf8b709e2bd9d308648a35971e52d8248403/app/services/user_api_key/expiry.rb) - requested expiry semantics
- [`config/routes.rb`](https://github.com/discourse/discourse/blob/76e5cf8b709e2bd9d308648a35971e52d8248403/config/routes.rb) - `/user-api-key/*` routes
- [`app/controllers/admin/admin_controller.rb`](https://github.com/discourse/discourse/blob/76e5cf8b709e2bd9d308648a35971e52d8248403/app/controllers/admin/admin_controller.rb) - admin role gate

The general Discourse REST API is not independently versioned. User API authorization exposes `Auth-Api-Version` and capability headers, but protocol changes ship as part of Discourse releases. Phase 0 must pin the tested Discourse version/commit, auth API version, advertised capabilities, and redacted request/response examples.

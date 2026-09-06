# `dsc upcoming-change` and `dsc setting upload`

> **Status: proposed for R59.** Discovery completed on 3 September 2026 against current Discourse source and a 16-forum fleet audit, then revalidated on 6 September 2026 against Discourse commit [`a6ade3e`](https://github.com/discourse/discourse/commit/a6ade3e9915f427d115f082a2ddbc6c208c5f3c1). No implementation exists yet.

## Problem

Two supported Discourse administration workflows are not representable safely in `dsc` 0.18.0, and the adjacent upload transport needs correction:

1. Hidden Upcoming Changes cannot be listed, inspected, enabled, or disabled. `dsc setting get` cannot retrieve them because they are intentionally absent from the ordinary site-settings API.
2. `dsc upload` creates composer-style uploads but cannot perform the validated upload-and-bind workflow required by upload-type site settings such as `llms_txt`, logos, and favicons.
3. The existing upload client still sends Discourse's deprecated multipart field `type`; supported Discourse versions require its replacement, `upload_type`.

The immediate driver is core Discourse's `enable_generated_llms_txt` Upcoming Change and the need to replace a third-party LLM-index plugin with reviewed native custom files. The commands must remain general rather than embedding `llms.txt` policy in `dsc`.

## Goals

- Expose the supported Upcoming Changes admin API without raw-request escape hatches.
- Make enable and disable operations explicit, idempotent, dry-runnable, and post-verified.
- Upload a local file in site-setting context, bind its returned raw URL to a validated upload-type setting, and verify the binding.
- Move the existing generic upload client to the supported `upload_type` request field without changing the `dsc upload` CLI.
- Preserve standard `dsc` output, authentication, retry, redaction, fleet-safety, and error conventions.

## Out of scope

- Arbitrary HTTP requests.
- Rails-console access.
- A generic hidden-site-setting bypass.
- Automatic editorial generation of `llms.txt` files.
- Cross-forum copying of upload URLs.
- Deleting old or orphaned uploads.
- Fleet mutation in the first phase.
- Changing Upcoming Change group targeting; enable and disable preserve any existing group scope.

## Command Surface

```text
dsc upcoming-change list <discourse> [--format text|json|yaml] [--verbose]
dsc upcoming-change show <discourse> <setting-name> [--format text|json|yaml]
dsc upcoming-change enable <discourse> <setting-name> [--format text|json|yaml]
dsc upcoming-change disable <discourse> <setting-name> [--format text|json|yaml]

dsc setting upload <discourse> <setting> <file> [--format text|json|yaml]
```

All mutation commands honor global `--dry-run`. `upcoming-change` has the visible alias `upcoming`; `show` remains the canonical detail verb. Do not expose a literal `toggle` command because toggling relative to stale state is unsafe and non-idempotent. The API route is named `toggle`, but the CLI must send the caller's explicit target state.

Existing `dsc setting set <discourse> <setting> ""` remains the way to clear an upload setting. A separate `setting upload --unset` mode is unnecessary.

## Discourse API

### Upcoming Changes

List:

```http
GET /admin/config/upcoming-changes.json
```

Set explicit state:

```http
PUT /admin/config/upcoming-changes/toggle.json
Content-Type: application/x-www-form-urlencoded

setting_name=enable_generated_llms_txt&enabled=true
```

The server handles administrator authorization, existence validation, allowed-target validation, staff-action logging, audit events, and Discourse events. The list response reports dependency state, but the toggle service does not reject an enable request merely because `depends_on_met` is false. `dsc` must report that condition clearly and must not attempt a direct site-setting update.

The list controller responds only to an XHR request. Send `X-Requested-With: XMLHttpRequest` and parse the top-level `upcoming_changes` array.

### Upload and bind

Upload:

```http
POST /uploads.json
Content-Type: multipart/form-data

file=@llms.txt
upload_type=site_setting
for_site_setting=true
site_setting_name=llms_txt
synchronous=true
```

Bind the returned raw `url`:

```http
PUT /admin/site_settings/llms_txt.json
Content-Type: application/x-www-form-urlencoded

llms_txt=/uploads/default/original/.../llms.txt
```

The setting value is the upload URL, not the upload ID or `upload://` short URL. Supplying `site_setting_name` during upload allows Discourse's `UploadValidator` to apply setting-specific type and extension rules.

`upload_type` is the supported multipart field. Discourse deprecated the old `type` field in 3.4 with removal targeted for 3.5. Phase 3 must update the shared upload client to send `upload_type` for both `dsc upload` and `dsc setting upload`; no fallback to `type` is required under `dsc`'s current-stable-only Discourse compatibility policy.

## Data Model

Use a typed representation for the fields returned by the list API. Preserve unknown JSON fields through a flattened map only if the current API response contains material not yet modeled; do not reduce the result to name and boolean alone.

Structured output should preserve the list API's shape. A representative entry is:

```yaml
setting: enable_generated_llms_txt
humanized_name: Enable generated llms txt
description: Generates a concise /llms.txt when no custom file is uploaded.
value: true
upcoming_change:
  status: beta
  impact: feature,all_members
  impact_type: feature
  impact_role: all_members
  enabled_for: everyone
plugin: null
depends_on: null
depends_on_humanized_names: null
dependents: []
depends_on_met: true
overriding_defaults: true
groups: null
```

The final field names must follow the captured API response exactly. The API's `value` is the effective state displayed by Discourse. `overriding_defaults` means the Upcoming Change controls one or more separate site-setting default overrides; it does not indicate effective state or automatic promotion. Do not reconstruct state from the hidden setting's declared default because `promote_upcoming_changes_on_status` can promote beta changes and an administrator can opt out.

`setting upload` structured output should include:

```yaml
discourse: example
setting: llms_txt
previous_value: null
upload_id: 123
upload_url: /uploads/default/original/1X/example.txt
value: /uploads/default/original/1X/example.txt
status: updated
```

Text output should be concise and must not print file content.

## Read Behavior

`list` returns all Upcoming Changes in the server's stable setting-name order. Normal text output shows setting name, status, effective state, and enabled-for scope. `--verbose` adds impact, dependencies, target groups, plugin ownership, default-override status, and learn-more URL where present.

`show` performs one list request and selects an exact `setting_name`. It fails clearly if the change is absent, distinguishing:

- The selected forum does not expose the Upcoming Changes API.
- The API exists but the named change is absent on this Discourse commit.
- Authentication lacks administrator permission.

Do not turn an endpoint 404 into a claim that the individual setting is disabled.

## Mutation Behavior

### Upcoming Change enable and disable

1. Fetch current state.
2. Validate that the exact change exists. If enabling while `depends_on_met=false`, stop with a clear error rather than creating a knowingly ineffective or unsupported combination; do not automatically enable dependencies.
3. If already in the requested effective state, print `unchanged` and make no PUT request.
4. Under `--dry-run`, print current state, current `enabled_for`/group scope, requested target, API route, and whether a mutation would occur; make no PUT request.
5. Send the explicit boolean to the supported toggle endpoint.
6. Re-fetch and require the effective state to equal the target; report the resulting `enabled_for`/group scope.
7. Return non-zero if verification fails, even if the PUT succeeded.

The server validates the effective target when enabling and records the change. An existing `SiteSettingGroup` record can cause enablement to resume its prior staff or specific-group scope rather than enabling for everyone; `dsc` must preserve and disclose that scope, not silently broaden it. If no group scope exists and the change disallows an `everyone` target, surface the server refusal with exact bounded error context. Dependency state is exposed by the list API but not enforced by the toggle service, so the client-side refusal above is an intentional safety guard.

### Setting upload

1. Validate the local path, regular-file status, non-empty content, and readable metadata before making a request.
2. Fetch the setting metadata from the ordinary admin site-settings catalogue and require an exact visible entry of type `upload`. Hidden and non-configurable settings are absent from this catalogue; reject absent, unknown, or non-upload settings before uploading. If the response exposes deprecation metadata, reject hard-deprecated settings too; the bind endpoint remains authoritative for deprecation, global-shadowing, visibility, and configurability policy.
3. Record the previous value for output and rollback guidance.
4. Under `--dry-run`, report the local file, size, setting, previous value, upload request context, and intended bind/verification steps. Do not upload.
5. Upload with `upload_type=site_setting`, `for_site_setting=true`, and `site_setting_name=<setting>`.
6. Require a non-empty raw `url` and upload ID in the response.
7. Compare the canonical returned URL with the current setting. If they already match, skip the PUT and report `unchanged`; Discourse may reuse an existing upload by digest.
8. Otherwise update the setting using the returned raw URL.
9. Re-fetch the setting and require it to resolve to the new upload URL.
10. Emit the previous value and new upload details in structured output.

If upload succeeds but binding fails, re-fetch when possible, return non-zero, and report the new upload ID/URL plus the observed or uncertain setting state. If binding succeeds but verification differs, report the previous, requested, and observed values without automatically rolling back over a possible concurrent administrator change. Do not delete the upload or hide the artifact. The standard retry helper may retry the upload only after an explicit HTTP 429, where Discourse has rejected the attempt before processing it; do not retry after an ambiguous transport failure. The operator can rerun after checking current state; a subsequent implementation may add checksum-aware upload reuse if a real need appears.

## Safety and Privacy

- Require administrator API credentials before any request.
- Apply existing capped-response and retry helpers.
- Retry multipart uploads only after an explicit HTTP 429, never after an ambiguous transport failure.
- Never print uploaded file content.
- Do not classify an arbitrary upload URL from another forum as portable.
- Preserve the previous setting URL in mutation output so rollback is explicit.
- Avoid a fleet mutation mode in Phase 1. Curated files are forum-specific and Upcoming Changes should be canaried before fleet expansion.
- Keep standard `dsc` redaction rules even though the known driver fields are not secrets.

## Error Cases

- Upcoming Changes endpoint unavailable on an older Discourse commit.
- Named Upcoming Change absent, retired, promoted to a normal setting, or not eligible for the requested target.
- Client refuses enable because the list response reports `depends_on_met=false`.
- Server rejects enable because the resulting target is not allowed.
- Caller is not an administrator.
- Setting is not type `upload`.
- File extension or MIME type is invalid for the selected setting.
- Upload succeeds but setting update fails.
- Setting update succeeds but post-verification differs because of serialization, CDN URL rewriting, or a concurrent administrator change.

For URL verification, compare the canonical setting value returned by Discourse rather than assuming byte-for-byte equivalence with a CDN-decorated upload response. Discovery tests must establish the exact response representation before implementation.

## Tests

### Unit and HTTP fixture tests

- Parse a representative Upcoming Changes list with enabled, disabled, promoted, dependency-constrained, and target-constrained entries.
- Exact-name selection and absent-name diagnostics.
- Explicit true/false form payloads to `/admin/config/upcoming-changes/toggle.json`.
- No PUT when current effective state already matches.
- No network mutation under `--dry-run`.
- Post-write state mismatch returns non-zero.
- Upload multipart fields include `upload_type=site_setting`, `for_site_setting=true`, and exact `site_setting_name`.
- Existing `dsc upload` also sends `upload_type` rather than deprecated `type`, with no CLI or output-shape change.
- Upload setting receives the raw URL under a form field named after the setting.
- A reused upload URL that already matches the setting skips the bind PUT and reports `unchanged`.
- Non-upload setting rejected before upload.
- Upload success followed by bind failure reports the orphan upload safely.
- Text, JSON, and YAML output contracts.

### Live compatibility tests

Use a resettable disposable forum and a harmless reversible Upcoming Change. Do not toggle a fleet production feature merely to test the client. Toggling creates durable audit records, so this is a manual compatibility capture rather than a standard persistent-forum test unless the fixture can restore the forum snapshot afterward.

For upload binding, use a disposable upload-type test setting or a test forum's non-critical logo setting:

1. Capture the old value.
2. Dry-run and assert no change.
3. Upload and bind a valid fixture.
4. Verify the setting and served asset.
5. Restore the old value.
6. Reset or destroy the disposable forum so the upload, user-upload association, and audit artifacts do not persist.

Do not add this upload mutation to the standard `s/test-live` suite until a prearmable cleanup mechanism exists. Restoring the setting alone does not delete the created upload and therefore does not satisfy the repository's live-test cleanup contract. The `llms_txt` rollout itself is production verification, not a compatibility test fixture.

## Documentation

- Add `docs/upcoming-change.md` with supported-version behavior and explicit enable/disable examples.
- Extend `docs/setting.md` with `setting upload`, rollback using the previous URL, and the cross-forum URL limitation.
- Correct `docs/upload.md` and any theme documentation that currently implies `dsc upload` plus ordinary `setting set` is a complete validated site-setting workflow.
- Include both surfaces in generated man pages and shell completions.

## Implementation Phases

| Phase | Deliverable | Effort |
| --- | --- | --- |
| 1 | Upcoming Change list/show and typed API response | Small |
| 2 | Explicit enable/disable with dry-run and post-verification | Small |
| 3 | `setting upload` validated upload, binding, output, and failure recovery | Medium |
| 4 | Resettable-forum compatibility capture and documentation; automate only after cleanup is prearmable | Small |

## Backward compatibility

- `upcoming-change` and `setting upload` are additive command surfaces.
- Existing `dsc upload <discourse> <file> [--upload-type ...]` syntax and output remain unchanged; only its multipart field moves from deprecated `type` to supported `upload_type`.
- Existing `setting set` remains available for direct URL binding and for clearing an upload setting with an empty string.
- Older Discourse releases that lack Upcoming Changes return the explicit capability-unavailable diagnostic described above. `dsc` supports current upstream stable Discourse only, so no legacy endpoint or deprecated upload-field fallback is introduced.

## Sources

API behavior above was revalidated against Discourse commit [`a6ade3e`](https://github.com/discourse/discourse/commit/a6ade3e9915f427d115f082a2ddbc6c208c5f3c1). Reconfirm these unversioned admin endpoints against the supported Discourse release during implementation.

- `app/controllers/admin/config/upcoming_changes_controller.rb`
- `app/services/upcoming_changes/list.rb`
- `app/services/upcoming_changes/toggle.rb`
- `spec/requests/admin/config/upcoming_changes_controller_spec.rb`
- `app/controllers/uploads_controller.rb`
- `lib/upload_creator.rb`
- `lib/validators/upload_validator.rb`
- `spec/requests/uploads_controller_spec.rb`
- `spec/requests/admin/site_settings_controller_spec.rb`
- [Generated `llms.txt` feature commit](https://github.com/discourse/discourse/commit/6bd7e5739561231078531073a3b83c257acff2b8)

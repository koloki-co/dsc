# Email branding template management and previews

Status: proposed, R78. Field driver: York Music, 3 November 2026.

## Motivation

An operator needs outgoing email to match a forum's branding, retain the previous template for rollback, review an exact diff before changing production, and inspect locally rendered output afterwards. `dsc setting` already manages the public email accent and link settings. It cannot currently inspect or round-trip the full email HTML/CSS customization or obtain an email preview. This matters for reproducible branding across managed forums, independently of interactive forum participation.

## Current state

Observed with `dsc 0.23.0` on 2026-11-03:

- `dsc email --help` fails with an unrecognized-subcommand error. Top-level help has no dedicated email branding or preview command.
- `dsc setting get` and curated `setting push --dry-run` / `setting push` successfully manage `email_accent_bg_color`, `email_accent_fg_color`, and `email_link_color`.
- Live target version: Discourse `2026.11.0-latest`, commit `0efb9ba588278a46fe116fe5229e466350f2cda8`.
- Native accent/link colour writes were verified by fresh reads and a subsequent zero-change push plan. That verifies configuration, not rendered email appearance.
- A browser without an authenticated admin session could not inspect the private email editor. No full-template API workaround, preview request, or test-email delivery was performed.

Basic colour branding is therefore supported today. Full HTML/CSS state, safe compilation, reset semantics, and rendered previews are the missing capabilities. Do not describe the whole task as blocked or duplicate the existing setting implementation.

## Proposed CLI surface

```console
dsc email-style show <discourse> --format json
dsc email-style pull <discourse> <directory>
dsc email-style diff <discourse> <directory>
dsc email-style push <discourse> <directory> --dry-run
dsc email-style push <discourse> <directory>
dsc email-style reset <discourse> --field css --dry-run
dsc email-style reset <discourse> --field css --yes
dsc email-style preview <discourse> --username <test-user> --since <timestamp> --output <file.html>
```

- `show` reports effective/default HTML and CSS with explicit provenance and text/JSON/YAML output.
- `pull` persists a versioned manifest plus separate HTML/CSS files. Record safe site identity, tool/server version, and effective/default values; distinguish defaults from overrides only where the API supplies evidence. Never infer override presence merely from equal values.
- `diff` compares intended content against freshly fetched state, with no mutation.
- `push` uses the supported email-style updater, preserving the server's HTML placeholder validation and CSS compilation. Do not write hidden `email_custom_css` directly through a generic setting call: compiled CSS is related state owned by the updater.
- `push --dry-run` constructs the same plan as apply, reports changed fields and the target, and performs no PUT. An unchanged input produces no PUT. Validate required placeholders locally and surface server validation errors without claiming success.
- `reset` resets explicitly selected fields using their server-reported defaults, supports dry-run, requires confirmation, and verifies the resulting state. There is no assumed DELETE endpoint.
- Re-fetch after mutation and compare exact HTML/CSS with the intended state. Preserve unselected fields; prevent stale snapshots silently overwriting concurrent edits.
- Colour settings stay under `dsc setting`; document the curated-settings-file workflow alongside this command rather than introducing competing state ownership.

## API evidence and discovery boundary

The following contracts are derived from the local Discourse source, not captured live requests/responses. Verify them against the target release before implementation. No authentication headers, credentials, or private rendered content belong in this spec.

### Email style

Routes in `config/routes.rb` and `Admin::EmailStylesController` indicate:

```http
GET /admin/customize/email_style.json
PUT /admin/customize/email_style.json
Content-Type: application/json

{"email_style":{"html":"<html>...%{email_content}...</html>","css":"/* reviewed CSS */"}}
```

`EmailStyleSerializer` exposes `id`, `html`, `css`, `default_html`, and `default_css`. Capture the actual live JSON root and exact errors during discovery; do not invent a successful response fixture from the serializer alone.

`EmailStyleUpdater` requires `%{email_content}` in any submitted HTML, compiles submitted CSS with SassC, and removes overrides when fields equal their defaults. CSS updates also maintain `email_custom_css_compiled`. Default HTML comes from `app/views/email/default_template.html`; retain other runtime placeholders when editing. Invalid HTML or CSS returns HTTP 422 from the controller.

### Preview

`Admin::EmailController#preview_digest` and the email routes indicate:

```http
GET /admin/email/preview-digest.json?username=<explicit-test-user>&last_seen_at=<timestamp>
```

Source-derived response fields are `html_content` and `text_content`. This is a user-specific digest renderer, not a generic transactional-email renderer. Establish handling of empty/skipped digests and exact response shape through live discovery. Require an explicit user and time window; use an approved test user because output can include private content and tokenized links. Export privately and never print full preview bodies into routine logs.

Preview must never send email. Mail delivery endpoints such as `test` and `send_digest` are separate mutating operations, outside the initial scope. Transactional sample previews require independent API discovery and must not be promised from this digest endpoint.

## Delivery phases and acceptance

### Phase 1 - template management

- [ ] Capture live, redacted style GET/PUT responses and invalid-template/invalid-CSS errors on an explicitly authorized disposable target.
- [ ] Add stable file schema, show/pull/diff/push/reset, help, and operator documentation.
- [ ] Verify preservation of `%{email_content}`, other placeholders, untouched fields, server default handling, and CSS compilation.
- [ ] Test no-op zero writes, dry-run zero writes and plan fidelity, stale-state refusal, validation errors, post-write mismatch, and rollback.

### Phase 2 - rendered verification

- [ ] Discover and implement bounded, read-only digest preview export with private output permissions and explicit test-user scope.
- [ ] Document browser inspection at narrow/desktop widths, link/button contrast, images, and email-client limitations. Preview in a browser is not Outlook/Gmail delivery verification.
- [ ] Investigate a supported non-delivering transactional preview only if an admin API exists.

## Backward compatibility and scope

This is additive. Existing `setting` commands and snapshots remain supported. Do not automatically copy website colours: email has independent contrast requirements. Exclude SMTP/DNS changes, member export, fleet-wide template writes, automatic test delivery, theme-source authoring, and replacement of Discourse's renderer.

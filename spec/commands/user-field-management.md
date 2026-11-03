# `dsc user-field` - custom user-field definition management

Roadmap: R68. Status: Phase 2 create-only workflow implemented on 2026-11-03. API list/show/create/delete compatibility was verified against `koloki-demo`; one synthetic discovery field was created and deleted, leaving the forum empty. Existing-field update remains deliberately unsupported because option replacement is destructive. This spec retains the desired guarded update workflow.

## Motivation and scope

Numun needs Countries, Regions and Movements as required signup multiselect fields, populated from client-reviewed lists including a long country list. An existing Strategies field must be left untouched. Operators need to review, create, export and maintain these definitions without entering each option individually. The unit of management is a forum-wide **field definition**, not individual users' answers. A newline-per-option source can be converted into a reviewed YAML or JSON list outside `dsc`; native ODS parsing is not required.

## Proposed workflow

```console
dsc user-field list <discourse> --format yaml
dsc user-field show <discourse> <id> --format yaml
dsc user-field pull <discourse> user-fields.yaml
# Edit a curated file containing only the definitions to manage.
dsc user-field diff <discourse> user-fields.yaml
dsc user-field push --dry-run <discourse> user-fields.yaml
dsc user-field push <discourse> user-fields.yaml
```

`list` and `show` support text, JSON and YAML. `pull` writes definitions only, never user answers, in YAML or JSON by extension. `diff` and dry-run fetch current state and use the exact same planner as push. Display every proposed create and each old-to-new change, including ordered option additions, removals and reordering, visibility and requirement changes. Honour the global dry-run without any mutation. `push` is idempotent and curated: omitted definitions are never modified or deleted. A failed read, ambiguous identity, unsupported change or invalid input aborts before any write. Apply sequentially, stop on first failure, report applied and outstanding actions, and re-read each changed definition; fail if the confirmed state differs from the plan. Multi-field operations are not atomic.

### File shape and identity

The implementation must version its file schema and document source-site identity without secrets. Illustrative shape, with syntax subject to implementation and compatibility discovery:

```yaml
version: 1
source: https://example.invalid
user_fields:
  - name: Countries
    description: Which countries do you work in? Select all that apply.
    field_type: multiselect
    requirement: on_signup
    editable: true
    show_on_signup: true
    show_on_profile: false
    show_on_user_card: false
    searchable: false
    options:
      - Example country A
      - Example country B
```

Pulled entries include their forum-local numeric ID; new entries omit it. An ID is valid only for the source forum identified in the snapshot: refuse an ID from a different or unknown origin rather than interpreting it on the target. For a new entry without an ID, create only if no live field has the same exact name; if one or more match, refuse with a clear instruction to pull and identify the existing field. An ID must resolve uniquely; missing IDs, duplicate file IDs/names, or a rename colliding with another live name fail closed. Never silently fall back from an ID to a name or create a replacement for a missing ID. Intentional cross-forum copying requires an explicit operator step to remove IDs and set the target origin; existing same-named target fields still require deliberate reconciliation. IDs keep renames attached to the existing field, but do not promise preservation of answer semantics when options or types change.

Validate the complete file before writes: recognized properties and types, nonempty names and descriptions where applicable, valid requirement and type values, explicit booleans, and unique nonblank options for dropdown/multiselect fields. Preserve Unicode, punctuation and ordering exactly; do not silently normalize names, deduplicate or reclassify client-supplied options. Reject contradictory input rather than relying on Discourse's UI-only constraints. The set of available field types and plugin-added properties can vary by server version; unknown data must not be silently lost or overwritten. Define safe handling during discovery, and refuse writes when unsupported state prevents a faithful plan.

## Safety and data implications

- Existing options are ordered values, not stable cross-site IDs. Option removal, renaming, replacement and field-type changes can affect existing answers; do not claim that simply recreating options or rolling back a definition restores user data. Discovery must establish exact behaviour using disposable test accounts before allowing these changes. Until then, refuse potentially destructive option edits and type changes. Additions must also be checked because upstream replaces option records for any nonempty submitted list.
- Require a distinct explicit confirmation for any supported destructive option or type change, with a clear plan and warning. Do not use a blanket `--yes` to bypass an unimplemented or unknown safety check. Preserve all unchanged options on additions; do not send options at all for a scalar-only edit.
- No implicit pruning or deletion. If an explicit `user-field delete` is implemented, require an exact ID, a live-state plan, explicit confirmation consistent with CLI policy, and verification; document answer retention and recoverability only once tested. Delete is not a prerequisite for the Numun creation workflow.
- Inspect a fresh state immediately before applying, detect drift from the planned state, and refuse stale updates. No ordinary read-then-write client check is atomic; document the remaining race if the server offers no conditional write.
- Requirement `for_all_users` can affect existing users. Show that impact in the plan and require explicit confirmation; do not conflate it with `on_signup`. Visibility and searchability are explicit field settings, not guarantees about every audience or plugin. Let the operator choose them.
- Use explicit values, not server defaults, for new managed definitions. Capture prior definitions for rollback, but warn that rollback may not restore associated responses or deleted directory configuration. Never export responses as part of this feature.

## Observed dry-run plan gap - Numun, 2026-11-03

With `dsc 0.23.0` against Numun's Discourse `2026.11.0-latest` (`112369ac34436e6597604e79fb35c093d3f9605e`), a curated YAML snapshot specified one new multiselect field with 21 ordered options and all visibility, signup and editability flags. Before creating the field, the operator ran:

```console
dsc ucf push --dry-run numun <curated-movements-snapshot.yaml>
+ create: Movements
[dry-run] No changes applied.
```

The command identified the intended creation and did not create the field, but displayed neither its question, type, requirement, flags nor options. The operator had to inspect the input file separately to review those values. A subsequent deliberate live push created the field, and fresh show/list reads confirmed its configuration; the mutation was the live push, not the dry-run. The placeholder above avoids publishing a client workspace path; use a synthetic snapshot and mock forum to reproduce without modifying Numun.

**Expected:** the create plan includes the target forum, field name, description, type, requirement, all submitted visibility/editability/search flags and every option in order, preserving Unicode and punctuation. If output must be bounded, explicitly indicate truncation and provide a way to review the complete plan; never imply an abbreviated list is complete. Derive the displayed plan and actual request from the same validated representation. A structured plan is welcome if consistent with the CLI contract, but the ordinary dry-run must itself be reviewable.

Add regression coverage using a long, synthetic option list: assert the printed planned values match the actual create request against the same mock state, including booleans, ordering and Unicode; assert dry-run sends no mutation and that unrelated live fields receive no write. Keep the create-only and collision guards intact. Track this as part of R68 and coordinate with R73's plan-fidelity coverage.

**Resolution (2026-11-03):** the create planner now carries the complete validated definition and its text plan prints the normalized target, description, type, requirement, all booleans, and every ordered option as JSON-quoted text. `tests/user-field-test.rs` uses 21 synthetic Unicode/punctuation options to verify that dry-run prints every submitted value without a POST and that a real run sends the matching `user_field` create payload.

## Source-derived API reference - verify before implementation

Inspected local Discourse source at commit `9c72f9d3e93292cca012525b428ff0eee777e140` on 2026-11-03; Numun reported `2026.11.0-latest` commit `112369ac34436e6597604e79fb35c093d3f9605e`. These are different commits. On 2026-11-03, authenticated requests to `koloki-demo` confirmed list/show/create/delete envelopes and that a scalar-only update preserves options while an update with `options` replaces the ordered option list. The synthetic discovery field was deleted after verification.

- `GET /admin/config/user_fields.json` returns a `user_fields` list; source request tests show this is admin-only. `GET /admin/config/user_fields/{id}.json`, `POST /admin/config/user_fields.json`, `PUT /admin/config/user_fields/{id}.json` and `DELETE /admin/config/user_fields/{id}.json` are source-derived routes. Create/update accept a `user_field` object. Confirm all response envelopes, authentication, errors and version differences on a disposable forum.
- Serializer fields include `id`, `name`, `description`, `field_type`, `editable`, `requirement`, `show_on_profile`, `show_on_user_card`, `show_on_signup`, `searchable`, `position`, and sometimes `options`. `required` is a derived compatibility boolean, not a substitute for the three-valued `requirement`: `optional`, `on_signup`, `for_all_users`.
- Current core types include `text`, `textarea`, `date`, `confirm`, `dropdown`, and `multiselect`. Create assigns the next position regardless of a submitted position. Position ordering, round-trip stability and explicit reordering need discovery; do not accidentally reorder existing fields during an ordinary push.
- The controller deletes existing option records and recreates unique values when a nonempty `options` array is sent, even for an apparently harmless change. Omitted or empty `options` do not clear existing options. Serializer omits `options` when empty. Ordering is not explicitly guaranteed by the serializer's `pluck`; verify it on supported versions. Reject an attempted empty replacement rather than reporting success.
- Disabling both profile and user-card visibility on update destroys a related directory-column record in the inspected source. Surface this side effect; restoring the booleans alone must not be described as a complete rollback.
- The current form and multiselect component expose no per-field maximum selections or conditional free-text Other option. Do not imply that a descriptive prompt enforces a three-choice cap. This CLI cannot add server-side constraints absent from the API.

## Phases and acceptance criteria

1. **Discovery:** on an isolated disposable forum, capture redacted list/show/create/update/delete shapes, ordering, versions, permissions and failure cases. Use synthetic accounts and values to establish option replacement, addition/removal/rename/reorder, type change, deletion, response preservation, directory-column effects, requirement behaviour and cleanup. Do not probe destructive semantics on the production forum.
2. **Numun-blocking workflow:** implement list/show/pull/diff/push for new and safely editable definitions, with long Unicode option lists and curated non-pruning semantics. If safe update behaviour remains unresolved, support creation and refuse the unresolved update instead of blocking safe creation. Test exact requests and responses against a stateful mock and verify on a disposable forum.
3. **Lifecycle and additional compatibility:** add only the destructive operations supported by evidence, with explicit guards, and evaluate reordering and plugin extensions separately. No bulk user-response editing, inferred three-choice enforcement, raw ODS import, or automatic data migration in this scope.

Tests must cover create, readback, semantic no-op, scalar-only update without options, adding options without losing existing values, malformed/duplicate input before writes, duplicate live names, wrong-site and missing IDs, same-site rename, empty option replacement refusal, unsupported type/option changes, stale state, visibility side-effect reporting, partial failures, and dry-run plan versus exact real requests with zero dry-run mutations. Include a long country-like list, accented and non-Latin values, punctuation and order, and ensure an unrelated existing Strategies field receives no write. Where actual server behaviour disagrees with the intended contract, record the limitation rather than faking success.

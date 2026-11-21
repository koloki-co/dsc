# `dsc ai llm` - reviewable Discourse AI provider configuration

Roadmap: R88. This feature makes Discourse AI LLM definitions inspectable and repeatable through `dsc`; it does not make `dsc` a chat client, an AI agent, or a provider credential store.

## Motivation

Configuring a self-hosted Discourse AI provider is currently a browser-only sequence of plugin settings, a provider definition, masked credentials, endpoint details, model identifiers, and a live test. An experienced administrator can still select the wrong protocol, lose a working secret while correcting a URL, make an unreviewed production change, or mistake a provider response-format failure for an authentication failure.

On 2026-11-19, Digital Health Networks needed an Ollama Cloud subscription-backed model, described by the operator as `GLM5.3-flash`. Definition 1 was partially configured and failed from the admin UI. On 2026-11-20, the operator reported that the full OpenAI-compatible endpoint `https://ollama.com/v1/chat/completions` resolved that failure. This is useful operational evidence, not a captured Discourse plugin API contract or a universal Ollama recipe. The prior test returned HTTP 500 with `JSON::ParserError` while the supplied body contained native Ollama newline-delimited streaming objects for `glm-5.3-flash:cloud`, which is consistent with a protocol/response-shape mismatch. The configured adapter, request, credential and exact model identifier were not retained or independently verified.

The target workflow is rapid, repeatable and reviewable:

1. inspect the installed capabilities and current non-secret state;
2. pull it to a reviewable snapshot;
3. compose a secret-free local recipe for a named provider setup;
4. validate and dry-run the exact changes against one explicitly named forum;
5. apply only after review, preserving remote secrets unless a fresh secret is deliberately supplied; and
6. run a separately requested synthetic connection test, then retain redacted evidence of the outcome.

## Product boundary

`dsc` owns API-backed discovery, configuration snapshots, plans, mutation, verification and bounded diagnostics. It must preserve the distinction between three things that the browser UI makes easy to conflate:

- ordinary site settings, such as enabling a Discourse AI feature;
- an LLM definition record, containing provider, endpoint, model and provider-specific configuration; and
- an external provider connection test, which can spend money, consume quota and leave remote logs.

`dsc setting` remains the owner of ordinary settings. `dsc ai llm` manages only definition records once a supported contract exists. A recipe may name prerequisite settings and planned definitions together, but must show them as separate resources and use their respective clients.

`dsc` does not invoke an LLM to answer a prompt, upload member content to a provider, hold provider credentials, infer a diagnosis from an opaque 500 response, automatically choose another provider/protocol/model, or bulk-enable AI across a fleet. It does not use browser routes as an API escape hatch.

## Current state and evidence gap

`dsc 0.24.0` shipped with no `ai` command family or LLM-definition client. `dsc setting` can manage visible site settings but not the plugin's separate definition records. R88 Phase 1 starts with safe `ai llm list` and `show`: they use the list endpoint and emit an allowlisted public representation only. Pull, diff, push, recipes and tests remain unavailable.

The reported editor URL, `/admin/plugins/discourse-ai/ai-llms/1/edit`, is a browser route. It does not establish a JSON endpoint, accepted request envelope, serializer, plugin-version compatibility or test route. No raw credential, response body or production configuration is needed to close this gap. Discovery must begin with the installed plugin source/version and then use an authorised disposable forum with synthetic values.

## Proposed command surface

The spelling below is provisional until discovery confirms the supported resource and test contracts. Every mutation is single-forum and requires an explicit definition identity.

```text
dsc ai capability <discourse> [--format text|json|yaml]

dsc ai llm list <discourse> [--format text|json|yaml]
dsc ai llm show <discourse> <id> [--format text|json|yaml]
dsc ai llm pull <discourse> <id> <file>
dsc ai llm diff <discourse> <id> <file>
dsc --dry-run ai llm push <discourse> <id> <file>
dsc ai llm push <discourse> <id> <file> [--secret-stdin <field>]
dsc ai llm test <discourse> <id> [--confirm-external-request]

dsc ai recipe validate <recipe>
dsc ai recipe plan <discourse> <recipe>
dsc --dry-run ai recipe apply <discourse> <recipe>
dsc ai recipe apply <discourse> <recipe> [--secret-stdin <binding>] [--confirm-external-request]
```

`capability` is read-only. It reports only observed capability and compatibility facts: the selected forum, Discourse and Discourse AI versions where exposed, supported definition/test operations, and the reason a capability is unavailable. It must never enable a plugin or feature to make a capability appear.

`list` and `show` return explicitly safe public records. They show secret presence or reference state where the API exposes it, never a secret value, a credential-bearing URL, or an unbounded remote error body.

`pull`, `diff` and `push` use the same parser, normaliser and planner. A dry run does read current state so its plan is real, but never sends a definition write, secret, provider request or test. A real push re-reads immediately before applying, refuses stale/ambiguous state, applies one definition, and re-reads safe fields afterwards. A semantic no-op sends no write.

`test` is deliberately separate from `push` and recipe application. It sends only a fixed synthetic request supported by Discourse, with a bounded timeout and result size. It must print before dispatch that it contacts the configured external provider, may consume quota and may create provider/Discourse logs. `--confirm-external-request` is required for the first interactive version; global `--dry-run` prints the planned test and makes no provider-facing call. It must never send posts, PMs, uploads, users, a caller-provided prompt, or other forum data.

`recipe validate` is offline and makes no network requests. `recipe plan` reads remote state and emits the same ordered plan that application would use. Recipe application never tests by default: a recipe must opt in to a named synthetic test and the caller must still pass `--confirm-external-request`. It is intentionally one forum at a time. The initial implementation may ship `validate` and `plan` before `apply`; it must not claim recipe application before each underlying API operation is evidenced.

## Snapshot contract

Definition snapshots are strict YAML or JSON, selected by file extension. The envelope is schema-versioned and records non-secret provenance. The final names follow the captured serializer, but the shape below defines the safety model:

```yaml
# Illustrative schema - exact fields await API compatibility discovery.
version: 1
kind: discourse-ai-llm-definition
source:
  forum: dhn-staging
  baseurl: https://forum.example.invalid
  discourse_version: 2026.11.0-latest
  discourse_ai_version: 2026.11.0
definition:
  id: 1
  name: Ollama Cloud GLM
  provider: openai_compatible
  endpoint: https://ollama.com/v1/chat/completions
  model: glm-5.3-flash
  parameters:
    max_tokens: 1024
  secrets:
    api_key: preserve
```

The envelope rejects unknown top-level fields and unknown schema versions. `definition` may contain only fields captured as writable and safe for that provider/version. `parameters` is a provider-specific typed map whose admitted keys and JSON value types come from discovery; it is not an unrestricted pass-through object. Unknown server fields are represented only by a field-name/type/presence inventory or a digest. Their values are never exported merely to make round-tripping convenient.

`pull` writes safe fields and `secrets.<field>: preserve` for every masked or omitted secret. It never writes a mask such as `********`, `null`, an empty string, a secret reference value, or a URI userinfo value into the snapshot. `push` treats `preserve` as an instruction to omit that field from the request. It refuses a snapshot that would need an unknown server field to be echoed back, or an edit that would make secret preservation impossible. It does not silently turn an omitted secret into a clear/reset operation.

Source identity is a guard, not a portability promise. An ID pulled from a different or unknown forum fails closed. Intentional copying requires removing the ID and source identity, selecting or creating a target definition through an evidenced workflow, and supplying every target-specific non-secret value deliberately.

## Recipes, configuration guidance and agent skill

A recipe is a secret-free, versioned local plan for a known provider setup. It is intended for a management repository or ignored local workspace, reviewed like other configuration. It is not a second configuration store and does not replace `dsc.toml` forum credentials.

```yaml
# Illustrative recipe - capabilities and setting names await compatibility discovery.
version: 1
kind: discourse-ai-setup
name: ollama-cloud-openai-compatible
description: Configure one existing LLM definition for Ollama Cloud's OpenAI-compatible endpoint.
requires:
  capabilities:
    - ai_llm_update
  settings:
    - name: discourse_ai_enabled
      value: true
definitions:
  - id: 1
    snapshot: ./llm-definition-1.yaml
    secret_requirements:
      - field: api_key
        binding: ollama_cloud_api_key
tests:
  - definition_id: 1
    synthetic: connection
```

The recipe schema rejects unknown keys, duplicate definition IDs/bindings, path traversal outside the recipe directory, empty names, and an attempt to embed a secret. `snapshot` paths are relative to the recipe directory and resolve to strict definition snapshots. `requires.settings` is an explicit prerequisite check and plan item, not an instruction to set arbitrary settings by prefix or pattern. The final initial scope may support only an allowlisted set of prerequisite settings after their interaction with the installed Discourse AI version is evidenced.

Recipes can be checked in because they contain no secret material. Do not put provider keys in YAML, JSON, command arguments, URLs, shell history, `dsc.toml`, test fixtures, plans, output or diagnostic bundles. A secret supplied for an initial or replacement configuration enters only through `--secret-stdin <binding>` from a non-echoing pipe. The command reads exactly one non-empty value, validates its binding and never repeats it. It must reject a secret on `--dry-run`. A future local secret-file mechanism requires a separate design covering permissions, ignore rules, overwrite behaviour and platform key storage; environment-variable and command-line shortcuts are out of scope for the first release.

The accompanying operator guide must state the exact prepare-plan-apply-test sequence, require `dsc config check` first, explain how to keep recipe work in a private/ignored workspace, and include a redacted Ollama Cloud example marked with the evidence date and protocol caveat. It must say that `https://ollama.com/v1/chat/completions` is operator-confirmed for the reported incident, not a guarantee for every Ollama plan or model.

The accompanying agent skill must guide an agent to inspect `dsc ai capability`, pull and diff before proposing a write, validate a recipe offline, require an operator-approved dry-run before applying, and never ask for or echo a provider secret in chat. It must treat provider tests as external, billable operations and report only the bounded/redacted result. The skill is an orchestration guide, not an implementation-specific source of truth; the CLI help and recipe schemas remain authoritative.

## Connection-test diagnostics and failure taxonomy

The test result is an observation, not a root-cause oracle. Text output must identify the definition, test kind, elapsed time, bounded status and safe next action. JSON/YAML use a stable outcome class and redacted evidence fields.

| Outcome class | Evidence needed before using it | Operator action |
|---|---|---|
| `unsupported` | Capability discovery shows no supported test operation | Use the provider/plugin UI or complete discovery; do not synthesize a private request. |
| `configuration_rejected` | Discourse rejects a known request field before provider dispatch | Pull/diff the definition and correct the named field. |
| `transport_failure` | A bounded DNS, TLS, connect or timeout error from the supported test path | Inspect endpoint reachability and TLS. Do not claim the key or model is wrong. |
| `authentication_or_authorisation_failure` | Provider status/body classification is explicitly evidenced and safely redacted | Verify the intended credential and provider account scope. |
| `quota_or_rate_limit` | Provider status/headers or supported plugin classification proves it | Wait, increase quota, or contact the provider. |
| `model_or_capability_failure` | Provider/plugin identifies the requested model or capability | Verify the exact provider model identifier and adapter compatibility. |
| `protocol_or_response_failure` | Bounded evidence identifies incompatible request/response framing | Review adapter and endpoint. Native Ollama and OpenAI-compatible interfaces must not be conflated. |
| `remote_failure` | Provider returned a safely captured non-specific failure | Retain the redacted correlation/status evidence; do not guess a cause. |
| `success` | Supported test reports success and the response passes bounded parsing | This verifies only the synthetic request, not every Discourse AI feature. |

The implementation may report a more specific class only when the contract exposes the required evidence. Otherwise it returns `remote_failure` or `unknown_failure` with status, elapsed time, a capped redacted body digest/summary, and a correlation identifier when safe. It never prints full provider responses by default.

## API discovery and compatibility evidence

Before code, record the following in this spec under a dated **Reference: API calls observed in the field** section:

1. The Discourse version/commit, Discourse AI plugin version/commit, enabled capability state, test forum identity and administrator scope.
2. The source file/controller/serializer or documented plugin API proving each list, show, create, update, delete and test route. Browser route evidence is insufficient.
3. Exact method, route, headers required beyond normal `dsc` authentication, request field names/types, response envelope, pagination and status/error shapes. Keep fixtures synthetic and redact every secret, bearer token, signed URL, userinfo component and provider response body.
4. How the server represents masked secrets, AI Secret references, custom endpoints, model identifiers, provider parameters and unknown fields on read and write.
5. A disposable-forum capture for every proposed mutation: dry-run zero mutation, exact request, readback, no-op, malformed/unsupported input, authentication failure, unavailable plugin/API and cleanup. Tests that create audit records or contact a provider remain opt-in and must follow the live-test cleanup policy.
6. Test-route dispatch semantics: whether it calls the provider synchronously/asynchronously, timeout, cost/logging behaviour, synthetic payload, response cap and whether a test can write persistent provider/Discourse state.

Capability detection is version-aware and conservative. An unknown version, missing plugin, absent endpoint, unexpected serializer field or incompatible contract disables the affected mutating/test command with an actionable explanation. It does not fall back to guessed `.json` routes, browser automation or raw HTTP.

## Phases and acceptance criteria

### Phase 1 - discovery, inspection and safety model

- [ ] Capture the API evidence above on an authorised disposable forum and record the exact supported capability matrix.
- [ ] Implement `ai capability`, `ai llm list` and `ai llm show` with explicit safe public types, capped errors and unavailable/unauthorised/missing-definition distinction.
- [ ] Define and test the field classifier for safe, secret, masked, reference and unknown fields. No snapshot/push command ships until this is complete.

### Phase 2 - reviewable definition updates

- [ ] Implement strict definition snapshots, pull/diff and offline schema validation.
- [ ] Implement one-definition dry-run/push with source guards, preflight re-read, exact plan fidelity, no-op handling and post-write safe readback.
- [ ] Prove remote secret preservation and refusal cases using synthetic credentials. Test endpoint/model/parameter changes, unknown fields, malformed schema, stale state, non-secret output and zero mutation under dry-run.

### Phase 3 - recipes and operating guidance

- [ ] Implement offline recipe validation and remote recipe planning using the same underlying planners as direct commands.
- [ ] Add recipe application only after Phase 2 is live-verified, with separate setting/definition plan lines, one-forum scope and failure/partial-application reporting.
- [ ] Publish the operator guide, recipe reference/example and agent skill. Test that examples validate and contain no secret-looking values.

### Phase 4 - explicit synthetic connection tests

- [ ] Implement `ai llm test` only if a supported route exists, including confirmation, cost warning, hard timeout, body cap, redaction and global dry-run blocking.
- [ ] Add the evidence-bounded outcome taxonomy and tests for every safely reproducible class. Do not fake provider failures with real keys in ordinary CI.
- [ ] Add recipe test opt-in only after the direct test contract is verified. A successful test never automatically enables features or promotes configuration to another forum.

### Phase 5 - lifecycle coverage

- [ ] Consider create/delete only when discovery proves their API and relationship semantics. Refuse deletion while a definition is referenced by a feature, persona, bot or unknown resource. Lifecycle operations are not required for the initial repair workflow.

## Backward compatibility and out of scope

Existing `dsc setting` behaviour is unchanged. The new commands and schemas are additive. Schema versions are never silently upgraded; later versions require an explicit migration command or documented manual conversion.

Out of scope for the initial feature:

- production configuration changes without explicit operator authorisation;
- fleet mutation, bulk provider rollout, automatic fallback or configuration copying by numeric ID;
- provider credential storage, environment-variable/argument secrets, secret export, or a key manager;
- arbitrary prompts, forum/member content, retrieval tests, tool-call tests or quality evaluation;
- claiming that a successful synthetic test validates every Discourse AI feature, provider account entitlement or future model availability;
- browser automation, guessed API routes, Rails-console access and raw HTTP escape hatches; and
- a conversational AI client, MCP server or embedded agent runtime.

## Reference: API calls observed in the field

### Source contract - 2026-11-21

The current Discourse core commit [`ba3bd0d`](https://github.com/discourse/discourse/tree/ba3bd0d0d47622cf8a9d216d1d935cf93aaec41d) ships Discourse AI under `plugins/discourse-ai/`; the former standalone `discourse/discourse-ai` repository was folded into core in August 2026. The current [controller](https://github.com/discourse/discourse/blob/ba3bd0d0d47622cf8a9d216d1d935cf93aaec41d/plugins/discourse-ai/app/controllers/discourse_ai/admin/ai_llms_controller.rb), [routes](https://github.com/discourse/discourse/blob/ba3bd0d0d47622cf8a9d216d1d935cf93aaec41d/plugins/discourse-ai/config/routes.rb), [serializer](https://github.com/discourse/discourse/blob/ba3bd0d0d47622cf8a9d216d1d935cf93aaec41d/plugins/discourse-ai/app/serializers/llm_model_serializer.rb), and [request specs](https://github.com/discourse/discourse/blob/ba3bd0d0d47622cf8a9d216d1d935cf93aaec41d/plugins/discourse-ai/spec/requests/admin/ai_llms_controller_spec.rb) establish this source-derived contract:

| Operation | Method and path | Finding |
|---|---|---|
| List | `GET /admin/plugins/discourse-ai/ai-llms.json` | Returns `ai_llms` plus `meta` with provider parameters, capabilities, tokenizers and AI Secret metadata. |
| Browser edit | `GET /admin/plugins/discourse-ai/ai-llms/:id/edit` | Exists as a Rails resource route, but R88 does not need it: list returns full serialized definition rows. |
| Create | `POST /admin/plugins/discourse-ai/ai-llms.json` | Expects an `ai_llm` envelope. Not implemented by R88 yet. |
| Update | `PUT /admin/plugins/discourse-ai/ai-llms/:id.json` | Uses an `ai_llm` envelope. Quota/credit subrecords have replacement semantics and must not be managed until separately designed. |
| Test | `POST /admin/plugins/discourse-ai/ai-llms/test.json` | Rate-limited to six requests/minute per user. It builds a model from submitted configuration, can test an existing seeded ID, and probes both non-streaming and streaming modes. It can contact a provider and is not implemented by R88 yet. |
| Delete | `DELETE /admin/plugins/discourse-ai/ai-llms/:id.json` | Refuses definitions in use. Not implemented by R88. |

The serializer exposes `api_key` directly for custom definitions, uses `********` for seeded ones, and returns provider-specific parameters plus AI Secret identifiers. This is a credential boundary, not an output model. R88's initial client parses the response privately and emits only an allowlisted summary: signed ID, display/model/provider names, tokenizer, redacted endpoint, API-key presence, AI Secret ID, seeded state and context limits. It does not emit `api_key`, provider parameters, AI Secret metadata, use references, quotas or unknown response fields.

The old standalone-plugin source at commit [`b6630b6`](https://github.com/discourse/discourse-ai/tree/b6630b670f36fadcff1bc85d5f0d9dcf1e440420) used the same list/create/update/delete paths but a `GET` test route limited to three requests/minute and lacked current secret/credit fields. Do not use it as a current test contract.

### Authorised read-only compatibility check - `koloki-demo`, 2026-11-21

`dsc plugin list koloki-demo` reported the core `discourse-ai` plugin at version `0.0.1`, disabled. An authenticated `GET /admin/plugins/discourse-ai/ai-llms.json` returned HTTP 404. Its body was discarded without parsing or retention. No AI setting, definition, provider request or test was changed. This proves only that a disabled plugin returns an unavailable API response on this forum; it is not a success-shape capture and this forum must not be enabled merely for R88 discovery.

<!--
SPDX-FileCopyrightText: 2026 Marcus Baw and Koloki Ltd

SPDX-License-Identifier: CC-BY-SA-4.0
-->

# dsc ai

Inspect non-secret Discourse AI LLM definition summaries. The current R88 Phase 1 commands are read-only: they do not enable Discourse AI, change a definition, send a provider request, or test a model.

Discourse's current LLM list API returns raw API keys for custom definitions. `dsc` deliberately omits API keys, provider parameters, AI Secret metadata, quotas, usage references and unknown fields from every output format. It reports only whether an API key is configured, plus a redacted endpoint when one is available.

A 404 means Discourse AI may be disabled, unavailable on this Discourse version, or its admin API may have changed. `dsc` does not enable the plugin to resolve that condition.

## List LLM definitions

```text
dsc ai llm list <discourse> [--format text|json|yaml]
```

```bash
dsc ai llm list myforum
dsc ai llm list myforum --format yaml
```

Definitions are ordered by display name. Seeded definitions can use negative IDs.

## Show one LLM definition

```text
dsc ai llm show <discourse> <llm-id> [--format text|json|yaml]
```

```bash
dsc ai llm show myforum 1
dsc ai llm show myforum -200 --format json
```

`show` selects the exact ID from the documented list response. It does not call the browser-oriented edit route.

Pull, diff, push, recipes and provider tests are planned under [R88](https://github.com/koloki-co/dsc/blob/main/spec/commands/ai-llm-management.md). Do not use a browser route or an ad-hoc HTTP request as a substitute.

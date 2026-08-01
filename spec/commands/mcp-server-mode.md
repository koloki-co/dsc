# `dsc mcp` - expose dsc as a Model Context Protocol server

Spec for R24. Goal: let an MCP client that cannot spawn a binary drive a Discourse fleet through `dsc`. Driver: none yet - this spec records the design decisions and the open question that must be answered before any code is written.

**Status: not scheduled.** R24 sits under *Stretch / exploratory*: build only on real demand. This document exists so the reasoning is not lost, and so that whoever picks it up starts from the decisions below rather than re-litigating them.

## Motivation

`dsc` already reaches the Discourse admin API and, over SSH, the container host. Agents use it today by shelling out: `--format json` for structured output, `--dry-run` for a review gate, and [AGENTS.md](../../AGENTS.md) to orient. That works, and it is the reason this is not urgent.

An MCP server earns its place in exactly one situation: a client that **cannot** spawn a binary. Claude Desktop, a mobile client, or any host where the tool surface must be declared rather than executed. For a terminal-capable agent the marginal gain over shelling out is small - schema discovery and no shell quoting - and does not justify a second surface.

## The open question that gates this work

The assumed justification is that `dsc` reaches the **admin** API while the official Discourse MCP is limited to the **User API**, leaving fleet administration uncovered. That assumption is **unverified and looks doubtful**.

Discourse announced an official MCP server ([Discourse MCP is here!](https://meta.discourse.org/t/discourse-mcp-is-here/386983)). Community setup notes describe a locally-run stdio server configured with `auth_pairs`, plus `read_only` and `allow_writes` flags, which implies configurable authentication and a genuine write surface.

**Before writing any code, establish:**

1. What authentication the official server accepts - User API key only, or an admin API key.
2. Which operations it exposes, and whether they cover site settings, themes, categories, backups, and fleet-wide reads.
3. Whether it is multi-forum, or one server instance per forum.

If it already accepts admin credentials and covers the common surface, the case for `dsc mcp` largely collapses and this item should be closed as *Out of scope*, not built. The genuinely distinctive capabilities would then be only those with no HTTP API at all: SSH-driven `update`, `app env`, `harden`, and `plugin`.

## Current state (as of 2026-08-01)

No MCP surface exists. `dsc` is deliberately synchronous (`reqwest::blocking`). The official Rust SDK, `rmcp`, is at 3.1.0, actively maintained, and out of beta, so SDK maturity is not a blocker.

## Design decisions

These hold whenever the work is picked up.

### Consolidate, do not coexist

Fold `discourse-bawmedical-mcp` into this rather than maintaining two servers. `dsc` already owns multi-forum configuration and tag filtering, the API client, rate-limit retry with `Retry-After` handling, and the destructive-action guards. A separate MCP server reimplements all of that and will drift from it.

### Isolate the async runtime

`rmcp` brings `tokio`. Keep it out of the CLI binary: a separate workspace crate and binary (`dsc-mcp`) depending on a shared library, per the leaf-crate discipline in [library-extraction.md](../../../house-style/library-extraction.md). Every `dsc list` should not carry an async runtime it never uses.

### Read-only by default

Writes are opt-in per server invocation (for example `--allow-writes`, off by default). Handing destructive fleet administration to a model with no human at the call site is a materially different risk posture from a person typing `dsc`. The default surface should be diagnostics: `list`, `show`, `diff`, `audit`, `explorer`, `analytics`.

### Curate the tool list

`dsc` has 133 leaf commands. Do **not** map them one-to-one: every tool definition costs context on every request. Choose roughly 10 to 15 tools covering the pull/edit/push loop and the cross-forum reads that are the tool's real advantage.

### Reuse the existing safety machinery

MCP tools call the same command functions as the CLI, so `--dry-run`, the `--yes` confirmation guards, and `dry_run_refusal_reason` all apply unchanged. Do not re-implement request building in the MCP layer.

Extend [tests/dry-run-mutation-test.rs](../../tests/dry-run-mutation-test.rs) so every exposed tool maps to a command already triaged there. That test exists because `explorer run` shipped a `--dry-run` gap that the classification tests could not see; an MCP surface that bypasses it would reopen exactly that hole with a model, rather than a human, holding the trigger.

## Proposed CLI surface

```text
dsc mcp serve [--allow-writes] [--tags <tags>] [--forum <name>]
```

Stdio transport, matching how MCP clients launch local servers. `--tags` and `--forum` narrow which configured forums the server exposes, so a client can be given a single staging forum rather than the whole fleet.

## Phases

### Phase 1 - answer the gating question

- [ ] Establish the official Discourse MCP's auth model, capability surface, and multi-forum behaviour.
- [ ] Decide build or close. Record the outcome here either way.

### Phase 2 - minimum useful server (only if Phase 1 says build)

- [ ] `dsc-mcp` crate with stdio transport and read-only tools.
- [ ] Tool schemas generated from the same source as the CLI arguments, not hand-written.
- [ ] Triage test extended to cover every exposed tool.

### Phase 3 - guarded writes

- [ ] `--allow-writes`, with dry-run plans surfaced to the client before any mutation.
- [ ] Decide how a confirmation step is represented over MCP.

## Backward compatibility

Purely additive. The CLI is unchanged, and the MCP binary ships separately so `dsc` gains no dependencies.

## Out of scope

- Exposing every command as a tool.
- A hosted or remote-transport server; local stdio only.
- Re-implementing Discourse API access outside the existing client.
- Any write surface before the read-only surface is in real use.

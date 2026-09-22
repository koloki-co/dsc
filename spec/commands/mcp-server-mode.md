# `dsc mcp` - closed proposal

Spec for R24. Goal considered: expose `dsc` as a Model Context Protocol server. Driver: none materialised.

**Status: closed as out of scope, reaffirmed 2026-09-21.** Discourse now ships a built-in, per-site MCP server with OAuth, user and group scopes, normal Guardian permission checks, administrator-controlled capability exposure, and remote Streamable HTTP for web-hosted AI clients. CDCK also maintains the separate [`@discourse/mcp`](https://github.com/discourse/discourse-mcp) package for a broader API-key-backed administration surface. A `dsc` MCP server would create a third overlapping protocol surface without a demonstrated operator need.

## Resolved decision

For conversational AI access to one community, use [Discourse's built-in MCP server](https://meta.discourse.org/t/connect-your-ai-apps-to-your-community-with-discourse-s-built-in-mcp-server/412755). For MCP access to broader Discourse HTTP administration, consider the official `@discourse/mcp` package. A terminal-capable local agent can execute `dsc` directly and consume `--format json` output.

`dsc` remains distinct as a local-first operations and state-management tool for multiple Discourse installations:

- fleet selection, bounded fan-out, and cross-forum aggregation;
- durable YAML, JSON, and Markdown snapshots suitable for review and Git;
- pull, edit, diff, dry-run, and push workflows;
- deterministic archival and compliance exports;
- backups, upgrades, installation, and SSH/Docker/storage operations;
- repeatable shell automation independent of an AI conversation.

The existence of an upstream MCP tool does not automatically exclude an overlapping CLI command. The command must add an operator property such as fleet behavior, deterministic export, local persistence, dry-run/diff, unattended scripting, or stronger preconditions and verification. Do not add a command solely to reproduce a built-in MCP interaction for one forum.

## Distinguish the two official MCP implementations

The built-in server and `@discourse/mcp` are related but different products:

- The built-in server runs inside one Discourse instance at `/mcp`, authenticates a real user through OAuth, and applies both granted MCP scopes and the user's existing Discourse permissions. It is the preferred route for web-hosted AI applications.
- `@discourse/mcp` is a separately installed Node server that accepts User or Admin API credentials, can hold credentials for several sites, and exposes broader opt-in administration toolsets.

Neither provides `dsc`'s declarative local files, universal CLI dry-run contract, SSH operations, or first-class fleet orchestration.

## Safety lessons retained as R54

The upstream implementations remain useful references for opt-in writes, narrow capability exposure, fresh-state checks, explicit confirmation, bounded output, rate limiting, redacted diagnostics, and post-write verification. R54 tracks adopting those patterns in `dsc` without adopting MCP transport or duplicating upstream tools.

## Backward compatibility

No MCP surface shipped, so closing the proposal removes no supported behavior.

## Out of scope

- A `dsc mcp` or `dsc-mcp` binary.
- Remote or stdio MCP transport in this repository.
- Mirroring every `dsc` command as an MCP tool.
- Dynamic remote-tool discovery.

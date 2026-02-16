---
title: "MCP HTTP Headers and CLI Integration"
date: 2026-03-01
category: integration-issues
tags:
  - mcp
  - rmcp
  - http-headers
  - authentication
  - cli
severity: medium
component: mika-agent, mika-cli
related_files:
  - crates/mika-agent/src/mcp/config.rs
  - crates/mika-agent/src/mcp/mod.rs
  - crates/mika-cli/src/commands/mcp.rs
  - crates/mika-cli/src/commands/ask.rs
  - crates/mika-cli/src/commands/chat.rs
  - crates/mika-cli/src/init.rs
  - templates/skills/mcp/system_prompt.md
---

# MCP HTTP Headers and CLI Integration

## Problem Statement

Three gaps in the MCP integration prevented full use of the MCP ecosystem:

1. **No HTTP headers** -- Remote MCP servers requiring authentication (Bearer tokens, API keys) could not be connected to. The `connect_http()` function called `StreamableHttpClientTransport::from_uri(url)` with no header support.
2. **CLI modes had no MCP** -- Both `mika ask` and `mika` (chat) passed `mcp_manager: None` to `AgentParams`. Only server mode (`mika-server`) connected to MCP servers.
3. **No enable/disable commands** -- Users could set `"enabled": false` in `mcp.json` manually, but there were no CLI commands to toggle servers without editing JSON.

## Root Cause

The original MCP implementation (Feb 28) focused on server mode and the core transport/dispatch infrastructure. HTTP headers, CLI integration, and convenience commands were deferred to a follow-up.

## Solution

### 1. HTTP Headers on McpServerConfig

Added `headers: Option<HashMap<String, String>>` to `McpServerConfig` with `#[serde(default)]` for backwards compatibility. In `connect_http()`, the `Authorization` header routes through rmcp's `auth_header()` method (case-insensitive match), while other headers use `custom_headers()` with `http::HeaderName`/`http::HeaderValue` validation.

**Key insight:** rmcp 0.17's `StreamableHttpClientTransportConfig` has both `.auth_header()` (for Authorization) and `.custom_headers()` (for arbitrary headers). Using `from_config()` instead of `with_client()` avoids needing to implement the `StreamableHttpClient` trait manually.

### 2. CLI MCP Integration

Created `init::connect_mcp()` helper that encapsulates the load-config/check-empty/connect-all/check-connections pattern (previously duplicated in server mode). Used in both `ask.rs` (per-invocation with explicit `shutdown()`) and `chat.rs` (session-persistent, moved into worker task).

### 3. Enable/Disable Commands

Added `toggle_server()` function with idempotent behavior (enabling an already-enabled server succeeds silently). Added `--header KEY=VALUE` flag to `mcp add` with `parse_headers()` that splits on first `=` (handles values containing `=` like Base64 tokens).

## Security Measures

- Manual `Debug` impl redacts both `headers` and `env` values (shows keys only)
- `mcp.json` written with `0600` permissions on Unix
- Case-insensitive Authorization header matching prevents misrouting through wrong API
- CLI `--header` values visible in shell history/process list -- documented as known limitation

## What We Learned

1. **rmcp API discovery requires reading source** -- The Context7/docs.rs documentation showed only `auth_header()`. Reading the actual rmcp 0.17 source at `~/.cargo/registry/src/` revealed `custom_headers()` and `from_config()`, which simplified the implementation significantly.

2. **Case sensitivity matters for HTTP headers** -- Initial implementation used exact-match `headers.get("Authorization")` which would silently misroute lowercase `authorization` keys. HTTP header names are case-insensitive per RFC 9110.

3. **File permissions for secrets** -- The existing `home.rs` bootstrap sets `0600` on known config files, but dynamically created files like `mcp.json` need explicit permission setting after write.

4. **Team agent MCP deferred wisely** -- Adding MCP to team agents involves per-agent connection management during parallel `JoinSet` execution, concurrent access safety on `RunningService`, and potentially 10+ connections at team init time. Deferring avoided significant scope creep.

## References

- Plan: `docs/plans/2026-03-01-feat-mcp-headers-per-agent-enable-plan.md`
- Original MCP integration: `docs/solutions/integration-issues/mcp-client-integration-rmcp.md`
- rmcp crate: `~/.cargo/registry/src/*/rmcp-0.17.0/src/transport/streamable_http_client/`

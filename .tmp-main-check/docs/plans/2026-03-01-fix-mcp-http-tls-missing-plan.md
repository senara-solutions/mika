---
title: "fix: MCP HTTP transport fails on HTTPS URLs due to missing TLS"
type: fix
status: completed
date: 2026-03-01
---

# fix: MCP HTTP transport fails on HTTPS URLs due to missing TLS

## Overview

MCP HTTP transport connections to HTTPS servers (e.g., Context7 at `https://mcp.context7.com/mcp`) fail silently at startup with `ConnectError("invalid URL, scheme is not http")`. The rmcp crate's reqwest dependency lacks TLS support because Mika enables `transport-streamable-http-client-reqwest` but not the `reqwest` feature flag that activates rustls.

## Problem Statement

The user configured Context7 as an HTTP MCP server in `~/.mika/agents/main/mcp.json`:
```json
{
  "mcpServers": {
    "context7": {
      "transport": "http",
      "url": "https://mcp.context7.com/mcp",
      "headers": { "Authorization": "Bearer ..." },
      "enabled": true
    }
  }
}
```

Mika starts, tries to connect, gets a TLS error from hyper/reqwest, logs a warning, and continues without MCP tools. The user only discovers the failure when they ask Mika and it says "no mcp__context7__* tools are available."

**Error chain:**
```
rmcp::transport::worker: worker quit with fatal: Transport channel closed,
  when Client(reqwest::Error { kind: Request, url: "https://mcp.context7.com/mcp",
  source: hyper_util::client::legacy::Error(Connect,
    ConnectError("invalid URL, scheme is not http")) })
```

**Root cause in rmcp's Cargo.toml:**
```toml
[dependencies.reqwest]
version = "0.13.2"
features = ["json", "stream"]  # No TLS!
optional = true
default-features = false
```

The `transport-streamable-http-client-reqwest` feature only enables `__reqwest` (the dep), NOT TLS. TLS requires the separate `reqwest` feature (adds `reqwest?/rustls`) or `reqwest-native-tls`.

**Mika's current features (Cargo.toml workspace):**
```toml
rmcp = { version = "0.17", default-features = false, features = [
    "client",
    "transport-child-process",
    "transport-streamable-http-client",
    "transport-streamable-http-client-reqwest",
    # Missing: "reqwest" for TLS!
] }
```

## Proposed Solution

### 1. Add TLS feature to rmcp dependency

**File:** `Cargo.toml` (workspace root, line 90-95)

Add `"reqwest"` to the rmcp features list. This activates `reqwest?/rustls` for TLS support:

```toml
rmcp = { version = "0.17", default-features = false, features = [
    "client",
    "transport-child-process",
    "transport-streamable-http-client",
    "transport-streamable-http-client-reqwest",
    "reqwest",
] }
```

**Why `reqwest` (rustls) over `reqwest-native-tls`:**
- Consistent with Mika's existing reqwest 0.12 which uses rustls (via `default-features`)
- No system OpenSSL dependency needed
- Smaller binary size on Linux

### 2. Add integration test for HTTPS MCP URL validation

**File:** `crates/mika-agent/src/mcp/config.rs` (tests section)

Add a test confirming HTTPS URLs parse and validate correctly (already covered by `test_validate_http_accepts_https`, but add a comment noting TLS feature requirement).

### 3. Verify the fix

Run `mika ask "list your mcp tools"` with Context7 configured and confirm `mcp__context7__*` tools appear.

## Acceptance Criteria

- [x] rmcp dependency includes `"reqwest"` feature for TLS (rustls) support
- [x] `mika ask "list your mcp tools"` shows `mcp__context7__*` tools when Context7 is configured
- [x] `cargo test` passes
- [x] `cargo clippy` passes
- [x] Existing stdio MCP transport unaffected

## Context

- rmcp 0.17 separates TLS from transport features — easy to miss
- The error message "invalid URL, scheme is not http" is misleading (sounds like a URL validation error, not a missing-TLS error)
- This affects ALL HTTPS MCP servers, not just Context7

## References

- `Cargo.toml:90-95` — rmcp workspace dependency
- `crates/mika-agent/src/mcp/mod.rs:286-346` — `connect_http` function
- `~/.cargo/registry/src/*/rmcp-0.17.0/Cargo.toml:60-63` — rmcp `reqwest` feature enables rustls
- `docs/solutions/integration-issues/mcp-http-headers-cli-integration.md` — MCP HTTP headers solution doc

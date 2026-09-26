---
title: "MCP HTTP Transport Fails on HTTPS URLs Due to Missing TLS"
date: 2026-03-01
category: integration-issues
tags:
  - mcp
  - rmcp
  - tls
  - rustls
  - reqwest
  - cargo-features
  - rust
severity: high
component: mika-agent
related_files:
  - Cargo.toml
  - crates/mika-agent/src/mcp/mod.rs
  - crates/mika-agent/src/mcp/config.rs
---

# MCP HTTP Transport Fails on HTTPS URLs Due to Missing TLS

## Problem Statement

MCP HTTP transport connections to HTTPS servers (e.g., Context7 at `https://mcp.context7.com/mcp`) fail silently at startup. Mika logs a warning and continues without the MCP tools, so the user only discovers the failure when asking Mika and it reports no `mcp__context7__*` tools available.

### Symptoms

- User configures an HTTPS MCP server in `mcp.json` with `"transport": "http"` and an `https://` URL
- Mika starts successfully but the MCP server is not connected
- When asked, Mika says it has no MCP tools for that server
- Log output shows: `ConnectError("invalid URL, scheme is not http")`

### Error Chain

```
rmcp::transport::worker: worker quit with fatal: Transport channel closed,
  when Client(reqwest::Error { kind: Request, url: "https://mcp.context7.com/mcp",
  source: hyper_util::client::legacy::Error(Connect,
    ConnectError("invalid URL, scheme is not http")) })
```

The error message is misleading — it sounds like a URL validation error, but it's actually a missing TLS implementation error from hyper's HTTP-only connector.

## Root Cause

The `rmcp` crate (v0.17) separates TLS from transport features. Its internal `reqwest` dependency uses `default-features = false` with only `["json", "stream"]` — no TLS:

```toml
# Inside rmcp's Cargo.toml
[dependencies.reqwest]
version = "0.13.2"
features = ["json", "stream"]  # No TLS!
optional = true
default-features = false
```

Mika enabled `transport-streamable-http-client-reqwest` which activates the reqwest dependency, but NOT TLS. TLS requires the separate `reqwest` feature flag (which adds `reqwest?/rustls`) or `reqwest-native-tls`.

**Before (broken):**
```toml
rmcp = { version = "0.17", default-features = false, features = [
    "client",
    "transport-child-process",
    "transport-streamable-http-client",
    "transport-streamable-http-client-reqwest",
    # Missing: "reqwest" for TLS!
] }
```

## Solution

Add the `"reqwest"` feature to the rmcp dependency in the workspace `Cargo.toml`. This activates `reqwest?/rustls` for TLS support:

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
- Consistent with Mika's existing reqwest 0.12 which uses rustls
- No system OpenSSL dependency needed
- Smaller binary size on Linux
- Pure Rust — no C library linking issues

### Files Changed

| File | Change |
|------|--------|
| `Cargo.toml:90-96` | Added `"reqwest"` to rmcp features list |

### Verification

```bash
RUST_LOG=info cargo run --bin mika -- ask "list your mcp tools"
# Output: connected to MCP server, server: context7, tools: 2
```

## Key Insight

**Cargo feature flags for transport vs TLS are separate concerns in rmcp.** The `transport-streamable-http-client-reqwest` feature only enables the reqwest transport implementation. TLS is an orthogonal feature that must be explicitly opted into. This is easy to miss because:

1. Most Rust HTTP clients enable TLS by default via `default-features = true`
2. The rmcp feature name suggests full HTTP client support
3. The error message (`"invalid URL, scheme is not http"`) doesn't mention TLS at all

**Rule of thumb:** When adding an HTTP client dependency with `default-features = false`, always check whether TLS is included in your selected features. Look for `rustls`, `native-tls`, or similar features in the crate's `Cargo.toml`.

## Prevention

- When adding HTTP transport features to Cargo dependencies, verify TLS is included by checking if HTTPS URLs work
- The `ConnectError("invalid URL, scheme is not http")` error from hyper should be treated as a "missing TLS" signal, not a URL validation error
- Test MCP HTTP connections with both `http://` and `https://` URLs

## Related

- [MCP Client Integration with rmcp](mcp-client-integration-rmcp.md) — initial MCP integration
- [MCP HTTP Headers and CLI Integration](mcp-http-headers-cli-integration.md) — HTTP headers support
- `docs/plans/2026-03-01-fix-mcp-http-tls-missing-plan.md` — original plan

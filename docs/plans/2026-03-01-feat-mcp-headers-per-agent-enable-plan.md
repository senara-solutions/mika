---
title: "feat: Add MCP HTTP Headers, CLI Integration, and Enable/Disable Commands"
type: feat
status: completed
date: 2026-03-01
---

# feat: Add MCP HTTP Headers, CLI Integration, and Enable/Disable Commands

## Overview

Three enhancements to the existing MCP (Model Context Protocol) system:

1. **HTTP Headers** -- Add `headers` field to `McpServerConfig` so HTTP-transport MCP servers can receive custom headers (Authorization, API keys, etc.)
2. **CLI MCP Integration** -- Connect to MCP servers in CLI mode (`mika ask` and `mika chat`), which currently pass `mcp_manager: None`
3. **Enable/Disable CLI Commands** -- Add `mika mcp enable <name>` and `mika mcp disable <name>` subcommands

## Problem Statement / Motivation

**Headers:** The current `connect_http()` function uses `StreamableHttpClientTransport::from_uri(url)` with no headers. Many remote MCP servers require authentication (Bearer tokens, API keys). Users cannot connect to authenticated HTTP MCP servers at all.

**CLI Integration:** MCP servers are only connected in server mode (`mika-server`). The CLI (`mika ask` and `mika chat`) passes `mcp_manager: None`, meaning CLI users get zero MCP tool access despite having configured servers in `mcp.json`. Team agents also pass `None`.

**Enable/Disable:** Users can set `"enabled": false` in `mcp.json` manually, but there are no CLI commands to toggle this. The existing `mika mcp add/remove/list` commands don't cover toggling.

## Proposed Solution

### 1. HTTP Headers Config

Add a `headers` field to `McpServerConfig`:

```json
{
  "mcpServers": {
    "remote-api": {
      "transport": "http",
      "url": "https://mcp.example.com/v1",
      "headers": {
        "Authorization": "Bearer sk-...",
        "X-API-Key": "abc123"
      },
      "enabled": true
    }
  }
}
```

Use rmcp's `StreamableHttpClientTransportConfig` builder for the `Authorization` header (via `.auth_header()`). For arbitrary headers, wrap the reqwest client to inject additional headers.

### 2. CLI MCP Integration

Load and connect MCP servers in both `ask.rs` and `chat.rs` using the same pattern as `server/mod.rs`:
- Load `McpConfig` from `agent_home`
- Call `McpManager::connect_all()`
- Pass `Option<&McpManager>` to `AgentParams`
- Ensure MCP connections are shut down on exit

For chat mode, MCP connections persist for the session lifetime. For ask mode, connections are created and torn down per invocation.

### 3. Enable/Disable CLI Commands

Add two new subcommands:
- `mika mcp enable <name>` -- sets `enabled: true` in `mcp.json`
- `mika mcp disable <name>` -- sets `enabled: false` in `mcp.json`

## Technical Approach

### Phase 1: Headers Support

**Files to modify:**

- `crates/mika-agent/src/mcp/config.rs` -- Add `headers` field to `McpServerConfig`
- `crates/mika-agent/src/mcp/mod.rs` -- Use headers in `connect_http()`
- `crates/mika-cli/src/cli.rs` -- Add `--header` flag to `mcp add`
- `crates/mika-cli/src/commands/mcp.rs` -- Pass headers through `add_server()`

#### 1.1 Config schema change

```rust
// crates/mika-agent/src/mcp/config.rs
pub struct McpServerConfig {
    pub transport: McpTransport,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub url: Option<String>,
    /// HTTP headers for Streamable HTTP transport (e.g. Authorization, API keys).
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}
```

Backwards compatible: `headers` is optional with `#[serde(default)]`, so existing `mcp.json` files parse without changes.

#### 1.2 HTTP transport with headers

The rmcp `StreamableHttpClientTransportConfig` has a built-in `.auth_header()` method that injects the `Authorization` header. For additional arbitrary headers, we build a custom reqwest `Client` with default headers and pass it via `StreamableHttpClientTransport::with_client()`.

```rust
// crates/mika-agent/src/mcp/mod.rs
async fn connect_http(name: &str, config: &McpServerConfig) -> Result<McpConnection> {
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

    let url = config.url.as_deref().unwrap_or_default();
    let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url);

    // Extract Authorization header for rmcp's built-in auth support
    if let Some(headers) = &config.headers {
        if let Some(auth) = headers.get("Authorization") {
            transport_config = transport_config.auth_header(auth.clone());
        }
    }

    // Build reqwest client with remaining custom headers
    let extra_headers = config.headers.as_ref()
        .map(|h| h.iter()
            .filter(|(k, _)| k.as_str() != "Authorization")
            .collect::<Vec<_>>())
        .unwrap_or_default();

    let transport = if extra_headers.is_empty() {
        StreamableHttpClientTransport::from_config(transport_config)
    } else {
        let mut header_map = reqwest::header::HeaderMap::new();
        for (key, value) in &extra_headers {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                header_map.insert(name, val);
            } else {
                warn!(server = name, header = %key, "invalid header, skipping");
            }
        }
        let client = reqwest::Client::builder()
            .default_headers(header_map)
            .build()?;
        StreamableHttpClientTransport::with_client(client, transport_config)
    };

    // ... existing handshake + list_tools code
}
```

**Note:** The exact API for `StreamableHttpClientTransport::with_client()` and `from_config()` needs to be verified against rmcp 0.17. The approach may simplify to just building a reqwest client with all headers (including Authorization) as default headers, bypassing rmcp's `auth_header()` entirely.

#### 1.3 Header validation

- Headers must not have empty keys. Values are validated by reqwest's `HeaderValue::from_str()`. Invalid headers are warned and skipped (same graceful degradation pattern as env vars).
- `headers` field on a stdio-transport server: `validate()` should warn and ignore (not error) since the field is simply irrelevant. CLI `mika mcp add --header ... --transport stdio` should error with "headers are only for HTTP transport."
- No RFC 7230 header name validation at config level — let reqwest reject invalid headers at connection time.

#### 1.4 Security: Debug redaction

Add manual `Debug` impl for `McpServerConfig` that shows header **keys** but redacts **values** (e.g., `{"Authorization": "[REDACTED]"}`). Header values are more likely to contain bearer tokens than env values. Follow the same pattern as `Settings`' manual `Debug` impl that redacts `anthropic_api_key`.

#### 1.5 CLI `--header` flag

```rust
// crates/mika-cli/src/cli.rs
Add {
    name: String,
    #[arg(long)]
    transport: String,
    #[arg(long)]
    command: Option<String>,
    #[arg(long, num_args = 1..)]
    args: Option<Vec<String>>,
    #[arg(long)]
    url: Option<String>,
    /// HTTP headers as KEY=VALUE pairs (http transport)
    #[arg(long = "header", num_args = 1..)]
    headers: Option<Vec<String>>,
},
```

Parse `KEY=VALUE` pairs in `add_server()` and store in `McpServerConfig.headers`.

### Phase 2: CLI MCP Integration

**Files to modify:**

- `crates/mika-cli/src/commands/ask.rs` -- Load MCP, pass to AgentParams
- `crates/mika-cli/src/commands/chat.rs` -- Load MCP, persist for session, shutdown on exit

#### 2.1 Ask mode (one-shot)

Connect eagerly on startup (consistent with server mode). MCP connection adds latency (up to 30s for the slowest server, parallel via JoinSet). This is acceptable because `ask` mode already requires API round-trips.

```rust
// crates/mika-cli/src/commands/ask.rs
pub async fn run(message: &str, agent_name: &str) -> Result<()> {
    let ctx = init::init_for_agent(agent_name)?;
    // ... existing setup ...

    // Connect MCP servers
    let mcp_config = mika_agent::mcp::config::McpConfig::load(&ctx.home_dir)?;
    let mcp_manager = if mcp_config.mcp_servers.is_empty() {
        None
    } else {
        let manager = mika_agent::mcp::McpManager::connect_all(&mcp_config).await;
        if manager.has_connections() { Some(manager) } else { None }
    };

    let output = agent::run_agent(&AgentParams {
        // ... existing fields ...
        mcp_manager: mcp_manager.as_ref(),
    }).await?;

    // Shutdown MCP
    if let Some(mcp) = mcp_manager {
        mcp.shutdown().await;
    }

    // ... existing output handling ...
}
```

#### 2.2 Chat mode (persistent)

In `chat.rs`, MCP connections should be established once when the worker spawns and persist for the session:

```rust
// Inside spawn_agent_worker()
let mcp_config = mika_agent::mcp::config::McpConfig::load(&ctx.home_dir)?;
let mcp_manager = if mcp_config.mcp_servers.is_empty() {
    None
} else {
    let manager = mika_agent::mcp::McpManager::connect_all(&mcp_config).await;
    if manager.has_connections() { Some(manager) } else { None }
};

// Move into the worker task, pass to each AgentParams call
// Shutdown on worker exit
```

The `McpManager` needs to be moved into the worker task's `move || async {}` closure. Since `McpManager` is not `Clone`, move it into the spawned task as an owned `Option<McpManager>`. The TUI does not need direct access — tool names are communicated via `AgentResponse`. On worker exit (when the channel closes), call `McpManager::shutdown()`. For stdio servers, `kill_on_drop(true)` provides safety-net cleanup.

**Note:** If an MCP server crashes mid-session (stdio process exits, HTTP server goes down), tool calls to that server will return errors. There is no reconnection logic — this is consistent with the existing design. The user must restart `mika` to recover. Reconnection is a future enhancement.

#### 2.3 Team agents (deferred)

Team agents currently pass `mcp_manager: None` at `teams/engine.rs:334` and `:463`. **Defer team agent MCP to a follow-up PR.** Reasons:
- Adds significant scope (per-agent `McpManager` creation, lifetime management during parallel `JoinSet` execution, concurrent access questions)
- The `rmcp` `RunningService<RoleClient, ()>` concurrent call safety needs verification
- A team of 5 agents with 2 MCP servers each = 10 connections at team init time
- CLI ask/chat is the higher-value integration point

Document `mcp_manager: None` in team engine as a known limitation with a `// TODO: support per-team-agent MCP servers` comment.

### Phase 3: Enable/Disable CLI Commands

**Files to modify:**

- `crates/mika-cli/src/cli.rs` -- Add `Enable` and `Disable` variants to `McpCommand`
- `crates/mika-cli/src/commands/mcp.rs` -- Implement toggle logic

#### 3.1 CLI definition

```rust
// crates/mika-cli/src/cli.rs
pub enum McpCommand {
    List,
    Add { /* existing */ },
    Remove { name: String },
    /// Enable a configured MCP server
    Enable { name: String },
    /// Disable a configured MCP server
    Disable { name: String },
}
```

#### 3.2 Implementation

```rust
// crates/mika-cli/src/commands/mcp.rs
fn toggle_server(agent_home: &Path, name: &str, enabled: bool) -> Result<()> {
    let mut config = McpConfig::load(agent_home)?;
    let server = config.mcp_servers.get_mut(name)
        .ok_or_else(|| anyhow::anyhow!("MCP server '{name}' not found."))?;
    server.enabled = enabled;
    config.save(agent_home)?;
    let state = if enabled { "enabled" } else { "disabled" };
    println!("MCP server '{name}' {state}. Restart Mika to apply.");
    Ok(())
}
```

## Acceptance Criteria

### Functional Requirements

- [x] `headers` field in `mcp.json` is parsed and applied to HTTP transport connections
- [x] `Authorization` header works for authenticated MCP servers
- [x] Custom headers (e.g., `X-API-Key`) are sent with HTTP transport requests
- [x] `mika ask` connects to configured MCP servers and can use MCP tools
- [x] `mika` (chat mode) connects to MCP servers for the session lifetime
- [x] Team agents: `mcp_manager: None` remains (deferred)
- [x] `mika mcp enable <name>` sets `enabled: true` and saves
- [x] `mika mcp disable <name>` sets `enabled: false` and saves
- [x] `mika mcp add --header KEY=VALUE` stores headers in config
- [ ] `mika mcp list` shows header count (not values) for HTTP servers
- [x] Existing `mcp.json` files without `headers` field still parse correctly

### Non-Functional Requirements

- [x] Header values are not logged at INFO level (may contain secrets)
- [x] Invalid headers are warned and skipped (graceful degradation)
- [x] MCP connections are properly shut down on CLI exit
- [x] No regression in server mode MCP behavior
- [x] All existing tests pass

### Quality Gates

- [x] Unit tests for headers config parsing (with/without headers, invalid headers)
- [x] Unit tests for enable/disable toggle
- [x] Integration pattern: ask mode creates and tears down MCP per invocation
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## Dependencies & Prerequisites

| Dependency | Version | Purpose |
|-----------|---------|---------|
| `rmcp` | 0.17 | Already in use, `StreamableHttpClientTransportConfig` |
| `reqwest` | 0.12 | Already in use, custom client with default headers |

No new dependencies required.

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| rmcp `with_client()` API differs from docs | Medium | Medium | Verify API against actual crate source; fall back to custom `StreamableHttpClient` impl |
| Header secrets logged in debug output | Low | Medium | Don't log header values at INFO; derive Debug is OK since env field has same exposure |
| Chat mode MCP connection drops mid-session | Low | Medium | Existing graceful degradation: tool calls return error, agent continues |
| Team engine becomes async at init | Low | Low | `TeamEngine::new()` is already called in async context |

## Implementation Order

1. **Phase 1** (Headers) -- Config change + HTTP transport enhancement + CLI flag
2. **Phase 3** (Enable/Disable) -- Small, independent, can ship with Phase 1
3. **Phase 2** (CLI Integration) -- Larger change, depends on Phase 1 for full value

Phase 1+3 can be one PR, Phase 2 a second PR. Or all in one if the diff is manageable.

### Phase 4: Bundled Skill Update

**File to modify:**

- `templates/skills/mcp/system_prompt.md` -- Update to document headers config and enable/disable commands

Add documentation for:
- `headers` field in `mcp.json` with examples
- `mika mcp enable <name>` and `mika mcp disable <name>` commands
- `--header` flag on `mika mcp add`

## Edge Cases (from SpecFlow Analysis)

1. **Headers on stdio transport:** CLI `add --header` with `--transport stdio` errors immediately. Manual `mcp.json` edit with `headers` on stdio: `validate()` logs a warning, ignores the field.
2. **Empty headers map:** `"headers": {}` is valid, treated as no headers.
3. **Invalid header values:** Characters outside visible ASCII in header values cause `HeaderValue::from_str()` to fail. Warn and skip that header, connect without it.
4. **Enable already-enabled server:** `mika mcp enable foo` when `foo.enabled == true` — succeeds silently (idempotent), prints confirmation.
5. **Disable already-disabled server:** Same — idempotent, prints confirmation.
6. **No mcp.json in CLI mode:** `McpConfig::load()` returns empty config, `mcp_manager` is `None`, agent works exactly as before.
7. **Claude Desktop compatibility:** Adding `headers` extends beyond Claude Desktop convention. Claude Desktop should ignore unknown fields, so portability is maintained.
8. **MCP startup in ask mode:** Adds up to 30s latency (parallel JoinSet). Acceptable for one-shot mode which already requires API calls.
9. **Chat mode MCP crash:** Server crash mid-session returns errors for that server's tools. Agent continues with other tools. No reconnection — restart required.

## References

### Internal

- MCP config: `crates/mika-agent/src/mcp/config.rs`
- MCP manager: `crates/mika-agent/src/mcp/mod.rs`
- HTTP connect: `crates/mika-agent/src/mcp/mod.rs:292` (`connect_http`)
- Server init: `crates/mika-agent/src/server/mod.rs:97` (MCP loading pattern)
- CLI ask: `crates/mika-cli/src/commands/ask.rs:56` (`mcp_manager: None`)
- CLI chat: `crates/mika-cli/src/commands/chat.rs:139` (`mcp_manager: None`)
- Team engine: `crates/mika-agent/src/teams/engine.rs:334` (`mcp_manager: None`)
- CLI MCP commands: `crates/mika-cli/src/commands/mcp.rs`
- CLI args: `crates/mika-cli/src/cli.rs:217` (`McpArgs`)
- Original MCP plan: `docs/plans/2026-02-28-feat-mcp-server-support-plan.md`

### External

- [rmcp StreamableHttpClientTransportConfig](https://docs.rs/rmcp/latest/rmcp/transport/streamable_http_client/struct.StreamableHttpClientTransportConfig.html)
- [MCP Specification](https://modelcontextprotocol.io/specification/2025-11-25)

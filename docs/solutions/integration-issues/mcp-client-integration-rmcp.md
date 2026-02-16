---
title: "MCP Client Integration with rmcp Crate"
date: 2026-02-28
category: integration-issues
tags:
  - mcp
  - rmcp
  - model-context-protocol
  - tool-dispatch
  - rust
  - async
severity: medium
component: mika-agent
related_files:
  - crates/mika-agent/src/mcp/mod.rs
  - crates/mika-agent/src/mcp/config.rs
  - crates/mika-agent/src/agent.rs
  - crates/mika-cli/src/commands/mcp.rs
---

# MCP Client Integration with rmcp Crate

## Problem Statement

Mika needed to connect to external MCP (Model Context Protocol) servers and expose their tools alongside builtin and skill tools during the agent loop. This required integrating the `rmcp` crate (v0.17, official Rust MCP SDK) for stdio and Streamable HTTP transports, with tool namespacing to avoid collisions, graceful degradation when no servers are configured, and CLI management commands.

### Symptoms

- No way to extend Mika's tool set without writing Rust code
- Users couldn't leverage the growing MCP ecosystem of servers
- No standard protocol for external tool integration

## Investigation Steps

1. Evaluated `rmcp` crate API surface — `ServiceExt`, `ClientHandler`, transport types
2. Studied Claude Desktop MCP config format (`mcpServers` JSON convention)
3. Tested stdio transport with `TokioChildProcess` and HTTP with `StreamableHttpClientTransport`
4. Explored tool name collision scenarios between multiple MCP servers and builtins

## Root Cause

This was a greenfield feature, not a bug fix. The core challenge was mapping rmcp's type system to Mika's existing tool dispatch chain cleanly.

## Solution

### 1. Configuration Layer (`mcp/config.rs`)

Adopted Claude Desktop's `mcp.json` format at `{agent_home}/mcp.json`:

```json
{
  "mcpServers": {
    "filesystem": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"],
      "env": {},
      "enabled": true
    }
  }
}
```

Key validation rules:
- Server names: lowercase alphanumeric, hyphens, single underscores (no `__` — reserved for namespacing)
- HTTP URLs: must use `http://` or `https://` scheme (prevents SSRF via `ftp://`, `file://`, etc.)
- Stdio transport requires `command`; HTTP requires `url`

### 2. Connection Manager (`mcp/mod.rs`)

`McpManager` connects to all enabled servers on startup using `tokio::task::JoinSet` for parallel connections:

```rust
let mut join_set = tokio::task::JoinSet::new();
for (name, server_config) in config.enabled_servers() {
    server_config.validate(&name)?;
    let name = name.to_string();
    let server_config = server_config.clone();
    join_set.spawn(async move {
        let result = connect_server(&name, &server_config).await;
        (name, result)
    });
}
```

Failed connections are logged as warnings — the system degrades gracefully.

### 3. Tool Namespacing

MCP tools are namespaced as `mcp__{server_name}__{tool_name}` using double-underscore separators. This prevents collisions between:
- Tools from different MCP servers
- MCP tools and builtin/skill tools

Duplicate tool names within a single server are detected and skipped with a warning.

### 4. Dispatch Chain in Agent Loop

The agent loop dispatches tool calls through a three-tier chain:

```
builtins → skills → MCP → unknown error
```

MCP dispatch in `agent.rs` uses edition 2024 let-chain syntax:

```rust
if let Some(mcp) = mcp_manager
    && mcp.is_mcp_tool(name)
{
    return match tokio::time::timeout(
        Duration::from_secs(TOOL_TIMEOUT_SECS),
        mcp.call_tool(name, input),
    ).await {
        Ok(output) => output,
        Err(_) => ToolOutput::error(format!("MCP tool '{name}' timed out")),
    };
}
```

### 5. Result Conversion

`convert_mcp_result` maps rmcp's `CallToolResult` to Mika's `ToolOutput`:

- **Text content**: Concatenated, char-boundary-safe truncation at 10,000 chars
- **Image content**: Base64-encoded with limits (max 5 images, 5MB each)
- **Error flag**: Maps to `ToolOutput::error` vs `ToolOutput::success`

### 6. Environment Sandboxing

Stdio child processes are spawned with `env_clear()` plus an explicit allowlist (`PATH`, `HOME`, `USER`, `LANG`, `TERM`). Server-configured env vars are applied, but `MIKA_*` prefixed vars are blocked to prevent config injection.

## What Didn't Work

### rmcp API Type Inference

Rust could not infer types for `().serve(transport)` because `()` implements `ClientHandler` but the generic bounds were ambiguous. Required UFCS (Uniform Function Call Syntax):

```rust
// Won't compile:
// ().serve(transport).await

// Works:
<() as ServiceExt<RoleClient>>::serve((), transport).await
```

### rmcp Content Type Structure

rmcp wraps content in `Annotated<RawContent>`, not plain enums. Initial code tried to match on `Content` directly. The correct pattern:

```rust
for item in &result.content {
    match &item.raw {
        RawContent::Text(text_content) => { /* ... */ }
        RawContent::Image(img) => { /* ... */ }
        _ => {}
    }
}
```

### Environment Variable Leaking

Initial implementation passed through all parent env vars to child processes. Code review identified this as a security risk — sensitive vars like `MIKA_ANTHROPIC_API_KEY` would be visible to MCP server processes. Fixed with `env_clear()` + allowlist pattern.

### Sequential Startup

Initial implementation connected to MCP servers sequentially, blocking on each connection. For users with multiple servers, this caused slow startup. Refactored to `JoinSet` for parallel connections.

## Prevention Strategies

1. **Always validate external input at system boundaries**: Server names, URLs, and env vars are all untrusted input from config files
2. **Use `env_clear()` for child processes**: Never inherit the full parent environment when spawning external processes
3. **Namespace collision prevention**: Use reserved separator sequences (`__`) and validate names against them
4. **Graceful degradation pattern**: Feature should work with 0 servers (skip entirely), partial failures (log and continue), or full connectivity
5. **Char-boundary-safe string operations**: Always check `is_char_boundary()` before truncating Rust strings — multi-byte UTF-8 will panic otherwise
6. **Parallel I/O with JoinSet**: Use `tokio::task::JoinSet` instead of sequential awaits for independent async operations

## Related References

- [MCP Specification](https://modelcontextprotocol.io/)
- [rmcp crate](https://crates.io/crates/rmcp) (v0.17)
- PR #37: feat/mcp-server-support
- Plan: `docs/plans/2026-02-28-feat-mcp-server-support-plan.md`

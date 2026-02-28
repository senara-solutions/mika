---
title: "feat: Add MCP Server Support"
type: feat
status: completed
date: 2026-02-28
---

# feat: Add MCP (Model Context Protocol) Server Support

## Overview

Add MCP client support to mika-agent so Mika can connect to external MCP servers
and expose their tools to Claude during the agent loop. This enables users to
extend Mika's capabilities with any MCP-compatible tool server (filesystem access,
database queries, web scraping, code analysis, etc.) without writing custom skill
handlers.

## Problem Statement / Motivation

Mika currently supports three tool handler types: builtin (Rust functions), exec
(shell scripts), and http (external HTTP endpoints). While exec and http handlers
enable extensibility, they require users to write handler scripts or host API
endpoints. MCP is the emerging standard for LLM tool servers with a growing
ecosystem of pre-built servers. Supporting MCP lets users plug into this ecosystem
immediately.

## Proposed Solution

Add MCP client integration using the `rmcp` crate (v0.17, official Rust MCP SDK).
Support stdio (child process) and Streamable HTTP transports. MCP servers are
configured via `~/.mika/mcp.json`, connected on startup, and their tools are
merged into the tool set alongside builtins and skill tools.

### Architecture

```
                    ┌──────────────────────────────┐
                    │        Agent Loop             │
                    │  (crates/mika-agent/agent.rs) │
                    └──────┬───────────────────────┘
                           │ execute_tool()
              ┌────────────┼────────────────┐
              │            │                │
              v            v                v
        ┌──────────┐ ┌──────────┐   ┌────────────┐
        │ Builtin  │ │  Skill   │   │    MCP     │
        │ Registry │ │  Tools   │   │  Manager   │
        │ (15+)    │ │ (exec/   │   │            │
        └──────────┘ │  http/   │   └──────┬─────┘
                     │  builtin)│          │
                     └──────────┘   ┌──────┴──────┐
                                    │  rmcp crate │
                                    └──────┬──────┘
                                    ┌──────┴──────┐
                               ┌────┴───┐  ┌──────┴──────┐
                               │ stdio  │  │ Streamable  │
                               │(child  │  │    HTTP     │
                               │process)│  │             │
                               └────────┘  └─────────────┘
```

### Key Design Decisions

1. **Config in `~/.mika/mcp.json`** -- Separate JSON file (not TOML). Matches the
   convention used by Claude Desktop, VS Code MCP extensions, and other MCP clients.
   Avoids TOML serialization complexity for programmatic CLI edits.

2. **`McpManager` struct** -- Holds all MCP client connections (`HashMap<String,
   RunningService>`). Created at startup, passed into the agent loop. `Send + Sync`
   via `Arc<McpManager>`.

3. **Tool name namespacing** -- MCP tools are prefixed with `mcp__<server_name>__`
   (double underscore separator) to prevent collisions with builtins and skill
   tools. Example: server "filesystem" tool "read_file" becomes
   `mcp__filesystem__read_file`. Claude sees the full name; the dispatch strips the
   prefix to route to the correct server.

4. **No new `ToolHandler::Mcp` variant** -- MCP tools are NOT defined via skill
   manifests. They are dynamically discovered from MCP servers at startup and
   injected directly into the tool definitions list. The `McpManager` handles
   dispatch independently from the skill system.

5. **Silent mode exclusion** -- MCP tools are filtered out during silent/heartbeat
   mode, consistent with exec/http handler filtering in `safe_always_on_skills()`.

6. **Persistent connections** -- MCP server connections are held open for the agent
   lifetime. Lazy reconnection on failure. For CLI `ask` mode (one-shot), connections
   are created and torn down per invocation.

7. **Graceful degradation** -- If no MCP servers are configured or all fail to
   connect, the agent works exactly as before. Failed servers log warnings and are
   marked as disconnected; their tools are excluded from the tool set.

## Technical Approach

### Phase 1: MCP Config + Client Manager

**Files to create/modify:**

- [x] `crates/mika-agent/src/mcp/mod.rs` -- New module, `McpManager` struct
- [x] `crates/mika-agent/src/mcp/config.rs` -- `McpServerConfig` struct, JSON loading
- [x] `crates/mika-agent/src/mcp/client.rs` -- Per-server connection wrapper (merged into mod.rs)
- [x] `Cargo.toml` (workspace + mika-agent) -- Add `rmcp` dependency

#### 1.1 Add rmcp dependency

```toml
# crates/mika-agent/Cargo.toml
[dependencies]
rmcp = { version = "0.17", default-features = false, features = [
    "client",
    "transport-child-process",
    "transport-streamable-http-client",
    "transport-streamable-http-client-reqwest",
] }
```

#### 1.2 MCP config schema (`~/.mika/mcp.json`)

```json
{
  "mcpServers": {
    "filesystem": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user/docs"],
      "env": { "NODE_ENV": "production" },
      "enabled": true
    },
    "github": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_..." },
      "enabled": true
    },
    "remote-db": {
      "transport": "http",
      "url": "http://localhost:8000/mcp",
      "enabled": true
    }
  }
}
```

Config struct:

```rust
// crates/mika-agent/src/mcp/config.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub transport: McpTransport,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub url: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
}
```

Loading: `McpConfig::load(home_dir)` reads `{home_dir}/mcp.json`. Returns
`McpConfig::default()` (empty servers) if file does not exist.

#### 1.3 McpManager

```rust
// crates/mika-agent/src/mcp/mod.rs
pub struct McpManager {
    clients: HashMap<String, McpClient>,
    tool_definitions: Vec<ToolDefinition>,
    tool_routing: HashMap<String, String>, // namespaced_name -> server_name
}

impl McpManager {
    /// Connect to all enabled MCP servers. Failures are logged and skipped.
    pub async fn connect_all(config: &McpConfig) -> Self { ... }

    /// Get tool definitions for all connected servers (namespaced names).
    pub fn tool_definitions(&self) -> &[ToolDefinition] { ... }

    /// Execute an MCP tool call. Returns ToolOutput.
    pub async fn call_tool(&self, namespaced_name: &str, input: Value) -> ToolOutput { ... }

    /// Check if a tool name belongs to an MCP server.
    pub fn is_mcp_tool(&self, name: &str) -> bool { ... }

    /// Graceful shutdown of all connections.
    pub async fn shutdown(&self) { ... }

    /// Get status of all configured servers.
    pub fn status(&self) -> Vec<McpServerStatus> { ... }
}
```

#### 1.4 Per-server client wrapper

```rust
// crates/mika-agent/src/mcp/client.rs
pub struct McpClient {
    name: String,
    service: RunningService<RoleClient, ()>,
    tools: Vec<rmcp::model::Tool>,
}

impl McpClient {
    pub async fn connect_stdio(name: &str, config: &McpServerConfig) -> Result<Self> { ... }
    pub async fn connect_http(name: &str, config: &McpServerConfig) -> Result<Self> { ... }
    pub async fn call_tool(&self, tool_name: &str, args: Value) -> Result<ToolOutput> { ... }
    pub async fn shutdown(self) { ... }
}
```

Tool definition conversion: MCP `Tool` → Claude `ToolDefinition`:
- `name`: Prefix with `mcp__<server_name>__`
- `description`: Use MCP tool description, prepend `[MCP: <server_name>]`
- `input_schema`: Use MCP tool `inputSchema` directly (already JSON Schema)

Result conversion: MCP `CallToolResult` → `ToolOutput`:
- Concatenate all `Content::Text` blocks into `content` string
- Convert `Content::Image` blocks to `ImageData` (base64 + media type)
- Set `is_error` from `result.is_error`
- Truncate to `MAX_OUTPUT_LEN` (10,000 chars)

### Phase 2: Agent Loop Integration

**Files to modify:**

- [x] `crates/mika-agent/src/agent.rs` -- Add MCP tools to definitions, dispatch MCP calls
- [x] `crates/mika-agent/src/tools/mod.rs` -- Passed as separate param (not ToolContext)
- [x] `crates/mika-agent/src/server/state.rs` -- Store `McpManager` in `AgentState`
- [x] `crates/mika-agent/src/skills/mod.rs` -- MCP tools excluded in silent mode via None

#### 2.1 Tool definitions injection

In `inject_skills_and_resolve_tools()` (agent.rs ~line 1177), after merging
builtin + skill tools, append MCP tool definitions:

```rust
// After skill tools are merged...
if let Some(mcp) = mcp_manager {
    for def in mcp.tool_definitions() {
        if !seen_tools.contains(&def.name) {
            seen_tools.insert(def.name.clone());
            all_definitions.push(def.clone());
        }
    }
}
```

#### 2.2 Tool dispatch

In `execute_tool()` (agent.rs ~line 792), add a third dispatch path after
builtin and skill lookups:

```rust
// After builtin tool lookup...
// After skill tool lookup...
// MCP tool dispatch
if let Some(mcp) = mcp_manager {
    if mcp.is_mcp_tool(tool_name) {
        return mcp.call_tool(tool_name, input).await;
    }
}
// Unknown tool error
```

#### 2.3 ToolContext extension

Add `mcp_manager: Option<&'a McpManager>` to `ToolContext`. However, since MCP
dispatch happens in `execute_tool()` before `ToolContext` is used, the MCP manager
can instead be passed directly to `execute_tool()` and `inject_skills_and_resolve_tools()`
as a separate parameter, avoiding changes to `ToolContext`.

Decision: **Pass `Option<&McpManager>` as a separate parameter** to `execute_tool()`
and `inject_skills_and_resolve_tools()` rather than adding it to `ToolContext`.
This is cleaner because MCP tools don't need `ToolContext` (no DB, no session).

#### 2.4 Silent mode filtering

In `run_silent_agent()`, do NOT pass the `McpManager` -- pass `None` instead.
This excludes all MCP tools from heartbeat/reminder mode, consistent with
exec/http handler filtering.

#### 2.5 Timeout

Wrap MCP tool calls with the same `TOOL_TIMEOUT_SECS` (30s) timeout used for
all other tools:

```rust
tokio::time::timeout(Duration::from_secs(TOOL_TIMEOUT_SECS), mcp.call_tool(name, input)).await
```

### Phase 3: Bundled MCP Skill

**Files to create:**

- [x] `templates/skills/mcp/skill.toml` -- Skill manifest
- [x] `templates/skills/mcp/system_prompt.md` -- Configuration guide for users
- [x] `crates/mika-agent/src/bundled_skills.rs` -- Register the new bundled skill

#### 3.1 Skill manifest

```toml
[skill]
name = "mcp"
description = "MCP server configuration and management"
version = "0.1.0"
always_on = false

[triggers]
keywords = ["mcp", "model context protocol", "mcp server", "mcp tool"]
```

No `tools.json` -- this is a prompt-only skill (informational).

#### 3.2 System prompt

The `system_prompt.md` will explain:
- What MCP is and why it's useful
- How to configure servers in `~/.mika/mcp.json`
- Example configurations for common MCP servers
- How to use `mika mcp list|add|remove` CLI commands
- Troubleshooting (connection failures, tool not appearing)

### Phase 4: CLI Commands

**Files to modify:**

- [x] `crates/mika-cli/src/main.rs` -- Add `mcp` subcommand
- [x] `crates/mika-cli/src/commands/mod.rs` -- Register module
- [x] `crates/mika-cli/src/commands/mcp.rs` -- New file with subcommands

#### 4.1 CLI subcommands

```
mika mcp list              # Show configured servers and their status
mika mcp add <name>        # Interactive: add a new MCP server
  --transport stdio|http
  --command <cmd>
  --args <arg1> <arg2>...
  --url <url>
  --env KEY=VALUE
mika mcp remove <name>     # Remove an MCP server from config
```

`mcp list` output format:

```
MCP Servers (~/.mika/mcp.json):

  filesystem     stdio  npx -y @modelcontextprotocol/server-filesystem  enabled
  github         stdio  npx -y @modelcontextprotocol/server-github      enabled
  remote-db      http   http://localhost:8000/mcp                       disabled
```

`mcp add` writes to `~/.mika/mcp.json` (creates if doesn't exist).
`mcp remove` removes the server entry and rewrites the file.

### Phase 5: Testing

**Files to create:**

- [x] `crates/mika-agent/src/mcp/tests.rs` -- Unit tests (inline in mod.rs and config.rs)

#### 5.1 Unit tests

- Config parsing: valid JSON, missing file, malformed JSON, disabled servers
- Tool name namespacing: prefix/strip roundtrip, collision detection
- Result conversion: text-only, text+image, error results, empty results
- Timeout handling: mock slow server, verify timeout error
- Graceful degradation: no config file, all servers fail to connect

#### 5.2 Integration testing approach

Since MCP servers are external processes, integration tests would require
either:
- A test MCP server binary (could use rmcp's server features to build one)
- Mocking at the rmcp transport level

For the initial implementation, focus on unit tests with mocked MCP responses.

## Acceptance Criteria

### Functional Requirements

- [x] MCP servers can be configured via `~/.mika/mcp.json`
- [x] Stdio transport: Mika spawns child process, communicates via stdin/stdout
- [x] HTTP transport: Mika connects to remote URL via Streamable HTTP
- [x] MCP tools appear in Claude's tool list with namespaced names
- [x] Claude can call MCP tools and receives results
- [x] MCP tool results support text and image content
- [x] Failed MCP connections log warnings and are skipped (agent still works)
- [x] MCP tools are excluded from silent/heartbeat mode
- [x] `mika mcp list` shows configured servers
- [x] `mika mcp add` adds a server to config
- [x] `mika mcp remove` removes a server from config
- [x] Bundled "mcp" skill responds to MCP-related questions with config guidance
- [x] Agent works identically when no MCP servers are configured (no regression)

### Non-Functional Requirements

- [x] MCP server connection failures do not prevent agent startup
- [x] MCP tool calls respect the 30s per-tool timeout
- [x] MCP tool output is truncated to 10,000 chars (MAX_OUTPUT_LEN)
- [x] Child processes are cleaned up on agent shutdown
- [x] No environment variable leakage beyond what's explicitly configured per server
- [x] All tests pass (`cargo test`)
- [x] Clippy clean (`cargo clippy`)

### Quality Gates

- [x] Unit tests for config parsing, name namespacing, result conversion
- [x] Existing tests unaffected (no regressions)
- [ ] CLAUDE.md updated with MCP architecture details
- [ ] Documentation updated (configuration.md, architecture.md)

## Dependencies & Prerequisites

| Dependency | Version | Purpose |
|-----------|---------|---------|
| `rmcp` | 0.17 | Official Rust MCP SDK |
| `tokio` | 1.x | Already in use, required by rmcp |
| `serde_json` | 1.x | Already in use, MCP config parsing |
| `reqwest` | 0.12 | Already in use, needed for HTTP transport |

No new runtime dependencies beyond `rmcp` (which pulls in some transitive deps).

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| rmcp API breaking changes | Low | Medium | Pin to 0.17.x, review changelogs before upgrading |
| MCP server crashes during tool call | Medium | Low | Timeout wrapping, error handling, graceful degradation |
| Tool name collisions | Medium | Low | Double-underscore namespacing prevents all collisions |
| Large Docker image increase | Low | Low | rmcp is pure Rust, minimal binary size impact |
| Child process zombie leaks | Medium | Medium | Use `kill_on_drop`, explicit shutdown in Drop impl |
| MCP server exfiltrates env vars | Low | High | Only pass explicitly configured `env` vars to child process, not inherit |

## Implementation Order

1. **Phase 1** (config + client manager) -- Foundation, no behavior change
2. **Phase 2** (agent loop integration) -- Core functionality
3. **Phase 3** (bundled skill) -- User guidance
4. **Phase 4** (CLI commands) -- Management UX
5. **Phase 5** (testing) -- Continuous throughout, final sweep at end

## References

### Internal

- Tool dispatch: `crates/mika-agent/src/agent.rs:792` (`execute_tool`)
- Tool definitions: `crates/mika-agent/src/agent.rs:1177` (`inject_skills_and_resolve_tools`)
- Tool registry: `crates/mika-agent/src/tools/mod.rs:154` (`ToolRegistry`)
- ToolHandler enum: `crates/mika-agent/src/skills/manifest.rs:48`
- Executor: `crates/mika-agent/src/skills/executor.rs:72` (`execute_inner`)
- Builtin handlers: `crates/mika-agent/src/skills/builtin_handlers.rs:44`
- Silent mode filter: `crates/mika-agent/src/skills/mod.rs:63` (`safe_always_on_skills`)
- Server state: `crates/mika-agent/src/server/state.rs:32` (`AppState`)
- Bundled skills: `crates/mika-agent/src/bundled_skills.rs`
- Config: `crates/mika-common/src/config.rs`

### External

- [rmcp crate (v0.17)](https://crates.io/crates/rmcp)
- [rmcp docs.rs](https://docs.rs/rmcp/latest/rmcp/)
- [Official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)
- [MCP Specification (2025-11-25)](https://modelcontextprotocol.io/specification/2025-11-25)
- [MCP Tools Spec](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)

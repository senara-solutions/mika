# Plan — mika#1450: Extract `tool_execution/` into its own module directory

**Parent:** mika#1259 (Layer 3 domain refactor)
**Foundation:** `docs/architecture/operational-partner-frame.md` §6 — `tool_execution/` owns "Tool dispatch, MCP integration, exec handlers, dispatch gates."
**Decomposition plan:** `docs/plans/2026-06-08-001-meta-1259-decomposition-plan.md` — sub-issue #1259-B, sequenced 7th (depends on evidence/ #1259-A, but this ticket is independently extractable since evidence/ doesn't exist yet — cross-module imports will use `crate::` paths to the current locations).

## Scope

Create `crates/mika-agent/src/tool_execution/` module directory and relocate tool dispatch, execution, and related types from `agent.rs` into it. Pure module split — zero behavior change.

## What moves

### From `agent.rs` (~400 lines)

| Item | Current location | Target file | Notes |
|------|-----------------|-------------|-------|
| `ToolCallSummary` struct | agent.rs:287–299 | `tool_execution/types.rs` | Public type used by post_condition, agent loop, server |
| `truncate_summary()` helper | agent.rs:325–337 | `tool_execution/types.rs` | Used by process_tool_calls for input/output summaries |
| `serialize_tool_metadata()` + tests | agent.rs:339–403 | `tool_execution/types.rs` | Serializes summaries to JSON metadata |
| `TOOL_METADATA_MAX`, `INPUT_SUMMARY_MAX` constants | agent.rs:314–316 | `tool_execution/types.rs` | Used by serialize_tool_metadata and process_tool_calls |
| `TOOL_TIMEOUT_SECS`, `TOOL_TIMEOUT_INPUT_EXCERPT_LEN`, `MAX_IMAGE_BYTES_PER_STEP` constants | agent.rs:38–58 | `tool_execution/dispatch.rs` | Used only by process_tool_calls and execute_tool |
| `process_tool_calls()` fn | agent.rs:3119–3390 | `tool_execution/dispatch.rs` | Main orchestration: dedup, routing, output collection, DB persistence |
| `ToolDispatchCtx` struct | agent.rs:3393–3400 | `tool_execution/dispatch.rs` | Bundle of dispatcher resources |
| `execute_tool()` fn | agent.rs:3406–3498 | `tool_execution/dispatch.rs` | Single-tool executor: builtin → skill → MCP → error |

### What stays

| Item | Location | Reason |
|------|----------|--------|
| `SilentTrigger` enum | agent.rs:3548–3599 | Agent-loop-scoped; not tool-execution concern |
| `run_loop()` / `run_agent()` | agent.rs | Agent-loop domain (future #1259-H) |
| `send_message_boundary_active` flag logic | agent.rs:3168 | Intra-step gating passed into process_tool_calls via parameters; the flag state belongs to the agent loop, the check is inside process_tool_calls |
| `ToolOutput` struct | tools/mod.rs:188 | Already in `tools/` module; no reason to relocate |
| `ToolRegistry` | tools/mod.rs | Already in `tools/` module |
| `skills/executor.rs` | skills/ | Skill execution is called FROM tool_execution via `executor::execute_skill_tool()`. Moving the entire 5,623-line executor is out of scope — it has deep coupling to skills/manifest, skills/builtin_handlers, and the long-running task engine. The decomposition plan (#1259-B) estimates ~3,500 LoC for tool_execution, but skill executor internals are better addressed as a cross-cutting concern when evidence/ and planning/ modules also exist. |
| `mcp/` module | mcp/ | Already a module; `tool_execution/dispatch.rs` calls `mcp.is_mcp_tool()` and `mcp.call_tool()` via the existing `McpManager` interface |
| `db.rs` tool_calls functions | db.rs:6480–7060 | DB read/write methods stay in db.rs (read-side is dashboard concern per #1259-I; write-side is called from process_tool_calls via `db.save_tool_call()` — the call crosses the module boundary, which is the correct shape) |
| `bundled_skills.rs` parity test | bundled_skills.rs:1425 | Foundation §6 says tool_execution "owns the loader-engine parity check from #1253." However, the test at bundled_skills.rs:1425 tests skill-loader reachability, not tool-execution dispatch. Moving it would break its logical home (it depends on `all_bundled_skills()` and skill manifest parsing). A future cross-cutting pass can add a tool_execution-side parity assertion if needed. |

## Module structure

```
crates/mika-agent/src/tool_execution/
├── mod.rs          — Module doc-comment with operational responsibility; re-exports
├── dispatch.rs     — process_tool_calls(), execute_tool(), ToolDispatchCtx, dispatch constants
└── types.rs        — ToolCallSummary, serialize_tool_metadata(), truncate_summary(), type constants
```

### `mod.rs`

```rust
//! Tool execution — dispatch, MCP integration, exec handlers, dispatch gates.
//!
//! Operational responsibility (per Foundation §6): routes tool-use blocks from
//! LLM responses through the three-tier dispatch chain (builtin → skill → MCP),
//! manages per-tool timeouts, per-turn dedup, image-budget accounting, and
//! persists tool-call metadata to SQLite.

pub mod dispatch;
pub mod types;

// Re-export primary public types for ergonomic use from agent.rs
pub use dispatch::process_tool_calls;
pub use types::{ToolCallSummary, serialize_tool_metadata};
```

## Implementation steps

### Step 1 — Create module directory and files

1. Create `crates/mika-agent/src/tool_execution/mod.rs` with the doc-comment above.
2. Create `crates/mika-agent/src/tool_execution/types.rs` — move:
   - `ToolCallSummary` struct (agent.rs:287–299)
   - `truncate_summary()` (agent.rs:325–337)
   - `TOOL_METADATA_MAX`, `INPUT_SUMMARY_MAX` constants (agent.rs:314–316)
   - `serialize_tool_metadata()` + its tests (agent.rs:339–403)
3. Create `crates/mika-agent/src/tool_execution/dispatch.rs` — move:
   - `TOOL_TIMEOUT_SECS`, `TOOL_TIMEOUT_INPUT_EXCERPT_LEN`, `MAX_IMAGE_BYTES_PER_STEP` constants (agent.rs:38–58)
   - `ToolDispatchCtx` struct (agent.rs:3393–3400)
   - `execute_tool()` fn (agent.rs:3406–3498)
   - `process_tool_calls()` fn (agent.rs:3119–3390)

### Step 2 — Update imports in moved code

`dispatch.rs` needs:
- `use crate::tools::ToolRegistry` (for `dispatch.tools.get()`)
- `use crate::tools::{ToolContext, ToolOutput}` (for execute/return types)
- `use crate::skills::executor` (for `execute_skill_tool`)
- `use crate::skills::builtin_handlers` (for `execute` on builtin handlers)
- `use crate::skills::manifest::ToolHandler` (for `ToolHandler::Builtin` match)
- `use crate::skills::index::ResolvedSkillTool` (for skill tool lookups)
- `use crate::mcp::McpManager` (for MCP dispatch)
- `use crate::async_db::AsyncDatabase` (for `db.save_tool_call()`)
- `use crate::secret_scrubber::scrub_secrets` (for metadata scrubbing)
- `use super::types::{ToolCallSummary, truncate_summary, INPUT_SUMMARY_MAX, TOOL_METADATA_MAX}` (for summary construction)
- `use mika_common::llm::*` types (LlmResponseContent, LlmContentBlock, etc.)
- `use tracing::{debug, warn, info}` macros
- `use std::collections::HashMap`
- `use serde_json::Value`

`types.rs` needs:
- `use crate::secret_scrubber::scrub_secrets` (used by serialize_tool_metadata)
- `use serde::{Serialize, Deserialize}` (for ToolCallSummary derive)
- `use tracing::warn` (for metadata serialization overflow)

### Step 3 — Update `lib.rs`

Add `pub mod tool_execution;` declaration to `crates/mika-agent/src/lib.rs` in alphabetical position (between `timestamp` and `tools`).

### Step 4 — Update `agent.rs` imports and call sites

1. Remove the moved items from agent.rs.
2. Add `use crate::tool_execution::{process_tool_calls, ToolCallSummary, serialize_tool_metadata};` at the top of agent.rs.
3. The call site at agent.rs:2292 (`let step_summaries = process_tool_calls(...)`) remains unchanged — same function, new module path.
4. Any other internal references to `ToolCallSummary` or `serialize_tool_metadata` in agent.rs use the new import.

### Step 5 — Update external consumers of moved types

Grep for `ToolCallSummary` and `serialize_tool_metadata` across the crate:
- `post_condition.rs` — uses `ToolCallSummary` (for required-tools checks, intent guards, etc.)
- `server/` — uses `ToolCallSummary` in API response types
- Any other file importing from `crate::agent::ToolCallSummary` → update to `crate::tool_execution::ToolCallSummary`

### Step 6 — Verify

1. `cargo build -p mika-agent` — compilation check
2. `cargo test -p mika-agent` — full test suite (per parent AC2)
3. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` — clean clippy

## Cross-module boundaries (post-extraction)

```
agent.rs::run_loop()
    │
    ├──→ tool_execution::process_tool_calls()   # dispatches all tool-use blocks
    │        │
    │        ├──→ tool_execution::execute_tool() # single-tool dispatch
    │        │        │
    │        │        ├──→ tools::ToolRegistry::get()          # builtin tools
    │        │        ├──→ skills::executor::execute_skill_tool() # skill tools
    │        │        ├──→ skills::builtin_handlers::execute()  # builtin handlers
    │        │        └──→ mcp::McpManager::call_tool()        # MCP tools
    │        │
    │        └──→ async_db::save_tool_call()    # persistence
    │
    └──→ post_condition guards                  # consume ToolCallSummary
```

## Dependency note (sequencing)

The decomposition plan sequences tool_execution (7th) after evidence/ (1st). However, the dependency is forward-looking — evidence/ will eventually own the fabrication-guard predicates that consume `ToolCallSummary`. Today, those guards live in `post_condition.rs` and `agent.rs`. This extraction is safe to do now; when evidence/ is extracted later, it will import from `tool_execution::types::ToolCallSummary` instead of `agent::ToolCallSummary`. No circular dependency risk.

## Risk

**Low.** This is a pure move-and-re-export refactoring:
- No logic changes, no new code paths
- All existing tests validate behavior unchanged
- The module boundary is clean: one call site in agent.rs, well-defined return types
- The only risk is missing an import update, which the compiler catches immediately

## Out of scope

- Moving `skills/executor.rs` contents (5,623 lines) — too deeply coupled to skills/ internals
- Moving `db.rs` tool_calls read/write functions — read-side is dashboard_queries concern (#1259-I)
- Moving `SilentTrigger` or agent-loop types — agent_loop concern (#1259-H)
- Creating new abstractions or traits — pure relocation per AC3
- Moving the loader-engine parity test from bundled_skills.rs — it tests skill-loader reachability, not dispatch

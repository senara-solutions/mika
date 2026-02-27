---
status: complete
priority: p2
issue_id: "078"
tags: [code-review, architecture, quality]
dependencies: []
---

# Extract shared logic from duplicated agent loops

## Problem Statement
`run_agent_inner` (agent.rs:72-199) and `run_silent_inner` (agent.rs:293-411) share ~80 lines of near-identical logic: soul/identity loading, core memory retrieval, tool definitions, ToolContext creation, MessagesRequest building, and the tool-step loop. Bug fixes must be applied in two places.

## Findings
- Identical code: soul.md loading, identity loading, core memory, tool defs, ToolContext construction, MessagesRequest building, tool-use dispatch loop
- Only differences: prompt type, initial messages, return type, post-turn compaction

## Proposed Solutions
### Option 1: Extract shared tool-step loop
```rust
async fn run_tool_loop(claude, tools, tool_ctx, system, messages) -> Result<(String, StopReason)>
```
Both inner functions call this shared core.
**Effort:** 1-2 hours | **Risk:** Medium (refactoring core loop)

### Option 2: Extract shared setup helper
Keep loops separate, extract `prepare_agent_context()` for common setup.
**Effort:** 45 minutes | **Risk:** Low

## Acceptance Criteria
- [x] No duplicated tool-dispatch loop logic
- [x] Both agent modes share the same core loop
- [x] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)

### 2026-02-27 - Implementation
**By:** Claude Code
**Actions:** Extracted shared helpers from 3 duplicated agent loops:
- `AgentContext` struct + `load_agent_context()` — replaces duplicated soul/identity/core_memory/timezone loading
- `LoopMode` enum (Conversation/Silent/Team) — parameterizes behavioral differences (thinking, usage tracking, follow-up, DB saves)
- `LoopResult` struct — unified return type from the shared loop
- `run_loop()` — single tool-step loop used by all 3 variants
- Each `_inner` function reduced to thin dispatcher: load context → build prompt → call `run_loop` → map result
- 842 → 738 lines (~12% reduction), all 475 tests pass, clippy clean on agent.rs

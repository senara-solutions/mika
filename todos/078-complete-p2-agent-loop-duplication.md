---
status: pending
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
- [ ] No duplicated tool-dispatch loop logic
- [ ] Both agent modes share the same core loop
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)

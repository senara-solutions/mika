---
status: complete
priority: p1
issue_id: "028"
tags: [code-review, agent, rust-v2]
dependencies: []
---

# search_memory Tool Referenced in Prompt But Does Not Exist

## Problem Statement

The system prompt in `prompt.rs` instructs: "Use the search_memory tool to recall relevant past context." But `search_memory` is not registered in `default_tools()`. Claude will attempt to call it, receive an "Unknown tool" error, waste a tool-use step (up to 30s), and may retry multiple times.

**Why it matters:** Directly impacts response latency and API cost. The agent will fail on every attempt to search memory, degrading the user experience.

## Findings

- **Source:** Architecture Strategist (K), Agent-Native Reviewer
- **Locations:**
  - `crates/mika-agent/src/prompt.rs:24` — instruction to use search_memory
  - `crates/mika-agent/src/tools/mod.rs:89-96` — tool not in default_tools()

## Proposed Solutions

### Option A: Remove the prompt instruction (Recommended for now)
- Delete the search_memory line from `build_system_prompt`
- Re-add when the tool is implemented (with sqlite-vec or SQL LIKE)
- **Pros:** Immediate fix, no wasted tool calls
- **Cons:** Agent loses awareness that memory search should exist
- **Effort:** Small
- **Risk:** Low

### Option B: Add a stub search_memory tool
- Implement basic SQL LIKE search over conversations and core_memory
- Register in default_tools()
- **Pros:** Agent can search memory immediately, even without vector search
- **Cons:** LIKE search is limited, may return poor results
- **Effort:** Small-Medium
- **Risk:** Low

## Acceptance Criteria

- [ ] System prompt does not reference tools that are not registered
- [ ] No "Unknown tool" errors during normal agent operation

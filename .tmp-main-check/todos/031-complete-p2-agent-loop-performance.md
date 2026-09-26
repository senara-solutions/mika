---
status: complete
priority: p2
issue_id: "031"
tags: [code-review, performance, rust-v2]
dependencies: []
---

# Agent Loop Clones Everything Per Iteration + No Total Timeout

## Problem Statement

Two related performance issues in the agent loop:
1. Every loop iteration deep-clones `system` (~4KB), `messages` (growing), and `tool_defs` (all JSON schemas). Over a 5-step tool loop, this wastes 250-500KB of allocations.
2. No total timeout on `run_agent` — worst case is 10 * (120s API + 30s tool) = 25 minutes.

**Why it matters:** Unnecessary allocations and unbounded execution time for per-customer containers.

## Findings

- **Source:** Performance Oracle (CRITICAL-3), Architecture Strategist (J), Security Sentinel (M3)
- **Location:** `crates/mika-agent/src/agent.rs:48-61`

## Proposed Solutions

### Option A: Borrow instead of clone + total timeout (Recommended)
- Change `send_message` to accept `&MessagesRequest` (it only serializes to JSON)
- Build request once, push new messages directly into `request.messages`
- Wrap entire `run_agent` in `tokio::time::timeout(Duration::from_secs(300))`
- **Pros:** Eliminates all cloning, bounds total execution
- **Cons:** Requires changing ClaudeClient API signature
- **Effort:** Medium
- **Risk:** Low

### Option B: Cache tool definitions + total timeout only
- Cache tool definitions at registry level (already static)
- Add total timeout
- Keep cloning for messages (simpler)
- **Pros:** Quick win on the static parts
- **Cons:** Still clones messages per iteration
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] Tool definitions built once, not per iteration
- [ ] System prompt not cloned per iteration
- [ ] Total agent loop timeout exists (e.g., 5 minutes)
- [ ] Timeout produces a user-friendly fallback message

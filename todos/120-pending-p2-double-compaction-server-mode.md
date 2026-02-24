---
status: pending
priority: p2
issue_id: "120"
tags: [code-review, performance, architecture]
dependencies: []
---

# Double Compaction in Server Mode

## Problem Statement

In server mode, conversation compaction runs twice per message turn:
1. Inline in `agent.rs` (the agent loop calls `maybe_compact` at the end)
2. Spawned in `handlers.rs:133-137` after the agent loop completes

This is wasteful (two Claude API summarization calls) and could cause race conditions if both run concurrently in edge cases.

## Findings

- **Source:** performance-oracle (CRITICAL-1), architecture-strategist (CRITICAL-1), code-simplicity-reviewer
- **Location:** `crates/mika-agent/src/agent.rs` (inline compaction) and `crates/mika-agent/src/server/handlers.rs:131-137` (spawned compaction)
- **Evidence:** Both call `compaction::maybe_compact(&db, &claude)` — the agent.rs call is inline, handlers.rs spawns it after dropping the lock

## Proposed Solutions

### Option 1: Remove inline compaction from agent.rs, keep only handlers.rs spawned version
- **Pros**: Single compaction path in server mode, runs outside agent lock
- **Cons**: CLI mode loses compaction (needs separate handling)
- **Effort**: Small
- **Risk**: Low

### Option 2: Add a flag to AgentParams to skip inline compaction
- **Pros**: Both modes work correctly, explicit control
- **Cons**: Adds a parameter
- **Effort**: Small
- **Risk**: Low

### Option 3: Remove spawned compaction from handlers.rs, keep inline only
- **Pros**: Simplest change, compaction already works inline
- **Cons**: Compaction holds the agent lock longer (blocks next message)
- **Effort**: Trivial
- **Risk**: Low

## Recommended Action

Option 2 — add `skip_compaction: bool` to `AgentParams`, set to `true` in server mode. Server handler spawns compaction outside the lock. CLI keeps inline compaction.

## Technical Details

- **Affected Files**: `crates/mika-agent/src/agent.rs`, `crates/mika-agent/src/server/handlers.rs`
- **Database Changes**: None

## Acceptance Criteria

- [ ] Compaction runs exactly once per message turn in both CLI and server modes
- [ ] Server mode compaction runs outside agent lock
- [ ] All tests pass

## Work Log

### 2026-02-24 - Identified during PR #5 review
**By:** performance-oracle, architecture-strategist, code-simplicity-reviewer

## Resources

- PR #5: Phase 2 Container HTTP Server

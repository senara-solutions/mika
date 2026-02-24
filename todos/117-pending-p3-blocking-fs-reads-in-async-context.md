---
status: pending
priority: p3
issue_id: "117"
tags: [code-review, performance, phase2]
dependencies: []
---

# Blocking std::fs::read_to_string in Async Context

## Problem Statement

Both `run_agent_inner` and `run_silent_inner` call `std::fs::read_to_string` for `soul.md` and `identity.toml` on the tokio runtime without `spawn_blocking`. For Phase 1 CLI with tiny local files this is fine. For Phase 2 HTTP server under load, blocking I/O on the runtime can cause latency for other tasks.

## Findings

- **Source:** architecture-strategist, pattern-recognition-specialist (5C)
- **Location:** `crates/mika-agent/src/agent.rs` lines 82, 319; `crates/mika-agent/src/prompt.rs` (`load_identity`)

## Proposed Solutions

### Option 1: Cache at startup (Recommended)
- **Pros**: Zero per-request I/O, simplest
- **Cons**: Doesn't pick up runtime changes to soul.md
- **Effort**: Small
- **Risk**: Low

### Option 2: Use tokio::fs::read_to_string
- **Pros**: Non-blocking, picks up changes
- **Cons**: More async propagation
- **Effort**: Small
- **Risk**: Low

## Recommended Action

_To be filled during triage — defer to Phase 2_

## Technical Details

- **Affected Files**: `crates/mika-agent/src/agent.rs`, `crates/mika-agent/src/prompt.rs`

## Acceptance Criteria

- [ ] No blocking file I/O on the tokio runtime during agent execution
- [ ] soul.md and identity.toml content still available to prompt builder

## Work Log

### 2026-02-24 - Identified in v4 Code Review
**By:** Multi-agent review (architecture-strategist, pattern-recognition-specialist)

## Resources

- Commit under review: 38a843b

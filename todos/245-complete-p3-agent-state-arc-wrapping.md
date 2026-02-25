---
status: complete
priority: p3
issue_id: "245"
tags: [code-review, performance]
dependencies: []
---

# AgentState cloned on every request (3 heap allocs)

## Problem Statement

`resolve_agent` returns `&AgentState` which is then `.clone()`d in handlers. The clone involves 3 heap allocations (PathBuf + 2 EmbeddingClient strings). Wrapping in `Arc<AgentState>` within the HashMap would make cloning a single atomic increment.

## Findings

- **Source:** Performance Oracle
- **File:** `crates/mika-agent/src/server/handlers.rs:59-60`, `crates/mika-agent/src/server/state.rs`

## Proposed Solutions

Change `agents: Arc<HashMap<String, AgentState>>` to `agents: Arc<HashMap<String, Arc<AgentState>>>`.

## Acceptance Criteria

- [x] AgentState wrapped in Arc in HashMap
- [x] Handlers clone Arc instead of full struct

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | Minor optimization for per-request path |
| 2026-02-25 | Implemented Arc wrapping | Changed HashMap value type to Arc<AgentState>, updated resolve_agent return type and all insertion sites |

---
title: "Observability polish: request_id linkage, session cleanup"
type: fix
status: completed
date: 2026-03-15
---

# Observability polish: request_id linkage, session cleanup

## Overview

Three observability gaps that reduce dashboard usefulness without breaking functionality:

1. HTTP server `request_id` is disconnected from agent `trace_id` — correlating server logs with agent turns requires timestamp matching
2. Team engine per-agent sessions are never materialized as DB rows — audit_events and tool context reference non-existent session IDs
3. Heartbeat/reflection/callback/skill-run sessions accumulate indefinitely with `ended_at = NULL` — no cleanup or lifecycle management

## Problem Statement

The observability system has two correlation axes: `trace_id` (per-request) and `session_id`/`agent_id` (system-level). All three gaps weaken these axes:

- **Gap 1** breaks the request axis: a gateway request cannot be traced into the agent turn it triggered
- **Gap 2** breaks the session axis: per-agent team sessions exist only as string IDs in audit_events/tool contexts, not as queryable session rows
- **Gap 3** creates unbounded growth: ~1095 heartbeat sessions/year plus callbacks and skill runs, never ended or pruned

## Proposed Solution

### 1. Server request_id → agent trace_id (`agent.rs`, `handlers.rs`)

Add `trace_id: Option<String>` to `AgentParams` (mirroring existing `SilentAgentParams` and `TeamAgentParams`). Pass `Some(request_id)` from the HTTP handler. Use `unwrap_or_else(generate_trace_id)` in `run_agent`.

**Files:**
- `crates/mika-agent/src/agent.rs` — add field to `AgentParams` (~line 638), use it at trace_id generation (~line 644)
- `crates/mika-agent/src/server/handlers.rs` — pass `trace_id: Some(request_id.clone())` when constructing `AgentParams` (~line 223-243)
- All call sites constructing `AgentParams` (CLI chat, CLI ask) — pass `trace_id: None` to preserve current behavior

**Pattern reference:** `SilentAgentParams` (line 1214-1217) and `TeamAgentParams` (line 1551-1554) already implement this exact pattern:
```rust
let trace_id = params.trace_id.clone().unwrap_or_else(mika_common::trace::generate_trace_id);
```

### 2. Team agent session creation (`teams/engine.rs`)

Create per-agent session rows before spawning agent tasks in `execute_tasks()`. Use `get_or_create_session`-style logic (existence check or INSERT OR IGNORE) to handle resumed team runs where per-agent sessions may already exist.

**Files:**
- `crates/mika-agent/src/teams/engine.rs` — create session in agent spawn loop (~line 912), before `run_team_agent` call
- `crates/mika-agent/src/db.rs` — may need a `create_session_if_not_exists` or similar helper with metadata support

**Design decisions:**
- Use delegated agent's own `agent_id` (from `resources.db.agent_id()`) — natural for queries filtering by agent
- Use `INSERT OR IGNORE` semantics for idempotency on resumed runs (same `team-{run_id}-{agent_name}` format is deterministic)
- Include metadata: `{"trigger": "team", "team_run_id": "<run_id>"}`
- End per-agent sessions after agent task completes (in the post-spawn result collection)

### 3. Silent session lifecycle management (`task_engine/dispatcher.rs`, `db.rs`)

Two parts: (a) call `end_session()` after all silent dispatcher variants complete, (b) add session pruning.

#### 3a. End sessions after silent runs

Call `end_session()` after each dispatcher variant completes:
- **Heartbeat** (~line 475-504) — after `run_silent_agent` returns
- **Reflection** (~line 602-639) — after `run_silent_agent` returns
- **Callback** (~line 297-329) — after `run_silent_agent` returns
- **Skill run** (~line 223-248) — after `run_silent_agent` returns

Each dispatcher already has the `session_id` in scope — it's a one-line addition per variant.

#### 3b. Prune old ended sessions

Add `prune_old_sessions(retention_days: i64)` to `db.rs`:
- Predicate: `ended_at IS NOT NULL AND ended_at < cutoff` (only ended sessions, never active ones)
- Scope: all sessions matching system/silent patterns (`heartbeat-%`, `callback-%`, `skill-%`, `reflection-%`)
- Retention: 7 days (matching `heartbeat_sends` pattern)
- Cascade: `ON DELETE CASCADE` on messages table handles message cleanup automatically
- Call site: `startup_recovery()` in task engine (runs in both server and CLI modes)
- Batch delete with LIMIT to avoid long-running transactions

**Out of scope:** Server handler (`handlers.rs`) session ending — same class of issue but different code path, tracked separately.

## Acceptance Criteria

- [x] `AgentParams` has `trace_id: Option<String>` field
- [x] HTTP handler passes `request_id` as `trace_id` to `run_agent`
- [x] CLI paths pass `trace_id: None` (preserves current behavior)
- [x] `run_agent` uses `unwrap_or_else(generate_trace_id)` pattern
- [x] Team engine creates per-agent session rows before spawning agents
- [x] Per-agent team sessions use delegated agent's `agent_id`
- [x] Resumed team runs don't fail on existing session rows (idempotent creation)
- [x] Per-agent team sessions are ended after agent task completes
- [x] All four silent dispatcher variants call `end_session()` after completion
- [x] `prune_old_sessions` method exists with 7-day retention
- [x] Pruning runs at task engine startup
- [x] Tests verify trace_id propagation from AgentParams
- [x] Tests verify team session row creation
- [x] Tests verify session ending after silent dispatch
- [x] Tests verify session pruning

## Technical Considerations

### Error handling
- Session creation failure in team engine: log warning and continue (matching existing pattern at line 371-376 for parent session)
- Pruning failure: log warning, don't block startup
- `end_session()` failure: log warning, don't fail the dispatch

### Race conditions
- Team session creation is sequential in the spawn loop — no race between creation and agent start
- Pruning at startup runs before the tick loop starts — no race with concurrent heartbeat dispatch
- Pruning predicate uses `ended_at < cutoff` — never touches sessions that haven't been properly ended

### Interaction with existing code
- `unified_timeline` VIEW already queries sessions — no changes needed there
- Dashboard session list will show properly ended sessions — improved UX
- No schema migration needed — all changes are code-level

## Sources & References

- **Learnings:** `docs/solutions/architecture-patterns/trace-id-structural-linkage-delegate-silent-callback.md` — trace_id propagation patterns
- **Learnings:** `docs/solutions/code-review-patterns/background-agent-mode-design-checklist.md` — bounded growth, pruning patterns
- **Learnings:** `docs/solutions/database-issues/trace-id-observability-gaps-callback-team-timeline.md` — trace_id gap patterns
- Related issue: #162

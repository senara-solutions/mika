---
title: "Observability polish: request_id → trace_id linkage and session lifecycle management"
category: architecture-patterns
date: 2026-03-15
severity: medium
tags: [observability, trace_id, session, pruning, team-engine, dispatcher]
related_issues: [162]
---

# Observability polish: request_id linkage and session lifecycle

## Problem

Three observability gaps reduced dashboard usefulness:

1. **HTTP server `request_id` disconnected from agent `trace_id`**: The gateway sends a `request_id` on each inbound request, but the agent loop generated a completely independent `trace_id`. Correlating server logs with agent turns required timestamp matching.

2. **Team per-agent sessions never materialized**: The team engine computed `team-{run_id}-{agent_name}` session IDs for per-agent runs but never created session rows. Audit events and tool contexts referenced non-existent sessions — a data quality issue (not a crash, since `audit_events.session_id` lacks a FK constraint).

3. **Silent sessions accumulated indefinitely**: Each heartbeat/reflection/callback/skill-run dispatch created a session with `ended_at = NULL` that was never ended or cleaned up. At ~3 heartbeats/day, this produces ~1095 orphaned session rows per year plus callbacks and skill runs.

## Root Cause

- `AgentParams` (unlike `SilentAgentParams` and `TeamAgentParams`) had no `trace_id: Option<String>` field, so `run_agent()` always called `generate_trace_id()` unconditionally.
- The team engine's `execute_tasks()` only created the orchestrator session (`team-{run_id}`), not the per-agent sessions.
- No dispatcher variant called `end_session()`, and no pruning logic existed for system sessions.

## Solution

### 1. request_id → trace_id propagation

Added `trace_id: Option<String>` to `AgentParams`, mirroring the existing pattern in `SilentAgentParams` and `TeamAgentParams`:

```rust
// agent.rs — AgentParams
pub trace_id: Option<String>,

// run_agent() — uses the same unwrap_or_else pattern
let trace_id = params.trace_id.clone()
    .unwrap_or_else(mika_common::trace::generate_trace_id);
```

HTTP handler passes `Some(req.request_id.clone())`; CLI passes `None`.

### 2. Team per-agent session creation

Added `create_session_if_not_exists()` (INSERT OR IGNORE) for idempotent session creation. Called in the agent spawn loop before `run_team_agent`, with `end_session()` after completion:

```rust
// teams/engine.rs — before spawning
resources.db.create_session_if_not_exists(
    &session_id, &agent_id, "team",
    Some(&metadata),  // {"trigger":"team","team_run_id":"..."}
).await;

// After agent completes
team_db.end_session(&end_session_id).await;
```

INSERT OR IGNORE handles resumed runs where session rows may already exist.

### 3. Silent session lifecycle

All four dispatcher variants now call `end_session()` after the silent agent run completes. Added `prune_old_sessions()` with 7-day retention, called during `startup_recovery()`:

```rust
// Prune predicate: only ended sessions with system prefixes
DELETE FROM sessions WHERE ended_at IS NOT NULL AND ended_at < ?1
  AND (id LIKE 'heartbeat-%' OR id LIKE 'callback-%'
       OR id LIKE 'skill-%' OR id LIKE 'reflection-%' OR id LIKE 'team-%')
```

Messages are cascade-deleted via FK `ON DELETE CASCADE`.

## Prevention

- **New async boundaries**: Any new code path that runs an agent loop MUST accept `trace_id: Option<String>` and use `unwrap_or_else(generate_trace_id)`. Grep for `trace_id: None` after changes — each site is a potential observability gap.
- **New session types**: Any new dispatcher variant or engine that creates sessions must also call `end_session()` and be included in the pruning predicate.
- **Pruning pattern**: Follow the established pattern from `prune_completed_tasks` — retention period constant, called at startup, log-and-continue on error.

## Key Files

- `crates/mika-agent/src/agent.rs` — `AgentParams.trace_id`, `run_agent()` trace_id generation
- `crates/mika-agent/src/server/handlers.rs` — passes `request_id` as `trace_id`
- `crates/mika-agent/src/teams/engine.rs` — per-agent session creation and ending
- `crates/mika-agent/src/task_engine/dispatcher.rs` — `end_session()` in all four variants
- `crates/mika-agent/src/task_engine/engine.rs` — session pruning in `startup_recovery()`
- `crates/mika-agent/src/db.rs` — `create_session_if_not_exists()`, `prune_old_sessions()`

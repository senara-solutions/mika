---
title: "delegate_task session and message persistence for observability"
category: architecture-patterns
date: 2026-03-24
severity: high
tags: [delegate-task, session-lifecycle, message-persistence, observability, unified-timeline]
modules: [tools/delegate_task, db, async_db, agent]
issue: "#253"
---

# delegate_task session and message persistence for observability

## Problem

When the orchestrator delegates work via `delegate_task`, the delegate agent's session contained zero messages in the database. The task prompt was injected into the system prompt (`TeamAgentParams.task_message`) and consumed by `run_team_agent()` as in-memory context — never persisted. The response flowed back as `ToolOutput::success()` to the orchestrator — also never persisted.

**Symptoms:**
- Dashboard showed empty sessions for delegate agents
- `unified_timeline` VIEW had no record of delegate interactions
- No audit trail of what delegates were asked or answered
- Only trace was in the orchestrator's tool call metadata (opaque)

**Root cause:** `LoopMode::Team` has `saves_to_db() == false` (line 133 of agent.rs), so `run_loop()` skips all message persistence. The team engine works around this by manually persisting messages after `run_team_agent()` returns (engine.rs lines 993, 1023), but `delegate_task` did not.

Additionally, `delegate_task` generated a `session_id` (UUID) but never created a session row in the DB, so even if it tried to save messages, the FK constraint would fail.

## Solution

Follow the team engine's established pattern for external message persistence around `run_team_agent()`:

### 1. Register delegate agent (FK compliance)

```rust
// Register delegate agent so sessions FK constraint is satisfied.
let identity = crate::prompt::load_identity(&agent_home);
if let Err(e) = async_db
    .register_agent(agent_name, &identity.name, agent_home.to_str().unwrap_or(""))
    .await
{
    tracing::warn!(agent = agent_name, error = %e, "failed to register delegate agent");
}
```

The `sessions` table has `agent_id REFERENCES agents(id)`. Without registration, session creation fails with an FK violation for agents not yet in the `agents` table.

### 2. Use `delegate-` session ID prefix

```rust
let session_id = format!("delegate-{}", uuid::Uuid::new_v4());
```

Enables `prune_old_sessions()` to clean up delegate sessions (7-day retention after `ended_at`).

### 3. Create session with parent linkage BEFORE `run_team_agent()`

```rust
let delegate_metadata = serde_json::json!({
    "trigger": "delegate",
    "orchestrator": current_agent_id,
    "work_item_id": work_item_id
}).to_string();
if let Err(e) = async_db
    .create_session_with_parent(&session_id, agent_name, "system",
        Some(&delegate_metadata), Some(ctx.session_id))
    .await
{
    tracing::warn!(session = %session_id, error = %e, "failed to create delegate session");
}
```

**Critical timing:** Session must be created BEFORE `run_team_agent()` so tool-level DB writes (audit events) during the agent run have a valid session FK via `ToolContext.session_id`.

### 4. Persist user message (task text)

```rust
if let Err(e) = async_db
    .save_message(&session_id, "user", task, Some(ctx.trace_id))
    .await
{
    tracing::warn!(session = %session_id, error = %e, "failed to persist delegate task message");
}
```

### 5. Persist result message after `run_team_agent()`

```rust
match &result {
    Ok(Some(text)) => {
        // Save assistant response
        if let Err(e) = async_db.save_message(&session_id, "assistant", text, Some(ctx.trace_id)).await {
            tracing::warn!(...);
        }
    }
    Ok(None) => {} // No text — skip empty assistant message
    Err(e) => {
        // Save error as system message
        let error_msg = format!("Delegation failed: {e}");
        if let Err(pe) = async_db.save_message(&session_id, "system", &error_msg, Some(ctx.trace_id)).await {
            tracing::warn!(...);
        }
    }
}
```

### 6. End session and shutdown

```rust
if let Err(e) = async_db.end_session(&session_id).await {
    tracing::warn!(...);
}
async_db.shutdown(); // Must be last DB operation
```

### 7. Add `delegate-` to `prune_old_sessions()`

```sql
DELETE FROM sessions WHERE ended_at IS NOT NULL AND ended_at < ?1
AND (id LIKE 'heartbeat-%' OR id LIKE 'callback-%' OR id LIKE 'skill-%'
     OR id LIKE 'reflection-%' OR id LIKE 'team-%' OR id LIKE 'delegate-%')
```

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Session created BEFORE agent run | Tools write audit events referencing `session_id` during `run_loop` |
| `channel_type = "system"` | Matches non-interactive convention used by all silent dispatchers |
| `parent_session_id = ctx.session_id` | Enables session hierarchy tracing in dashboard |
| `trace_id = ctx.trace_id` | Correlates delegate events with orchestrator's turn in `unified_timeline` |
| Persistence failures are warn-and-continue | Observability is best-effort; tool functionality is not degraded |
| `LoopMode::Team` unchanged | Changing `saves_to_db()` would affect all team agent runs — out of scope |

## Prevention

- **Pattern check for new dispatch sites:** Any code path that calls `run_team_agent()` or `run_silent_agent()` must create a session, persist messages, and end the session. Grep for `run_team_agent` to find all call sites.
- **Error handling rule:** Never use `let _ =` on DB insert results. Use `warn!` logging at minimum (per P2-443 findings in `docs/solutions/logic-errors/team-engine-code-review-findings-batch.md`).
- **Session pruning rule:** Any new session ID prefix must be added to `prune_old_sessions()` SQL in `db.rs`, or sessions accumulate indefinitely.

## Related

- `docs/solutions/architecture-patterns/observability-request-id-session-lifecycle.md` — Session lifecycle patterns for silent dispatchers
- `docs/solutions/architecture-patterns/trace-id-structural-linkage-delegate-silent-callback.md` — Trace ID propagation across delegate boundaries
- `docs/solutions/logic-errors/team-engine-code-review-findings-batch.md` — P2-443: never `let _ =` on DB mutations
- `docs/solutions/database-issues/team-task-child-wrong-agent-id.md` — Agent ID scoping for delegate DB operations
- `docs/solutions/integration-issues/multi-agent-telegram-delivery-and-reply-routing.md` — Delegate sender and chat_id pass-through

# Plan: bug(engine) — Deferred-dispatch resume strips webhook auth context (mika#1205)

**Issue:** [mika#1205](https://github.com/senara-solutions/mika/issues/1205)
**Type:** bug
**Component:** agent-core (task engine + dispatch guards)
**Priority:** p1-important

## Root Cause Analysis

### The three-event failure chain

When a `ready` label webhook arrives but the dispatch slot is busy (`global_dispatch_active`), the engine correctly defers the dispatch. The failure occurs in three steps:

1. **Authorized deferral.** Ready-label webhook for issue B arrives (authorized). `validate_dispatch_readiness` passes guard (0) — `is_unauthorized_webhook_dispatch("[GitHub] Issue labeled ready on ...")` returns `false`. But guard (4) fires — slot busy with issue A's groom. Deferred callback registered. Conversation turn ends.

2. **Cross-contamination.** Before the DeferredDispatch fires, a *different* webhook event arrives (issue comment, label change, or other fallthrough-domain event). This triggers a new conversation turn for mika-dev. The `originating_message` for this turn is the new (unauthorized) event. The LLM, seeing the pending supervisor task for B in conversation history, retries `run_claude_pilot`. Guard (0) fires: `is_unauthorized_webhook_dispatch("[GitHub] New comment on ...")` returns `true`. Tool returns `unauthorized_webhook_dispatch`.

3. **State poisoning.** The LLM, seeing the rejection, transitions B's supervisor task from `in_progress` → `blocked`. When the DeferredDispatch silent agent finally fires (after A completes), guard (1) at `executor.rs:892` rejects with `task_not_dispatchable` because the task is `blocked`. The dispatch is permanently wedged.

### Why the DeferredDispatch path alone is insufficient

The DeferredDispatch silent agent path at `agent.rs:3626` correctly sets `originating_message: None`, which bypasses guard (0). This is sound engineering. But it doesn't prevent the cross-contamination at step 2 — the guard operates on the **current turn's** `originating_message`, not the **task's** authorization history. A deferred task that was originally authorized through the positive-consent ready-label path can be blocked by an unrelated webhook that triggers a conversation turn between deferral and resume.

### Evidence (overnight 2026-05-18 → 2026-05-19)

- 22:51:02Z — Registration: `global_dispatch_active` + `deferred_dispatch_registered:true`
- 23:06:07Z — Cross-contamination: `unauthorized_webhook_dispatch` in a subsequent turn
- 23:08:18Z — Blocking task delivered (DeferredDispatch would fire here, but task already blocked)
- 23:11:04Z — Supervisor `in_progress → blocked`. No PR. No retry.

The 23:06 timestamp precedes the 23:08 delivery — the failure happens *before* the DeferredDispatch has a chance to fire.

## Fix Approach

**Option chosen:** Issue Option 1 — tag deferred dispatches at registration time with their authorization provenance. This is strictly more correct than Option 2 (which only covers the DeferredDispatch silent path, already working) because it covers ALL dispatch paths for the task, including conversation-turn retries.

### Design

Add a `dispatch_authorized` marker to the parent task's metadata when a deferred dispatch is registered. In `validate_dispatch_readiness`, check this marker before applying the unauthorized_webhook_dispatch guard.

**Invariant:** The marker can only be set if the dispatch previously PASSED guard (0). `register_deferred_callback` is only called from the `global_dispatch_active` rejection path (executor.rs:959), which is downstream of guard (0). If guard (0) had rejected, we'd never reach the registration. Therefore, every task with `dispatch_authorized: true` was authorized by a prior turn.

**Security property preserved:** The positive-consent contract (mika#841) is not weakened. The guard still fires for tasks that were NEVER authorized. The marker only bypasses the guard for tasks that provably passed it in a prior turn.

## Implementation

### Phase 1: Store authorization stamp at deferral time

**File:** `crates/mika-agent/src/skills/executor.rs`

In `register_deferred_callback` (line 1493), after successfully creating the deferred callback task (the `Ok(deferred_id)` branch at line 1565), write `dispatch_authorized: true` to the **parent** task's metadata via shallow merge:

```rust
// After successful deferred registration:
// Tag the parent task with authorization provenance (#1205).
// This marker is downstream of guard (0) in validate_dispatch_readiness —
// if the originating turn was unauthorized, we'd never reach this code.
let auth_meta = serde_json::json!({ "dispatch_authorized": true });
if let Err(e) = db.update_task_metadata_merge(task_id, &auth_meta.to_string()).await {
    warn!(
        task_id,
        error = %e,
        "failed to write dispatch_authorized marker — guard bypass will not apply"
    );
    // Fail-open: the DeferredDispatch path still works (originating_message: None).
}
```

### Phase 2: Add bypass in validate_dispatch_readiness

**File:** `crates/mika-agent/src/skills/executor.rs`

The unauthorized_webhook_dispatch guard at line 850 currently fires before the task is fetched (line 870). Since we need the task's metadata to check the marker, reorder:

1. Move the task fetch (lines 870-889) ABOVE the unauthorized_webhook_dispatch guard.
2. Add a metadata bypass to the guard:

```rust
async fn validate_dispatch_readiness(
    db: &AsyncDatabase,
    task_id: &str,
    github_token: Option<&str>,
    tool_input: Option<&serde_json::Value>,
    originating_message: Option<&str>,
) -> Result<String, String> {
    // Fetch task first — needed by both the auth-stamp bypass (#1205) and
    // all subsequent guards. Previously ran after guard (0) as an optimization
    // (avoid DB hit on pure-string rejection); moved up because guard (0) now
    // needs task metadata.
    let task = match db.get_task(task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => { /* existing not-found error */ },
        Err(e) => { /* existing DB error */ },
    };

    // Guard (0): unauthorized webhook dispatch (#933, #1205).
    if let Some(msg) = originating_message
        && crate::webhook_dispatch::is_unauthorized_webhook_dispatch(msg)
    {
        // Bypass: task was previously authorized via a ready-label webhook that
        // was deferred by global_dispatch_active (#1205). The marker is set by
        // register_deferred_callback, which is downstream of this guard — so the
        // marker can only exist if a prior turn provably passed guard (0).
        let is_previously_authorized = task.metadata
            .as_ref()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
            .and_then(|v| v.get("dispatch_authorized")?.as_bool())
            .unwrap_or(false);

        if !is_previously_authorized {
            let rejection = serde_json::json!({
                "error": "unauthorized_webhook_dispatch",
                // ... existing rejection body
            });
            record_dispatch_rejection(db, task_id, &rejection.to_string()).await;
            return Err(rejection.to_string());
        }
        // Previously authorized — fall through to remaining guards.
    }

    // Status check — now uses the already-fetched task.
    if !matches!(task.status.as_str(), "pending" | "in_progress") {
        // ... existing rejection
    }

    // ... remaining guards unchanged
}
```

### Phase 3: Clear marker after successful dispatch

**File:** `crates/mika-agent/src/skills/executor.rs`

In `execute_long_running` (line 1606), after the subprocess spawns successfully, clear the `dispatch_authorized` marker to prevent stale authorization from leaking into future dispatch cycles:

```rust
// After successful spawn — clear the authorization marker (#1205).
// The dispatch succeeded; stale markers could authorize future unauthorized retries.
let clear_meta = serde_json::json!({ "dispatch_authorized": null });
let _ = ctx.db.update_task_metadata_merge(task_id, &clear_meta.to_string()).await;
```

### Phase 4: Tests

**File:** `crates/mika-agent/src/skills/executor.rs` (in `#[cfg(test)] mod tests`)

Add three test cases:

1. **`test_dispatch_guard_bypasses_on_authorized_deferred`** — Create a task with `metadata: {"dispatch_authorized": true}`. Call `validate_dispatch_readiness` with an unauthorized originating_message (`"[GitHub] New comment on ..."`). Assert: returns `Ok` (guard bypassed).

2. **`test_dispatch_guard_rejects_unauthorized_without_marker`** — Create a task with no `dispatch_authorized` marker. Call `validate_dispatch_readiness` with an unauthorized originating_message. Assert: returns `Err` containing `"unauthorized_webhook_dispatch"`.

3. **`test_register_deferred_sets_authorized_marker`** — Create a parent task. Call `register_deferred_callback`. Assert: parent task's metadata contains `"dispatch_authorized": true`.

### Phase 5: Integration test (eval harness)

**File:** `crates/mika-agent/tests/eval/` (new scenario or extension)

Add a `MockLlmProvider` scenario that exercises the full chain:
1. First turn: `run_claude_pilot` → `global_dispatch_active` → deferred registered
2. Verify parent task metadata contains `dispatch_authorized: true`
3. Simulate a second turn with unauthorized webhook originating_message
4. Verify `run_claude_pilot` succeeds (guard bypassed via marker)

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/executor.rs` | Phase 1: write marker in `register_deferred_callback`. Phase 2: reorder guards + add bypass in `validate_dispatch_readiness`. Phase 3: clear marker in `execute_long_running`. Phase 4: unit tests. |

## Out of Scope

- **Prompt-level retry prevention:** Preventing the LLM from retrying deferred dispatches in conversation turns is a prompt discipline issue (mika#716). The engine-level fix here is the correct primary defense.
- **DeferredDispatch silent agent path:** Already works correctly (`originating_message: None`). No changes needed.
- **Relaxing the positive-consent contract:** mika#841 is load-bearing security. The bypass is scoped to tasks with provable prior authorization.
- **The Class D body callout drift (mika#1204):** Separate ticket.
- **LLM fabrication behavior (mika#716):** Separate ticket.

## Risks

1. **Stale marker after task metadata overwrites.** Mitigated: metadata updates use shallow merge, and Phase 3 clears the marker after successful dispatch. Even if the marker persists incorrectly, the security impact is bounded — it only bypasses guard (0) for a specific task that was provably authorized in a prior turn.

2. **Guard reordering moves DB fetch earlier.** Performance impact: one additional DB query on conversation turns where `originating_message` is unauthorized. This is rare (fallthrough-domain webhooks are the minority of mika-dev's traffic) and the DB hit is cheap (single-row PK lookup).

3. **Race between marker write and DeferredDispatch.** Both paths now work: the marker enables conversation-turn retries, and the DeferredDispatch path bypasses via `originating_message: None`. They compose correctly — whichever fires first succeeds.

## Acceptance Criteria

- AC1: A task deferred via `global_dispatch_active` from an authorized ready-label webhook can be re-dispatched in a subsequent turn triggered by an unauthorized webhook (the `dispatch_authorized` marker bypasses guard 0).
- AC2: A task that was NEVER authorized (no `dispatch_authorized` marker) is still rejected by guard (0) when the originating_message is unauthorized.
- AC3: The `dispatch_authorized` marker is cleared after successful dispatch to prevent stale authorization.
- AC4: Existing tests continue to pass. No behavioral change for tasks without the marker.

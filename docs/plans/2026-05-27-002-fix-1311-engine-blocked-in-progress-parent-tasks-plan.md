# Plan: fix(engine): blocked/in_progress parent tasks accumulate, preventing new dispatches

**Issue:** mika#1311
**Type:** bug fix
**Priority:** p1-important

## Problem Statement

Parent tasks (`groom mika#N`, ticket-titled manual tasks) on mika-dev accumulate in `blocked` or `in_progress` status indefinitely. When their child callback tasks deliver successfully, the parent never transitions to a terminal state. This wedges the dispatch slot — new `ready`-label webhooks find the slot apparently occupied and silently drop. Today (2026-05-27) required manual SQL cleanup of ~15 wedged parents.

## Root Cause Analysis

Three coverage gaps in the existing reaper/completer backstop family:

### Gap 1: `blocked` parents are invisible to both backstops

Both `reap_orphaned_parent_tasks` (#871) and `complete_parent_tasks_on_callback_success` (#1162) query only `parent.status = 'in_progress'`. Parents that the LLM set to `blocked` via `update_task_status` (e.g., during a milestone advance failure, or when the agent hit a wall and blocked the parent before the callback delivered) are never recovered. The evidence table shows 7 of ~15 stuck tasks were `blocked`.

The state machine allows `blocked → in_progress → completed/cancelled`, but no engine backstop transitions `blocked` parents to a terminal state when their children are all done.

### Gap 2: groom-class callbacks have no reaper/completer

Both backstops filter `COALESCE(child.dispatch_class, 'implement') = 'implement'`. This was intentional — groom dispatches don't produce `PR:` lines, so the `pr_url` predicate-based coupled pair can't determine success/failure.

But groom-class parents also wedge when their callback delivers and the silent turn fails to transition the parent. The evidence shows `groom mika#*` labels dominating the stuck-task list.

### Gap 3: no grace-window-free backstop for definitively-terminal children

The reaper/completer both use `REAPER_GRACE_SECONDS` (600s). But when ALL children of a parent are in terminal states (`delivered`/`failed`/`expired`/`cancelled`) and the parent is non-terminal, the parent is definitively orphaned — the grace window is unnecessary overhead that delays slot release.

## Solution Design

Generalize the reaper/completer family with a new third backstop — the **parent task reconciler** — that handles the broader class of orphaned parents regardless of dispatch class or `pr_url` presence.

### Approach: Generalized parent reconciler (issue option 1, expanded)

Add `reconcile_orphaned_parent_tasks()` as the third periodic scan sibling, running at the same `DB_SCAN_INTERVAL_TICKS` cadence.

**Selection query:** Find parent tasks where:
- `parent.status IN ('in_progress', 'blocked')` — covers both stuck states
- `parent.source = 'self_dev'` — only self-dev dispatch parents
- `parent.trigger_type = 'manual'` — matches existing reaper/completer scope
- ALL children are in terminal states: `NOT EXISTS (SELECT 1 FROM tasks child WHERE child.parent_task_id = parent.id AND child.status IN ('pending', 'in_progress'))`
- At least one child exists (parent without children is a different class of orphan)
- The most recent child's `updated_at` is older than a reconciler grace period

**Outcome determination:** Use a two-tier decision tree based on available evidence:
1. If `parent.metadata.claude_pilot.pr_url IS NOT NULL AND != ''` → transition to `completed` (success evidence exists)
2. Otherwise → transition to `cancelled` with reason `parent_reconciled_all_children_terminal`

Using `cancelled` (not `failed`) for the fallback because:
- The parent's children may have succeeded (groom-class delivering a plan) — marking `failed` is misleading
- `cancelled` accurately represents "this task was cleaned up by the engine, not completed by the operator"
- It matches the manual SQL intervention pattern from the incident (`SET status = 'cancelled'`)

**Audit trail:** `tool_name = 'task_engine_reconciler'`, reason distinguishes the two paths: `parent_reconciled_with_pr_url` and `parent_reconciled_all_children_terminal`.

### Interaction with existing backstops

The three backstops form a priority chain at query time — each one's SQL predicates exclude the others:

| Backstop | Parent status | pr_url | dispatch_class | Outcome |
|----------|--------------|--------|----------------|---------|
| Reaper (#871) | `in_progress` | `IS NULL` | `implement` | `failed` |
| Completer (#1162) | `in_progress` | `IS NOT NULL` | `implement` | `completed` |
| **Reconciler (#1311)** | `in_progress` OR `blocked` | any | any | `completed` or `cancelled` |

The reconciler is the catch-all that handles rows the reaper/completer miss:
- `blocked` parents (any dispatch class)
- `in_progress` groom-class parents
- Any future dispatch class that doesn't fit the reaper/completer's implement-only scope

**No interference with existing backstops:** The reconciler's "all children terminal" predicate means it cannot fire on parents that the reaper/completer would handle, because those queries also require `child.status = 'delivered'` (active sibling guard) — if the child is `delivered` and past grace, the reaper/completer fires first on `in_progress`/`implement` rows. The reconciler catches what falls through.

## Implementation Steps

### Step 1: Add `find_reconcilable_parent_tasks` DB query (`db.rs`)

New query method on `Database` that finds parents where:
- `parent.status IN ('in_progress', 'blocked')`
- `parent.source = 'self_dev'`
- `parent.trigger_type = 'manual'`
- `NOT EXISTS` active child (status `IN ('pending', 'in_progress')`)
- `EXISTS` at least one child (not childless orphans)
- Most recent child `updated_at` older than grace period (use a shorter grace: 300s — half the reaper's 600s, because the "all children terminal" predicate makes the parent definitively orphaned)

New struct `ReconcilableParentTask`:
```rust
pub struct ReconcilableParentTask {
    pub id: String,
    pub agent_id: String,
    pub status: String,         // "in_progress" or "blocked"
    pub created_at: String,
    pub pr_url: Option<String>, // from metadata
    pub latest_child_id: String,
}
```

**Coupled-pair comment:** Document that this query is the catch-all sibling to `find_orphaned_parent_tasks` and `find_completable_parent_tasks_on_pr_url`. Filter changes to the shared predicates (agent_id, source, trigger_type) must be applied to all three queries.

### Step 2: Add `reconcile_orphaned_parent_tasks` engine method (`engine.rs`)

New async method on `TaskEngine`, structured identically to the reaper/completer:

```rust
async fn reconcile_orphaned_parent_tasks(&self) {
    let candidates = self.db
        .find_reconcilable_parent_tasks(RECONCILER_GRACE_SECONDS)
        .await;
    
    for parent in candidates {
        let trace_id = generate_trace_id();
        let system_session = format!("system-{}", parent.agent_id);
        
        // Decision tree: pr_url present → completed, else → cancelled
        let (outcome_status, reason) = if let Some(ref pr_url) = parent.pr_url {
            ("completed", format!("parent_reconciled_with_pr_url (pr_url: {pr_url})"))
        } else {
            ("cancelled", "parent_reconciled_all_children_terminal".to_string())
        };
        
        // Use appropriate guarded update
        let transitioned = match outcome_status {
            "completed" => self.db.update_task_completed(&parent.id, Some(&reason)).await,
            _ => self.db.update_task_cancelled(&parent.id, Some(&reason)).await,
        };
        
        // Audit event + logging (mirror reaper pattern)
    }
}
```

Constants:
- `RECONCILER_GRACE_SECONDS: i64 = 300` — shorter than `REAPER_GRACE_SECONDS` (600s) because the all-children-terminal predicate already provides strong evidence of orphanhood

### Step 3: Wire into tick loop (`engine.rs`)

Add the reconciler call after the existing reaper and completer in the `tick()` method:

```rust
if self.tick_count.is_multiple_of(DB_SCAN_INTERVAL_TICKS) {
    // ... existing calls ...
    self.reap_orphaned_parent_tasks().await;
    self.complete_parent_tasks_on_callback_success().await;
    // New: catch-all reconciler for blocked parents and groom-class orphans
    self.reconcile_orphaned_parent_tasks().await;
}
```

Position after the existing backstops ensures the narrower, more-precise reaper/completer get first shot. The reconciler handles what falls through.

### Step 4: Add `update_task_cancelled` DB method (if not present)

Check if a guarded `update_task_cancelled` exists (analogous to `update_task_failed` and `update_task_completed`). If not, add one:

```rust
pub fn update_task_cancelled(&self, id: &str, reason: Option<&str>) -> Result<bool> {
    let result = reason.as_deref().unwrap_or("");
    let rows = self.conn.execute(
        "UPDATE tasks SET status = 'cancelled', result = ?1,
         updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?2 AND status IN ('pending', 'in_progress', 'blocked')",
        params![result, id],
    )?;
    Ok(rows > 0)
}
```

The `WHERE status IN (...)` guard prevents overwriting concurrent terminal transitions — same pattern as `update_task_failed` and `update_task_completed`.

### Step 5: Add inline reconciliation at callback delivery (`dispatcher.rs`)

Add `try_reconcile_parent_on_callback_delivery` as the fourth helper in the `if is_callback { ... }` block in `dispatch_resume_agent`:

```rust
if is_callback {
    try_extract_callback_metadata(&self.db, task).await;
    try_promote_parent_on_retry_success(&self.db, task).await;
    try_complete_parent_on_callback_success(&self.db, task).await;
    // #1311: catch-all reconciler for blocked parents and groom-class
    try_reconcile_parent_on_callback_delivery(&self.db, task).await;
}
```

This inline path handles the fast case: when a callback delivers and the parent is `blocked` (or `in_progress` groom-class), transition the parent immediately without waiting for the periodic scan.

Logic:
1. Get parent task (if any)
2. If parent is in terminal state → skip
3. If parent is `in_progress` and implement-class → skip (reaper/completer covers this)
4. Check if parent has any other active children → skip if yes
5. Apply same pr_url decision tree → `completed` or `cancelled`

### Step 6: Tests

Add tests mirroring the reaper/completer test structure:

**DB tests (`db.rs`):**
- `test_find_reconcilable_parent_tasks_blocked_parent` — blocked parent with delivered child matches
- `test_find_reconcilable_parent_tasks_in_progress_groom_class` — in_progress parent with groom-class delivered child matches
- `test_find_reconcilable_parent_tasks_skips_active_children` — parent with pending child does not match
- `test_find_reconcilable_parent_tasks_skips_childless` — parent with no children does not match
- `test_find_reconcilable_parent_tasks_skips_in_progress_implement` — in_progress parent with implement-class child does not match (deferred to reaper/completer)
- `test_find_reconcilable_parent_tasks_grace_period` — child updated within grace period does not match

**Engine tests (`engine.rs`):**
- `test_reconcile_orphaned_parent_tasks_blocked_to_cancelled` — blocked parent transitions to cancelled
- `test_reconcile_orphaned_parent_tasks_blocked_with_pr_url_to_completed` — blocked parent with pr_url transitions to completed
- `test_reconcile_orphaned_parent_tasks_groom_class_to_cancelled` — groom-class parent transitions to cancelled

**Dispatcher tests (`dispatcher.rs`):**
- `test_try_reconcile_parent_on_callback_delivery_blocked_parent` — inline path transitions blocked parent
- `test_try_reconcile_parent_on_callback_delivery_groom_class_parent` — inline path transitions groom parent
- `test_try_reconcile_parent_on_callback_delivery_implement_skips` — inline path defers implement-class to existing backstops

## Scoping Notes

### In scope
- Generalized reconciler for blocked and groom-class parent tasks
- Both inline (at callback delivery) and periodic (tick loop) paths
- `update_task_cancelled` guarded helper if not present
- Audit trail and structured logging

### Out of scope
- Operator-facing CLI command (`mika tasks reap-blocked --agent <name>`) — the structural fix eliminates the need for manual SQL
- Promotion-on-redeliver (issue option 3) — adds complexity; the reconciler catches orphans within ~5 min which is acceptable
- Changing the existing reaper/completer queries — they remain correct and narrowly-scoped for their specific cases
- Root-cause investigation of WHY parents get stuck in `blocked` — the reconciler is the safety net regardless of root cause

## Risk Assessment

**Low risk.** The reconciler follows the exact same pattern as the reaper (#871) and completer (#1162) — same cadence, same guarded updates, same audit trail. The new query targets a mutually-exclusive population (parents the existing backstops miss).

**Regression risk:** Near zero. The `WHERE status IN (...)` guard on the DB update prevents overwriting concurrent transitions. The new query cannot match rows the existing backstops match (different status/class predicates).

**Performance:** One additional SQL query per 60-tick cycle per agent. Same ORDER/GROUP as existing queries. Negligible.

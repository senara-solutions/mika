# Plan: bug(engine) — Deferred-dispatch retry strips webhook auth context (mika#1205)

**Issue:** [mika#1205](https://github.com/senara-solutions/coordinated/issues/1205)
**Type:** bug
**Component:** agent-core (task engine + dispatch guards)
**Priority:** p1-important
**Base SHA for pins:** `2f60757d` (the plan's parent commit on this branch)

## Root Cause Analysis

### The three-event failure chain (refined)

When a `ready`-label webhook arrives but the per-class dispatch slot is busy (`global_dispatch_active`), the engine correctly defers the dispatch by creating a child task with `label = DEFERRED_DISPATCH_LABEL`. The failure occurs in three steps:

1. **Authorized deferral.** Ready-label webhook for issue B arrives (authorized). `validate_dispatch_readiness` passes guard (0) — `is_unauthorized_webhook_dispatch("[GitHub] Issue labeled ready on ...")` returns `false`. Guard (1) `task_active_dispatch` passes (no children yet). Guard (2) `global_dispatch_active` fires — slot busy with A's groom. `register_deferred_callback` creates the deferred-callback child row. Conversation turn ends with a `global_dispatch_active` rejection that includes `deferred_dispatch_registered: true`.

2. **Cross-contamination.** Before the DeferredDispatch silent agent fires, a *different* webhook event arrives (issue comment, label change, or other fallthrough-domain event). This triggers a new conversation turn for mika-dev. The `originating_message` for this turn is the new (unauthorized) event. The LLM, seeing the pending supervisor task for B in conversation history, retries `run_claude_pilot`. Guard (0) fires: `is_unauthorized_webhook_dispatch("[GitHub] New comment on ...")` returns `true`. Tool returns `unauthorized_webhook_dispatch` JSON.

3. **State poisoning.** The LLM, seeing the rejection it does not know how to interpret, transitions B's supervisor task from `in_progress → blocked` (LLM fabrication tracked separately at mika#716). When the DeferredDispatch silent agent finally fires (after A completes), guard (1) at `validate_dispatch_readiness` rejects the dispatch with `task_not_dispatchable` because the task is `blocked`. The dispatch is permanently wedged.

### Why the DeferredDispatch silent-agent path is not the broken surface

`crates/mika-agent/src/agent.rs:3626` constructs `LongRunningContext { ..., originating_message: None }` for `SilentTrigger::DeferredDispatch`. Guard (0) at `executor.rs:843` only fires `if let Some(msg) = originating_message`, so `None` bypasses guard (0). The body's "Option 2" (treat deferred-resume as internal-engine event) **was already shipped** via mika#920 + mika#1058 — the silent-agent path works correctly.

The actual broken surface is the **LLM-conversation-turn retry** that happens *between* deferral and DeferredDispatch resume. This is what step (2) above describes. The author of the issue body misidentified the broken surface (they thought it was the deferred resume) but the failure mode they evidenced is the cross-contamination path.

### Why a guard-0 bypass alone does not "let dispatch proceed"

Tracing the guard ordering at `executor.rs:840-1010`:

| # | Guard | Where | What it checks |
|---|---|---|---|
| 0 | `unauthorized_webhook_dispatch` | 843 | originating_message text prefix |
| 1 | `task_not_dispatchable` | 894 | task.status NOT IN ('pending','in_progress') |
| 2 | `task_active_dispatch` | 910 | any callback child with status pending/in_progress |
| 3 | `global_dispatch_active` | 954 | another task in same dispatch_class has active callback |

If guard (0) is bypassed for a task that has a pending deferred-callback child, guard (2) `task_active_dispatch` fires next and rejects with a *different* error (`task_active_dispatch`). The LLM's retry would still be rejected — just with a different (clearer) message. **Bypassing guard (0) alone does not enable a duplicate dispatch — nor should it, because a duplicate dispatch is incorrect when one is already queued.**

The correct semantic for the LLM's retry on a deferred-pending task is **idempotent acknowledgement**: "Your prior dispatch is queued and will fire automatically; do nothing." That semantic already exists at `executor.rs:316` for the callback-turn entry path (after `register_deferred_callback` succeeds). The fix is to surface the same idempotent semantic on the long-running-context path *before* any guard rejection that might confuse the LLM.

## Fix Approach

**Adopt the existence-based authorization signal (mika-arch first-pass F3):** the presence of a pending `deferred_dispatch` child task on the parent is itself proof that a prior turn passed guard (0) (since `register_deferred_callback` is downstream of guard 0). When `execute_long_running` is invoked on a task that has such a child, short-circuit with the same idempotent `status: "deferred"` success already returned by `executor.rs:316` — do not enter `validate_dispatch_readiness` at all.

**Why this over the issue body's "Option 1" metadata-marker design:**
- No new DB infrastructure required (the marker approach calls `update_task_metadata_merge` which does not exist; only full-replace `update_task_metadata` scoped to `trigger_type='manual'` exists).
- No marker lifecycle to manage (Phase 1 write + Phase 3 clear of the original plan are both eliminated).
- The authorization signal is **structurally coupled** to the condition it represents — the child row exists if and only if the prior dispatch was authorized AND queued.
- **Failure mode is fail-closed** (correct for a security gate): if the deferred-callback child is cleaned up before the LLM retries, the retry hits guard (0) normally; no stale authorization can leak.

**Why this over the body's "Option 2" (treat resume as internal event):** Option 2 has already been shipped (silent-agent path uses `originating_message: None`). The bug is on a different surface (LLM-conversation-turn retry).

**Why this over the body's "Option 3" (reject at registration time):** Option 3 would regress the dispatch-queue feature shipped in mika#1011 — the entire point of deferred dispatches is to absorb queue pressure without forcing the operator to retry.

### Security property preserved

The positive-consent contract (mika#841 / mika#933) is not weakened. Guard (0) still fires for all tasks that never went through an authorized turn. The intercept only short-circuits when a pending deferred-dispatch child exists, which can only exist if a prior turn passed guard (0) (because `register_deferred_callback` is downstream of guard 0 at both call sites: `executor.rs:311` after `check_lineage_cycle`, and `executor.rs:960` inside `validate_dispatch_readiness` after guard 0 passes).

**Per-agent scoping:** the intercept restricts the existence check to children with `child.agent_id == ctx.db.agent_id()`. This prevents cross-agent authorization leakage in team-task trees where children of different agents may coexist (per `db::get_child_tasks` doc comment: "No agent_id filter — team task trees have children with different agent_ids"). Each agent's authorization stays its own.

## Phase 0 — Pinned slices (base SHA `2f60757d`)

All Rust paths are inside the worktree at `crates/mika-agent/src/`.

### Pin 0.1 — `executor.rs:840-895` (`validate_dispatch_readiness` signature + guard 0)

```rust
async fn validate_dispatch_readiness(
    db: &AsyncDatabase,
    task_id: &str,
    github_token: Option<&str>,
    tool_input: Option<&serde_json::Value>,
    originating_message: Option<&str>,
) -> Result<String, String> {
    // #933 — Tool-boundary gate for unauthorized webhook dispatch. Cheapest check
    // (pure string-prefix match, no DB), runs first. Rejects `run_claude_pilot`
    // when the originating user message is in the Webhook Fallthrough domain.
    if let Some(msg) = originating_message
        && crate::webhook_dispatch::is_unauthorized_webhook_dispatch(msg)
    {
        let rejection = serde_json::json!({
            "error": "unauthorized_webhook_dispatch",
            "task_id": task_id,
            "reason": "This turn was initiated by a [GitHub] webhook event in the \
                       Webhook Fallthrough domain (issue events, comments, or \
                       unknown event types). Only `[GitHub] Issue labeled ready on` \
                       webhooks (authorized dispatch) and PR / Check-suite events \
                       handled by self-dev-webhook-qa / self-dev-webhook-ci skills \
                       may dispatch claude-pilot. All other webhook events must use \
                       Webhook Fallthrough: acknowledge without dispatching \
                       (mika#841 positive-consent contract, mika#933)."
        });
        record_dispatch_rejection(db, task_id, &rejection.to_string()).await;
        return Err(rejection.to_string());
    }

    // Re-fetch the task to get the full struct (validate_task confirmed existence)
    let task = match db.get_task(task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => { /* task_not_found */ }
        Err(e) => { /* dispatch_check_failed */ }
    };
```

### Pin 0.2 — `executor.rs:909-948` (guard 2: `task_active_dispatch`)

```rust
    // Check for active callback children (double-dispatch prevention)
    match db.get_child_tasks(task_id).await {
        Ok(children) => {
            let active_callback = children.iter().find(|c| {
                c.trigger_type == "callback"
                    && matches!(c.status.as_str(), "pending" | "in_progress")
            });
            if let Some(child) = active_callback {
                let pr_url = extract_pr_url(&task.metadata);
                let rejection = serde_json::json!({
                    "error": "task_active_dispatch",
                    "task_id": task_id,
                    "current_status": task.status,
                    "active_child_id": child.id,
                    "active_child_status": child.status,
                    "pr_url": pr_url,
                    "reason": format!(
                        "Task already has an active dispatch (callback task '{}' \
                         in '{}' status). Wait for it to complete or cancel it before \
                         dispatching again.",
                        child.id, child.status
                    )
                });
                record_dispatch_rejection(db, task_id, &rejection.to_string()).await;
                return Err(rejection.to_string());
            }
        }
        ...
    }
```

This is the guard that would fire *next* if guard (0) were bypassed without the existence-based short-circuit — confirming why a guard-0 bypass alone is insufficient.

### Pin 0.3 — `executor.rs:949-980` (guard 3: `global_dispatch_active` + downstream call to `register_deferred_callback`)

```rust
    let dispatch_class = tool_input.and_then(extract_skill_from_input);
    let class = derive_dispatch_class(dispatch_class);
    match db.has_active_callback_tasks_excluding(task_id, class).await {
        Ok(Some((blocking_parent_id, blocking_callback_id))) => {
            // mika#1011 — Register a deferred-dispatch callback so the engine
            // auto-retries when the blocking dispatch completes. The LLM still
            // sees the rejection (γ composition) and may call send_message;
            // both paths are independent and validate_dispatch_readiness()
            // arbitrates any race on the next dispatch attempt.
            let deferred_registered = if let Some(input) = tool_input {
                register_deferred_callback(db, task_id, input).await
            } else {
                false
            };
            ...
```

This is one of the two call sites for `register_deferred_callback`. **Confirms invariant:** this call site is reached only after guards 0, 1, 2 have all passed.

### Pin 0.4 — `executor.rs:300-326` (the existing idempotent "deferred" success on the callback-turn entry path — the pattern this fix extends)

```rust
        if let Some(task_id) = callback_task_id
            && let Some(db) = callback_db
        {
            match check_lineage_cycle(db, task_id, &input).await {
                Ok(()) => {
                    if register_deferred_callback(db, task_id, &input).await {
                        info!(
                            tool = %skill_tool.definition.name,
                            task_id,
                            "callback_deferred_dispatch_registered"
                        );
                        return ToolOutput::success(
                            serde_json::json!({
                                "status": "deferred",
                                "message": "Long-running dispatch registered as deferred callback. \
                                            It will fire automatically when the current dispatch \
                                            slot is free. Do not retry.",
                                "deferred": true
                            })
                            .to_string(),
                        );
                    }
```

This is the second call site for `register_deferred_callback` and the existing pattern for idempotent "deferred" success. The fix mirrors this response shape on the long-running-context path.

### Pin 0.5 — `executor.rs:1487-1565` (`register_deferred_callback`)

```rust
/// Register a deferred-dispatch callback when `global_dispatch_active` fires (mika#1011).
///
/// Creates a `pending` callback task with `label = "long_running:run_claude_pilot:deferred"`
/// linked to the requesting parent task. When the blocking dispatch completes, the
/// dispatcher promotes this to `in_progress` and fires a `SilentTrigger::DeferredDispatch`
/// turn. Returns `true` if registered, `false` if cap exceeded or DB error (fail-open).
async fn register_deferred_callback(
    db: &AsyncDatabase,
    task_id: &str,
    input: &serde_json::Value,
) -> bool {
    // ... cap check, sentinel injection into original_call, create_task with
    //     label = DEFERRED_DISPATCH_LABEL and agent_id = db.agent_id() ...
}
```

The created child has `agent_id = db.agent_id()` (line 1539). The intercept relies on this — see Pin 0.7.

### Pin 0.6 — `executor.rs:1606-1660` (`execute_long_running` entry — where the intercept is inserted)

```rust
async fn execute_long_running(
    skill_tool: &ResolvedSkillTool,
    command: &str,
    input: serde_json::Value,
    estimated_duration_secs: Option<u64>,
    ctx: &LongRunningContext,
    github_token: Option<&str>,
) -> ToolOutput {
    let task_id = input.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(err) = crate::tools::validate_task(&ctx.db, task_id).await {
        return ToolOutput::error(err);
    }

    // <-- INTERCEPT INSERTED HERE (Phase 1) -->

    let wi_status = match validate_dispatch_readiness(
        &ctx.db,
        task_id,
        github_token,
        Some(&input),
        ctx.originating_message.as_deref(),
    )
    .await
    {
        Ok(status) => status,
        Err(err) => return ToolOutput::error(err),
    };
```

### Pin 0.7 — `db.rs:5796-5810` (`get_child_tasks`)

```rust
    /// Get all child tasks for a given parent task.
    /// No agent_id filter — team task trees have children with different agent_ids.
    pub fn get_child_tasks(&self, parent_task_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE parent_task_id = ?1
             ORDER BY created_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![parent_task_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }
```

`AsyncDatabase::get_child_tasks` at `async_db.rs:567` wraps this. **Critical: no agent_id filter at the DB level.** The intercept must enforce per-agent scoping in Rust to prevent cross-agent authorization leakage.

### Pin 0.8 — `agent.rs:3104` (the deferred-dispatch label constant)

```rust
pub const DEFERRED_DISPATCH_LABEL: &str = "long_running:run_claude_pilot:deferred";
```

### Pin 0.9 — `agent.rs:3620-3637` (silent-agent DeferredDispatch arm — confirms why this path is NOT broken)

```rust
    // Construct LongRunningContext for DeferredDispatch triggers only (mika#1058).
    // DeferredDispatch turns MUST be able to call run_claude_pilot — that's their
    // sole purpose. All other silent triggers (Heartbeat, Callback, etc.) keep None.
    // originating_message is None: deferred-dispatch retries are engine-initiated
    // and have no fresh [GitHub]-prefixed user turn (mika#933).
    let long_running_ctx = if matches!(&params.trigger, SilentTrigger::DeferredDispatch { .. }) {
        Some(executor::LongRunningContext {
            db: db.clone(),
            agent_name: db.agent_id().to_string(),
            session_id: params.session_id.to_string(),
            trace_id: trace_id.clone(),
            dispatch_count: AtomicU32::new(0),
            originating_message: None,
        })
    } else {
        None
    };
```

`originating_message: None` is exactly the body's "Option 2" — already shipped via mika#1058. Pin proves this path is sound and does not need modification.

## Implementation

### Phase 1 — Insert the idempotent-deferred intercept in `execute_long_running`

**File:** `crates/mika-agent/src/skills/executor.rs`
**Insertion point:** between `validate_task` and `validate_dispatch_readiness` at line 1625 (see Pin 0.6).

```rust
    // mika#1205: Idempotent ack when a deferred-dispatch child is already pending
    // for this task on this agent.
    //
    // Background: when an LLM-conversation turn retries `run_claude_pilot` on a task
    // that has a pending deferred-callback child (because a prior turn was deferred
    // via global_dispatch_active), the existing guard (0) at validate_dispatch_readiness
    // returns `unauthorized_webhook_dispatch` if the current turn's originating_message
    // is in the Webhook Fallthrough domain. The LLM does not know how to interpret that
    // rejection on a task it just authorized; it can hallucinate a supervisor → blocked
    // transition (separately tracked at mika#716). Even if guard (0) is bypassed, the
    // next guard (`task_active_dispatch`) would reject the duplicate dispatch anyway.
    //
    // Correct semantic: return the same idempotent "deferred" success that
    // executor.rs:316 returns when register_deferred_callback succeeds on a callback
    // turn. The prior dispatch is queued and will fire automatically via
    // SilentTrigger::DeferredDispatch (agent.rs:3626). Tell the LLM not to retry.
    //
    // Security: the deferred-callback child can only exist if a prior turn passed
    // guard (0) — register_deferred_callback is downstream of guard (0) at both
    // call sites (executor.rs:311 and executor.rs:960). Per-agent filter prevents
    // cross-agent authorization leakage in team-task trees (db::get_child_tasks
    // has no agent_id filter; see Pin 0.7).
    match ctx.db.get_child_tasks(task_id).await {
        Ok(children) => {
            let self_agent = ctx.db.agent_id();
            let pending_deferred = children.iter().find(|c| {
                c.label == crate::agent::DEFERRED_DISPATCH_LABEL
                    && c.agent_id == self_agent
                    && matches!(c.status.as_str(), "pending" | "in_progress")
            });
            if let Some(child) = pending_deferred {
                info!(
                    task_id,
                    deferred_callback_id = %child.id,
                    "deferred_dispatch_idempotent_ack — prior dispatch already queued"
                );
                return ToolOutput::success(
                    serde_json::json!({
                        "status": "deferred",
                        "already_deferred": true,
                        "deferred_callback_id": child.id,
                        "deferred_callback_status": child.status,
                        "message": "Your prior dispatch for this task is queued as a \
                                    deferred callback and will fire automatically when \
                                    the dispatch slot is free. Do not retry; do not \
                                    transition the supervisor task. (mika#1205)"
                    })
                    .to_string(),
                );
            }
        }
        Err(e) => {
            // Fail-closed on DB error: skip the intercept and let the existing
            // guards apply. Worst case is reverting to current behavior (the bug
            // we're fixing), not a security regression — guard (0) still rejects
            // unauthorized retries.
            warn!(
                task_id,
                error = %e,
                "deferred_dispatch_intercept_check_failed — falling through to validate_dispatch_readiness"
            );
        }
    }
```

### Phase 2 — Doc comment on `register_deferred_callback` for the security invariant (NF2)

**File:** `crates/mika-agent/src/skills/executor.rs` (function at line 1493 — see Pin 0.5).

Update the doc comment to add a Precondition section:

```rust
/// Register a deferred-dispatch callback when `global_dispatch_active` fires (mika#1011).
///
/// Creates a `pending` callback task with `label = "long_running:run_claude_pilot:deferred"`
/// linked to the requesting parent task. When the blocking dispatch completes, the
/// dispatcher promotes this to `in_progress` and fires a `SilentTrigger::DeferredDispatch`
/// turn. Returns `true` if registered, `false` if cap exceeded or DB error (fail-open).
///
/// # Precondition (security-load-bearing, mika#1205)
///
/// All callers MUST be downstream of the `unauthorized_webhook_dispatch` guard
/// (executor.rs guard 0). The deferred-callback child row created by this function
/// is later read by `execute_long_running` as proof that a prior turn was authorized
/// — that read uses the row's existence to short-circuit duplicate-retry rejection
/// with an idempotent "deferred" success.
///
/// Current call sites (both verified downstream of guard 0):
/// - `executor.rs:311` — callback-turn rejection path, downstream of `check_lineage_cycle`.
/// - `executor.rs:960` — `global_dispatch_active` path inside `validate_dispatch_readiness`,
///   downstream of guards 0, 1, 2.
///
/// Adding a new call site that does NOT pass guard 0 first would let unauthorized
/// dispatches forge authorization. If you add one, document the guard-0 equivalence
/// at the call site and update this comment.
```

### Phase 3 — Unit tests

**File:** `crates/mika-agent/src/skills/executor.rs` (in `#[cfg(test)] mod tests` — the existing test module).

Add four test cases adjacent to the existing `test_register_deferred_callback_injects_sentinel` test at line ~4596 (which establishes the same helper-utility surface):

1. **`test_execute_long_running_idempotent_ack_on_pending_deferred`** — Create a parent task. Call `register_deferred_callback` to seed a pending child. Invoke `execute_long_running` (via its test harness, or extract the intercept into a small helper if testing through the full call is too heavy) with an unauthorized originating_message in `LongRunningContext`. Assert: returns `ToolOutput::success` with `status: "deferred"` and `already_deferred: true`. Assert: `validate_dispatch_readiness` is NOT called (no `unauthorized_webhook_dispatch` rejection observable).

2. **`test_execute_long_running_no_intercept_when_no_deferred_child`** — Create a parent task with no children. Invoke `execute_long_running` with an unauthorized originating_message. Assert: falls through to `validate_dispatch_readiness`, which returns `unauthorized_webhook_dispatch` (existing behavior preserved).

3. **`test_execute_long_running_intercept_scopes_per_agent`** — Create a parent task. Seed a deferred-callback child with a *different* `agent_id` than the caller's. Invoke `execute_long_running` with the caller's agent_id and an unauthorized originating_message. Assert: intercept does NOT match (cross-agent isolation); falls through to `validate_dispatch_readiness` which rejects. This proves the per-agent filter prevents cross-agent authorization leakage.

4. **`test_execute_long_running_intercept_skips_completed_children`** — Create a parent task. Seed a `completed` (or `failed`) deferred-callback child. Invoke `execute_long_running` with an unauthorized originating_message. Assert: intercept does NOT match (only pending/in_progress children authorize); falls through and rejects with `unauthorized_webhook_dispatch`. This proves the fail-closed property after DeferredDispatch resumes and the child completes.

If the `execute_long_running` entry is too heavy to invoke from a unit test (it constructs subprocesses), extract the intercept into a small `intercept_pending_deferred(ctx, task_id) -> Option<ToolOutput>` helper and unit-test the helper directly. This is a refactor for testability, not a separate design change.

### Phase 4 — Eval-harness integration test (NF3)

**File:** `crates/mika-agent/tests/eval/` (extend an existing scenario file, or add a new `dispatch_deferred_retry.rs` keeping with existing naming).

Use `MockLlmProvider` (sequence-based, no network) via `EvalHarness` to exercise the full two-turn sequence:

1. **Turn 1 setup:** Pre-create a parent task in `in_progress` and an "A's groom" callback child in `pending` (to occupy the dispatch slot). Originating message: `[GitHub] Issue labeled ready on issue#789` (authorized).
2. **Turn 1 action:** Mock LLM tool sequence is `run_claude_pilot` with `task_id` = the parent.
3. **Turn 1 expectation:** Tool returns `global_dispatch_active` rejection with `deferred_dispatch_registered: true`. Assert a child task with `label = DEFERRED_DISPATCH_LABEL` exists.
4. **Turn 2 setup:** Same parent task. Originating message: `[GitHub] New comment on issue#789` (unauthorized fallthrough).
5. **Turn 2 action:** Mock LLM tool sequence is `run_claude_pilot` again on the same parent.
6. **Turn 2 expectation:** Tool returns `ToolOutput::success` with `status: "deferred"`, `already_deferred: true`. NO `unauthorized_webhook_dispatch` in the tool output. Assert supervisor task status remains `in_progress` (not `blocked`).

The test anchors on the observable tool-call shape, not on implementation details (e.g., whether the marker lives in metadata or in a child row). This makes the test resilient to future refactors of the intercept's internals.

## Files Changed

| File | Change | Phase |
|------|--------|-------|
| `crates/mika-agent/src/skills/executor.rs` | Insert idempotent-deferred intercept between `validate_task` and `validate_dispatch_readiness` in `execute_long_running`. Add unit tests. Update doc comment on `register_deferred_callback`. | 1, 2, 3 |
| `crates/mika-agent/tests/eval/` (new or extended scenario file) | Two-turn integration test exercising the cross-contamination chain. | 4 |

**Note:** Phases 1 and 3 of the *previous* plan revision (write metadata marker; clear marker after dispatch) are eliminated. No new `update_task_metadata_merge` method is needed.

## Failure-mode analysis (NF4)

The intercept's authorization signal is the *existence* of a pending deferred-callback child row scoped by `agent_id`.

| Scenario | Authorization signal | Intercept behavior | Net effect |
|----------|---------------------|--------------------|-----------|
| LLM retry, deferred child pending | Present | Idempotent ack | LLM gets success-shaped response, no rejection to hallucinate around. DeferredDispatch fires later as normal. (THE BUG WE'RE FIXING) |
| LLM retry, deferred child completed/failed | Absent (status not pending/in_progress) | Falls through to guards | guard (0) rejects with `unauthorized_webhook_dispatch` (existing behavior). LLM may still hallucinate — that's mika#716. |
| LLM retry, deferred child evicted by row deletion | Absent | Falls through | Same as above — fail-closed. |
| Cross-agent: child belongs to other agent | Filtered out by agent_id check | Falls through | guard (0) rejects normally; no cross-agent leakage. |
| First authorized dispatch on a fresh task | Absent | Falls through | All guards apply normally; if guard (0) passes, deferred-callback child is created downstream by `register_deferred_callback`. |
| DB error reading children | Treat as absent (warn) | Falls through | Reverts to current behavior (the bug we're fixing) — not a security regression. The fix is degraded gracefully, guard (0) still protects. |

**Fail mode summary:** fail-closed on every "uncertain authorization" path. The only fail-open case is "the authorization signal IS present" — and that case is exactly when the LLM should be told "your prior dispatch is queued, do nothing."

## Out of Scope

- **The LLM's hallucinated supervisor → blocked transition (mika#716):** This is a separate prompt-discipline issue. The engine-level fix here prevents the *triggering signal* (the confusing `unauthorized_webhook_dispatch` error) from reaching the LLM in the deferred-retry case, but does not address the LLM's broader tendency to mark tasks blocked on unfamiliar errors.
- **DeferredDispatch silent-agent path:** Already correct via `originating_message: None` (mika#920 + mika#1058). No changes needed.
- **Relaxing the positive-consent contract:** mika#841 is load-bearing security; the intercept does not relax it. Guard (0) still fires for all unauthorized turns on tasks without a pending deferred-callback child.
- **The Class D body callout drift (mika#1204):** Separate ticket.
- **A new `update_task_metadata_merge` DB method:** Not needed under the existence-based design.

## Risks

1. **Refactor for testability.** Phase 3 may require extracting the intercept into a small helper function if the full `execute_long_running` entry is too heavy to unit-test (it spawns subprocesses). This is a low-risk refactor; the helper has the same intercept logic and is unit-test-friendly.

2. **`get_child_tasks` cost.** Adds one DB query per `execute_long_running` call, even when no intercept fires. The query is a single-column-indexed lookup (`parent_task_id` is indexed). Performance impact is in the noise compared to subprocess spawn cost.

3. **Race: deferred child fires between intercept check and dispatch attempt.** In execute_long_running, after the intercept query returns "no pending deferred child," the existing guards still run. If DeferredDispatch fires concurrently and creates a callback child during this window, guard (2) `task_active_dispatch` catches it. No double-dispatch is possible.

4. **Team-task trees with shared parent.** If multiple agents share a parent task tree, the per-agent filter on the intercept means each agent's authorization is independent. If agent A registered a deferred dispatch and agent B retries on the same parent, B does NOT get A's idempotent ack — B gets normal guard treatment. This preserves per-agent authorization semantics. (If the desired semantic is the opposite, that's a design decision for mika#1205's follow-up, not this fix.)

## Acceptance Criteria

- **AC1:** When the LLM retries `run_claude_pilot` on a task that has a pending deferred-callback child for the same agent and the originating_message is unauthorized, the tool returns `ToolOutput::success` with `status: "deferred"` and `already_deferred: true`. No `unauthorized_webhook_dispatch` error reaches the LLM.
- **AC2:** When no pending deferred-callback child exists, `execute_long_running` falls through to `validate_dispatch_readiness` and the existing guard behavior is preserved (including `unauthorized_webhook_dispatch` rejection for unauthorized turns).
- **AC3:** When a pending deferred-callback child exists but belongs to a different `agent_id`, the intercept does not fire (per-agent isolation preserved).
- **AC4:** When a deferred-callback child has completed/failed, the intercept does not fire (fail-closed after DeferredDispatch resumes).
- **AC5:** The eval-harness integration test (Phase 4) demonstrates the two-turn sequence end-to-end: first turn defers, second turn (with unauthorized originating_message) receives idempotent ack rather than rejection.
- **AC6:** Existing tests pass — no behavior change for tasks without a pending deferred-callback child.

## Grooming history

- 2026-05-19: First pass via `/mika-groom-ticket`. Architect (`mika-arch-groom-ticket`, session `22011146-0da2-4925-a02e-d8720cd2cf5d`) returned `Disposition: ITERATE` with three blocking findings:
  - F1 (BLOCKING) — Phase 0 Pin absent.
  - F2 (BLOCKING) — `update_task_metadata_merge` does not exist; plan must resolve DB-layer gap.
  - F3 (BLOCKING) — Adopt existence-based design or explicitly reject.
  Plus four non-blocking findings (NF1-NF4). This revision adopts F3 (existence-based design via early intercept in `execute_long_running`), which eliminates F2 entirely. Phase 0 added per F1. NF2-NF4 addressed inline.

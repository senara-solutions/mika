# Plan: fix(verdict-handler): block[ac]/block[ci] verdicts don't re-dispatch when slot frees

**Issue:** senara-solutions/mika#1630
**Type:** fix (loop-breaker class)
**Branch:** `fix/1630/verdict-handler-block-ac-block-ci`

## Problem

`verdict_handler::handle_block_ac` and `handle_block_ci` return `VerdictAction::Handled { pre_digest }` instructing the LLM to call `run_claude_pilot`. This is **LLM-mediated dispatch** — the engine hands off the dispatch decision to the LLM.

When the implement slot is occupied by another task (e.g., a stuck orphan), two failure modes cascade:

1. **LLM non-compliance:** The LLM reads the pre-digest but doesn't call `run_claude_pilot` (misinterpretation, token budget, or unrelated EndTurn guards firing first).
2. **LLM calls it, slot stays busy:** The LLM calls `run_claude_pilot`, `register_deferred_callback` fires, but the blocking dispatch never completes (stuck orphan). The deferred wrapper waits indefinitely — functionally correct but practically equivalent to a loop-break when the orphan outlives the operator's patience.

The founding incident (PR #1626, mika#1612): 3 consecutive `block[ac]` verdicts over ~35 minutes produced zero auto-dispatches. The implement slot was held by an orphan task for CI fix mika#1623. Manual intervention (cancel orphan + tactical spawn) was required.

## Root Cause

The verdict handler for block[ac]/block[ci] relies on **LLM-mediated dispatch**: it returns a prescriptive pre-digest and hopes the LLM calls the tool. Compare with the ready-label handler (mika#1572), which spawns the dispatch subprocess **engine-side** before the LLM turn — eliminating LLM dependency entirely.

The deferred dispatch infrastructure (mika#1011, mika#1175) handles slot-busy scenarios correctly when `register_deferred_callback` is called. But for block[ac]/block[ci], that function is only reached if the LLM actually calls `run_claude_pilot` — the engine never registers the deferred callback itself.

## Solution

Migrate block[ac]/block[ci] to **engine-side dispatch**, following the ready-label handler pattern (mika#1572). The verdict handler will:

1. **Try engine-side dispatch first** (slot free → spawn subprocess directly)
2. **Register deferred callback when slot is busy** (engine-side, no LLM dependency)
3. **Fall back to prescriptive pre-digest** only when other readiness checks fail (non-slot rejections)

This eliminates the LLM-dependency gap for the auto-fix dispatch path.

## Requirements

From issue ACs:

- **AC1:** Reproduce: simulate block[ac] while implement slot held → zero auto-dispatch (current behavior, for regression test baseline)
- **AC2:** Post-fix: same scenario → deferred wrapper registered engine-side; once slot frees, fix dispatched without operator intervention
- **AC3:** N consecutive blocks on same PR collapse to one in-flight fix (idempotent)
- **AC4:** Audit event `verdict_redispatch_deferred` emitted when verdict cannot fire immediately

## Detailed Design

### Step 1: Widen `register_deferred_callback` visibility

**File:** `crates/mika-agent/src/skills/executor.rs`

Change `register_deferred_callback` from `async fn` (private) to `pub(crate) async fn`. The verdict handler (same crate, different module) needs to call it when the slot is busy.

### Step 2: Thread `SkillRegistry` into verdict handler

**File:** `crates/mika-agent/src/server/verdict_handler.rs`

Add `skills: &SkillRegistry` parameter to:
- `try_handle_pr_review_verdict()` (public entry point)
- `handle_block_ac()` (internal)
- `handle_block_ci()` (internal)

**File:** `crates/mika-agent/src/server/handlers.rs`

Pass `&skills` to `try_handle_pr_review_verdict()` at the call site (line ~774). The `skills` binding is already in scope (used by the ready-label handler call below).

### Step 3: Extract shared engine-side dispatch helper

**File:** `crates/mika-agent/src/server/verdict_handler.rs`

Create a shared helper for the engine-side dispatch attempt, reusable by both block[ac] and block[ci]:

```rust
/// Result of an engine-side dispatch attempt from the verdict handler.
enum EngineDispatchResult {
    /// Subprocess spawned; return Dispatched to the LLM.
    Spawned { callback_task_id: String },
    /// Slot busy; deferred callback registered engine-side.
    Deferred { deferred_task_id: String },
    /// Dispatch readiness failed for a non-slot reason; fall back to LLM-mediated.
    Fallback { reason: String },
}
```

```rust
async fn try_engine_dispatch(
    db: &AsyncDatabase,
    skills: &SkillRegistry,
    task_id: &str,
    github_token: Option<&str>,
    event: &PrReviewEvent,
    target_skill: &str, // "dev-pilot"
    target_tool: &str,  // "run_claude_pilot"
    iteration_context: &str, // AC list or CI failure context
    session_id: &str,
    trace_id: &str,
) -> EngineDispatchResult
```

The helper follows the ready-label handler pattern (steps 9a–9i of `ready_label_handler.rs`):

1. **Resolve dispatch tool** from `SkillRegistry` via `skills.resolve_tool_by_name(target_tool)`. On failure → `Fallback`.

2. **Extract handler command** and `estimated_duration_secs` from `ToolHandler::Exec`. On non-long-running → `Fallback`.

3. **Build dispatch input:**
   ```json
   {
     "skill": "dev-pilot",
     "prompt": "<repo>#<pr_number>",
     "task_id": "<task_id>",
     "iteration_context": "<ac_summary or ci_context>"
   }
   ```
   `prompt` uses `<repo>#<pr_number>` (bare form, not owner-qualified — dispatch-lib only accepts this form, per mika#1593).

4. **Call `validate_dispatch_readiness()`** with `originating_message = None` (engine-side dispatches are pre-authorized by the verdict handler's own event-type gate — the webhook already passed the PR review parse).

5. **Branch on result:**
   - **Passes:** Create callback task via `build_callback_task`, spawn subprocess via `spawn_long_running_exec`, auto-transition parent to `in_progress` → `Spawned`.
   - **Fails with `global_dispatch_active` error:** Call `register_deferred_callback(db, task_id, &dispatch_input)` → `Deferred`. (The deferred infrastructure handles promotion when the slot frees.)
   - **Fails with other error:** → `Fallback` (let the LLM-mediated path try; may hit different guard states).

6. **Slot-busy detection:** Parse the rejection JSON for `"error": "global_dispatch_active"`. The rejection is a JSON string; extract the `"error"` field to discriminate slot-busy from other rejections.

### Step 4: Wire engine-side dispatch into `handle_block_ac`

**File:** `crates/mika-agent/src/server/verdict_handler.rs`

After the existing retry counter increment and AC extraction (lines 693–728), insert the engine-side dispatch attempt:

```rust
// Engine-side dispatch (mika#1630) — eliminate LLM-dependency gap.
match try_engine_dispatch(
    db, skills, &task_id, github_token, event,
    "dev-pilot", "run_claude_pilot", &ac_summary,
    session_id, trace_id,
).await {
    EngineDispatchResult::Spawned { callback_task_id } => {
        // Subprocess spawned engine-side. Return Dispatched so the LLM
        // acknowledges without dispatching again.
        return VerdictAction::Dispatched {
            pre_digest: format_block_ac_dispatched_pre_digest(
                event, &ac_summary, &task_id, new_count, &callback_task_id,
            ),
            task_id: task_id.clone(),
        };
    }
    EngineDispatchResult::Deferred { deferred_task_id } => {
        // Slot busy; deferred callback registered engine-side (AC2).
        // Emit audit event (AC4).
        let _ = db.log_audit_event(
            session_id,
            "verdict_redispatch_deferred",
            &format!("task:{task_id}"),
            None,
            Some("deferred"),
            Some(&format!(
                "verdict=block[ac] deferred_id={deferred_task_id} pr_url={}",
                event.pr_url()
            )),
            Some(trace_id),
        ).await;
        // Return Handled with deferred notice — the LLM runs but does NOT
        // need to dispatch (the deferred wrapper handles it).
        return VerdictAction::Handled {
            pre_digest: format_block_ac_deferred_pre_digest(
                event, &ac_summary, &task_id, new_count,
            ),
        };
    }
    EngineDispatchResult::Fallback { reason } => {
        // Non-slot readiness failure. Fall through to the existing
        // LLM-mediated prescriptive pre-digest (backward compatible).
        warn!(
            task_id = %task_id,
            reason = %reason,
            "verdict engine-dispatch fallback — LLM-mediated path"
        );
    }
}

// Existing code: return prescriptive pre-digest for LLM-mediated dispatch
VerdictAction::Handled {
    pre_digest: format_block_ac_pre_digest(event, &ac_summary, &task_id, new_count),
}
```

### Step 5: Wire engine-side dispatch into `handle_block_ci`

Same pattern as Step 4, with `"dev-pilot"` / `"run_claude_pilot"` and CI context instead of AC summary.

### Step 6: Add pre-digest formatters

**File:** `crates/mika-agent/src/server/verdict_handler.rs`

Two new formatters per verdict class (4 total):

- `format_block_ac_dispatched_pre_digest` — engine-side dispatch succeeded; tells LLM the dispatch already fired (mirrors `format_engine_dispatch_pre_digest` in ready-label handler).
- `format_block_ac_deferred_pre_digest` — slot busy, deferred callback registered; tells LLM the fix is queued and will auto-fire when the slot frees. The LLM should NOT call `run_claude_pilot`.
- `format_block_ci_dispatched_pre_digest` — same, CI variant.
- `format_block_ci_deferred_pre_digest` — same, CI variant.

All pre-digests start with `<verdict_handler>` to avoid triggering INTENT_GUARDS.

### Step 7: Idempotent collapse (AC3)

The existing `has_active_callback_child(db, &task_id)` check at line 573 already catches both real dispatches AND deferred wrappers (both have `trigger_type='callback'` and `status='pending'` or `status='in_progress'`). When a deferred wrapper is pending, a subsequent block[ac] verdict for the same task will hit this check and return "fix already in-flight."

**Enhancement:** Distinguish the pre-digest message between "fix is actively running" (real dispatch) and "fix is queued, will fire when slot frees" (deferred wrapper). Check the child's label for `:deferred` suffix:

```rust
if has_active_callback_child(db, &task_id).await {
    let is_deferred = has_pending_deferred_child(db, &task_id).await;
    let status_msg = if is_deferred {
        "a fix is queued (deferred — waiting for dispatch slot)"
    } else {
        "a fix is already in-flight"
    };
    return VerdictAction::Handled {
        pre_digest: format!("... {} for task {task_id} ...", status_msg),
    };
}
```

Add a helper `has_pending_deferred_child` that checks child tasks for the `:deferred` label suffix.

### Step 8: Handle `VerdictAction::Dispatched` in handlers.rs for verdict handler

**File:** `crates/mika-agent/src/server/handlers.rs`

The verdict handler's match arm at line 783 currently has:
```rust
VerdictAction::Dispatched { .. } => {}
```

This is a no-op because previously only the ready-label handler returned `Dispatched`. Now block[ac]/block[ci] can also return it. The behavior is correct — `Dispatched` pre-digests replace `req.text` the same way `Handled` does. Update the match arm:

```rust
VerdictAction::Dispatched { pre_digest, .. } => {
    req.text = pre_digest;
}
```

### Step 9: Tests

#### Unit tests (verdict_handler.rs)

1. **`block_ac_engine_dispatch_spawns_when_slot_free`** — Mock SkillRegistry with a resolvable `run_claude_pilot` tool. Set up a task with no active callbacks. Call `handle_block_ac`. Assert `VerdictAction::Dispatched` returned and callback task created in DB.

2. **`block_ac_registers_deferred_when_slot_busy`** — Set up a task + another task with an active callback (occupying the implement slot). Call `handle_block_ac`. Assert `VerdictAction::Handled` with deferred notice in pre-digest. Assert deferred callback task exists in DB with label `long_running:run_claude_pilot:deferred`. Assert `verdict_redispatch_deferred` audit event.

3. **`block_ac_idempotent_collapse`** — Set up a task with a pending deferred child. Call `handle_block_ac`. Assert `VerdictAction::Handled` with "fix is queued" message. No new tasks created.

4. **`block_ci_engine_dispatch_spawns_when_slot_free`** — Mirror of test 1 for block[ci].

5. **`block_ci_registers_deferred_when_slot_busy`** — Mirror of test 2 for block[ci].

6. **`block_ac_fallback_to_llm_when_tool_not_found`** — SkillRegistry without `run_claude_pilot`. Assert `VerdictAction::Handled` with existing prescriptive pre-digest (LLM-mediated fallback).

#### Eval grounding regression (optional)

If time permits, add a scenario to `tests/eval/grounding_regressions/` for the verdict-handler dispatch-gap failure class.

## Verification Contract

| Signal | What to check | Expected post-fix |
|--------|--------------|-------------------|
| S1 — Engine dispatch fires | `grep verdict_engine_dispatch server.log` | Event emitted on block[ac]/block[ci] when slot is free |
| S2 — Deferred registration | `grep verdict_redispatch_deferred server.log` | Event emitted when slot is busy (AC4) |
| S3 — Deferred promotion | `grep deferred_dispatch_promoted server.log` | Deferred wrapper promotes when blocking dispatch completes |
| S4 — Idempotent collapse | Consecutive block[ac] verdicts on same PR → single callback child | No duplicate deferred wrappers |
| S5 — Fallback path | `grep "verdict engine-dispatch fallback" server.log` | LLM-mediated path fires when SkillRegistry can't resolve tool |

## Risks

1. **Pre-digest collision with INTENT_GUARDS.** The `webhook_no_unauthorized_dispatch` guard (entry b in INTENT_GUARDS) rejects turns where `run_claude_pilot` was successfully called AND the message starts with `[GitHub]`. For `Dispatched` returns, the subprocess is spawned engine-side (not via the LLM tool call), so the guard's tool-call check doesn't fire. The `<verdict_handler>` prefix in pre-digests also prevents false-positive matching. **Mitigation:** Existing — the guard only checks LLM-initiated tool calls.

2. **`originating_message` for validate_dispatch_readiness.** The verdict handler's engine-side dispatch passes `originating_message = None` because the dispatch is engine-authorized. Guard (0) (`unauthorized_webhook_dispatch`) only fires when `originating_message.is_some()`. Passing `None` skips guard (0) — which is correct because the verdict handler already validated the event type. The ready-label handler passes the original text; we pass `None` for simplicity. Both are safe.

3. **Retry counter was already incremented.** The verdict handler increments the retry counter BEFORE the engine-side dispatch attempt. If the dispatch is deferred, the counter reflects the attempt (correct). If the same verdict retriggers (qa syncs on same HEAD), `has_active_callback_child` catches the deferred wrapper and skips (AC3).

## Definition of Done

- [ ] `register_deferred_callback` is `pub(crate)` in `executor.rs`
- [ ] `SkillRegistry` threaded into verdict handler
- [ ] `try_engine_dispatch` helper implemented
- [ ] `handle_block_ac` uses engine-side dispatch with deferred fallback
- [ ] `handle_block_ci` uses engine-side dispatch with deferred fallback
- [ ] `VerdictAction::Dispatched` handled in handlers.rs verdict match arm
- [ ] Deferred pre-digest formatters added
- [ ] Idempotent collapse distinguishes "in-flight" from "queued"
- [ ] `verdict_redispatch_deferred` audit event emitted (AC4)
- [ ] Unit tests pass for engine dispatch, deferred registration, idempotent collapse
- [ ] `cargo clippy` clean
- [ ] `cargo test` passes

## Acceptance criteria

- **AC1 (reproduce):** Unit test `block_ac_registers_deferred_when_slot_busy` demonstrates current-equivalent scenario: block[ac] verdict while slot held → deferred wrapper created (vs zero dispatch in current code).
- **AC2 (auto-dispatch):** When slot frees (blocking dispatch completes), the periodic backstop in `engine::tick_loop` promotes the deferred wrapper → fix dispatch fires without operator intervention. Verified by S3 signal.
- **AC3 (idempotent collapse):** Unit test `block_ac_idempotent_collapse` → N consecutive blocks on same PR produce exactly one queued fix attempt.
- **AC4 (audit event):** Unit test `block_ac_registers_deferred_when_slot_busy` asserts `verdict_redispatch_deferred` audit event written.

## Out of Scope

- **Orphan reaper improvements:** The orphan task that blocked the slot in the founding incident is a separate concern (mika#871, mika#1162). This fix ensures the verdict-driven dispatch survives a busy slot; orphan reaping is independent.
- **CI failure handler engine-side dispatch:** `ci_failure_handler` has a similar LLM-mediated dispatch pattern. Deferring to a separate ticket if the pattern proves successful here.
- **Periodic verdict-replay tick:** The issue proposed an alternative (lighter) approach of replaying recent verdict events. Engine-side dispatch + deferred callback is the chosen approach — it composes with existing infrastructure and has proven reliability from the ready-label handler.

# Plan: chore(observability,agent-native) — close deferred-dispatch monitoring + agent-tool parity gaps (#1172)

**Issue:** mika#1172
**Type:** chore (observability + agent-native parity)
**Branch:** `chore/1172/observability-agent-native-close`

## Problem

mika#1163's CE review surfaced five observability and agent-native parity gaps in the deferred-dispatch subsystem. Operators can diagnose dispatch issues via SQL + log grep, but agents (mika-dev) cannot self-diagnose because the tool surface omits `dispatch_class`, doesn't enumerate `:deferred` wrappers, and doesn't expose blocker identity in rejection JSON. Additionally, the no-op-cascade failure mode (mika#1124) logs only at DEBUG — a recurrence would be invisible without targeted log-level changes.

## Acceptance Criteria Tie-backs

- **AC-R9:** Emit `deferred_dispatch_noop_completion` WARN + `audit_events` row when a wrapper's silent turn completes without registering a real callback child.
- **AC-W1:** Surface pending `:deferred` wrappers via an agent tool — add `label_contains` filter to `list_scheduled_tasks`.
- **AC-W2:** Add `dispatch_class` to `get_task` and `list_tasks` tool output.
- **AC-W3:** Add `blocker_kind` to `global_dispatch_active` rejection JSON in `validate_dispatch_readiness`.
- **AC-W4:** Scoped version — write `audit_events` rows for the three dispatch lifecycle events most relevant to debugging: `deferred_dispatch_promoted`, `deferred_dispatch_registered`, `deferred_dispatch_noop_completion` (from R9).

## Design Decisions

### D1: `list_scheduled_tasks` label filter (W1) vs new `list_dispatch_queue` tool

**Decision:** Add `label_contains` parameter to `list_scheduled_tasks`.

**Rationale:** The deferred wrappers are already callback-type tasks visible to `get_tasks_by_status`. Adding a label filter is the smallest surface change — one new optional parameter, one SQL `AND` clause. A dedicated `list_dispatch_queue` tool would be cleaner semantically but adds a new tool to the registry for a narrow use case. The agent can call `list_scheduled_tasks` with `label_contains: ":deferred"` to enumerate wrappers. The tool description already covers "scheduled tasks including... other time/event-triggered tasks" — callback wrappers fit this framing.

### D2: W4 scope — three events, not a general-purpose audit-event expansion

**Decision:** Scope W4 to three dispatch lifecycle events only.

**Rationale:** The ticket's out-of-scope section explicitly excludes "general `audit_events` schema expansion beyond the dispatch lifecycle." Writing audit rows for `deferred_dispatch_promoted`, `deferred_dispatch_registered`, and `deferred_dispatch_noop_completion` (the R9 event) covers the dispatch debugging surface without opening a broader epic. Each uses the existing `log_audit_event` pattern (tool_name, resource_id, old_value, new_value, note). No schema changes needed — `audit_events` table is general-purpose.

### D3: `blocker_kind` derivation (W3)

**Decision:** Derive `blocker_kind` from the blocking callback's label suffix.

**Rationale:** The `has_active_callback_tasks_excluding` query already excludes `label LIKE '%:deferred'` wrappers from the slot guard. But the rejection JSON is built from whatever callback IS blocking. To determine `blocker_kind`, we need the label of the blocking callback. The current query returns `(parent_task_id, callback_id)` — we need to also fetch the blocking callback's label and check for `:deferred` suffix. This means either: (a) a second query, or (b) extending the existing query to return the label. Option (b) is cleaner — add `label` to the SELECT and return a 3-tuple.

### D4: R9 detection — "no real callback child created during the silent turn"

**Decision:** Detect by checking the wrapper's parent task for active callback children at wrapper-completion time.

**Rationale:** The existing code at `dispatcher.rs:502` already branches on `task.label == DEFERRED_DISPATCH_LABEL`. The healthy path is: wrapper silent turn → LLM calls `run_claude_pilot` → new callback child created → wrapper completes. The degraded path: wrapper silent turn → LLM does NOT call `run_claude_pilot` → wrapper completes with no new child. Detection: after `mark_task_delivered`, if the completing task is a deferred wrapper, check whether the parent task now has any active (pending/in_progress) non-deferred callback child. If no active child exists, emit WARN + audit event. If an active child exists, the wrapper did its job (healthy completion).

## Scope

### In scope

1. **R9:** WARN log + audit event on no-op wrapper completion (dispatcher.rs)
2. **W1:** `label_contains` filter on `list_scheduled_tasks` tool
3. **W2:** `dispatch_class` in `get_task` and `list_tasks` output
4. **W3:** `blocker_kind` field in `global_dispatch_active` rejection JSON
5. **W4:** Audit event rows for 3 dispatch lifecycle events

### Out of scope

- Backstop class-blindness (mika#1171)
- Parent task auto-transition (mika#1162)
- General `audit_events` schema expansion beyond dispatch lifecycle

## Implementation

### Phase 1: W2 — Surface `dispatch_class` in tool output (smallest, no dependencies)

**File:** `crates/mika-agent/src/tools/get_task.rs`

1. Add `dispatch_class` to the format string at line 57-59:
   ```
   Dispatch class: {}
   ```
   Value: `task.dispatch_class.as_deref().unwrap_or("implement")` (mirrors SQL COALESCE semantics from v34 schema).

**File:** `crates/mika-agent/src/tools/list_tasks.rs`

2. Add `dispatch_class` to the per-item format at line 159-164. Follow the same compact-unless-interesting pattern used for `task_type` (line 146-149): hide `"implement"` (the default), surface `"groom"`:
   ```rust
   let dispatch_cls = match task.dispatch_class.as_deref() {
       Some("groom") => " class:groom",
       _ => "", // "implement" is the default — omit for compact output
   };
   ```
   Append `{dispatch_cls}` to the format string.

**Tests:** Add unit test in each tool file confirming `dispatch_class` appears in output for both `Some("groom")` and `None` (default) cases. Follow existing test patterns in those files.

### Phase 2: W1 — `label_contains` filter on `list_scheduled_tasks`

**File:** `crates/mika-agent/src/tools/list_scheduled_tasks.rs`

3. Add `label_contains` optional string parameter to the tool's `input_schema` (alongside existing `status`):
   ```json
   "label_contains": {
       "type": "string",
       "description": "Optional substring filter on task label. Use ':deferred' to find pending deferred dispatch wrappers."
   }
   ```

4. In `execute()`, extract `label_contains` from input. Pass to a new or modified DB query.

**File:** `crates/mika-agent/src/db.rs`

5. Modify `get_tasks_by_status` (or add a new method `get_tasks_by_status_and_label`) to accept an optional `label_contains: Option<&str>` parameter. Add `AND (?N IS NULL OR label LIKE '%' || ?N || '%')` to the WHERE clause. This is safe because the `LIKE` pattern is parameterized (no SQL injection) and the label column is indexed.

**Tests:** Unit test confirming `:deferred` filter returns only deferred wrappers when mixed task types exist.

### Phase 3: W3 — `blocker_kind` in rejection JSON

**File:** `crates/mika-agent/src/db.rs`

6. Extend `has_active_callback_tasks_excluding` to return a 3-tuple `(String, String, String)` — `(parent_task_id, callback_id, callback_label)`. Change the SELECT at line 5892 to include `label`:
   ```sql
   SELECT parent_task_id, id, label FROM tasks WHERE ...
   ```

**File:** `crates/mika-agent/src/skills/executor.rs`

7. At line 953, destructure the new 3-tuple. Derive `blocker_kind`:
   ```rust
   let blocker_kind = if blocking_label.ends_with(":deferred") {
       "deferred_wrapper"
   } else {
       "real_callback"
   };
   ```

8. Add `"blocker_kind": blocker_kind` to the rejection JSON at line 965-978. Also add `"blocking_label": blocking_label` for full transparency.

**File:** `crates/mika-agent/src/db.rs` (async wrapper)

9. Update the `AsyncDatabase` wrapper method's return type to match.

**Tests:** Unit test in `executor.rs` or integration test confirming `blocker_kind` appears in rejection JSON for both real and deferred blockers.

### Phase 4: R9 — WARN + audit event on no-op wrapper completion

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`

10. In the `else` branch at line 504-509 (the `DEFERRED_DISPATCH_LABEL` path), after the existing `debug!` log:
    - Query for active callback children of the wrapper's parent task: `db.has_active_callback_child(parent_task_id)`. The parent is `task.parent_task_id` (deferred wrappers always have a parent).
    - If NO active child exists → emit WARN:
      ```rust
      warn!(
          event = "deferred_dispatch_noop_completion",
          task_id = %task.id,
          parent_task_id = %parent_id,
          "deferred wrapper completed without spawning a real callback — no-op cascade risk (mika#1124)"
      );
      ```
    - If an active child exists → healthy path, log at DEBUG only.

11. For the `has_active_callback_child` check, we can reuse an existing query or add a lightweight one. Check if parent has any callback child in `pending`/`in_progress` status with `label NOT LIKE '%:deferred'` — this tells us a real dispatch was spawned.

**File:** `crates/mika-agent/src/db.rs`

12. Add `has_non_deferred_active_callback_child(parent_task_id: &str) -> Result<bool>`:
    ```sql
    SELECT EXISTS(
        SELECT 1 FROM tasks
        WHERE parent_task_id = ?1
          AND trigger_type = 'callback'
          AND status IN ('pending', 'in_progress')
          AND label NOT LIKE '%:deferred'
    )
    ```

### Phase 5: W4 — Audit event rows for dispatch lifecycle events

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`

13. At the R9 WARN site (from step 10), also write an audit event:
    ```rust
    db.log_audit_event(
        &session_id,
        "deferred_dispatch_noop_completion",
        &format!("task:{}", task.id),
        None,
        Some("noop_completion"),
        Some(&format!("parent:{parent_id} — wrapper completed without real dispatch")),
        Some(&trace_id),
    ).await;
    ```

14. At the `deferred_dispatch_promoted` info event sites (lines 997 and 1015), add audit events:
    ```rust
    // After the info! log at line 997 (class-agnostic promotion)
    db.log_audit_event(
        "system",
        "deferred_dispatch_promoted",
        &format!("task:{promoted_task_id}"),
        Some("pending"),
        Some("completed"),
        Some("inline promotion after dispatch completion"),
        None,
    ).await;
    ```
    Note: The promotion methods (`promote_next_deferred_callback` and `promote_next_deferred_callback_for_class`) currently return `bool` — they don't expose the promoted task's ID. To write a meaningful audit event, we need to return the promoted task ID. Modify the DB methods to return `Option<String>` instead of `bool`.

15. At the `register_deferred_callback` site (executor.rs line 960), add an audit event after successful registration:
    ```rust
    if deferred_registered {
        // existing: rejection["deferred_dispatch_registered"] = ...
        db.log_audit_event(
            "system",
            "deferred_dispatch_registered",
            &format!("task:{task_id}"),
            None,
            Some("deferred"),
            Some(&format!("dispatch_class:{class}, blocking:{blocking_parent_id}")),
            None,
        ).await;
    }
    ```

**File:** `crates/mika-agent/src/db.rs`

16. Update `promote_next_deferred_callback()` and `promote_next_deferred_callback_for_class()` return types from `Result<bool>` to `Result<Option<String>>` (returning the promoted task's ID when one was promoted). This enables meaningful audit event resource_id in step 14.

### Phase 6: Tests

17. **Unit tests for W2:** In `get_task.rs` and `list_tasks.rs`, add tests confirming `dispatch_class` / `class:groom` appears in output.

18. **Unit test for W1:** In `list_scheduled_tasks.rs`, add test confirming `label_contains` filter works.

19. **Integration test for W3:** Confirm `blocker_kind` and `blocking_label` appear in `global_dispatch_active` rejection JSON.

20. **Integration test for R9:** Using `EvalHarness` or direct dispatcher test, confirm WARN event fires when a deferred wrapper completes without spawning a real callback. This may be a unit test on the dispatcher method if the wrapper-completion path is testable in isolation.

21. **Unit test for W4:** Confirm audit events are written for each of the three lifecycle events.

### Phase 7: Documentation

22. Update `crates/mika-agent/CLAUDE.md`:
    - In the `list_scheduled_tasks` tool description, mention the `label_contains` filter.
    - In the `validate_dispatch_readiness` section, mention `blocker_kind` field.
    - In the deferred-dispatch section, mention the R9 WARN event and W4 audit events.

23. **(AC-R9 runbook)** Update root `CLAUDE.md` § "Post-restart safety check" with a new **Signal J** for the `deferred_dispatch_noop_completion` grep pattern:
    ```
    - **Signal J — no-op wrapper detection (#1172).** `grep deferred_dispatch_noop_completion server.log` — any hits indicate a deferred wrapper completed its silent turn without spawning a real `run_claude_pilot` dispatch. This is the failure mode mika#1124 fixed; a hit post-deploy means the fix regressed or a new no-op-cascade variant appeared. Investigate the parent task ID in the log event to determine whether the dispatch slot is stuck.
    ```

## File Change Summary

| File | Changes |
|------|---------|
| `crates/mika-agent/src/tools/get_task.rs` | Add `dispatch_class` to output format |
| `crates/mika-agent/src/tools/list_tasks.rs` | Add `dispatch_class` to per-item format (compact) |
| `crates/mika-agent/src/tools/list_scheduled_tasks.rs` | Add `label_contains` parameter + filter |
| `crates/mika-agent/src/db.rs` | Modify `get_tasks_by_status` for label filter; extend `has_active_callback_tasks_excluding` to return label; add `has_non_deferred_active_callback_child`; update promote return types |
| `crates/mika-agent/src/skills/executor.rs` | Derive and emit `blocker_kind`; add audit event for `deferred_dispatch_registered` |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | R9 WARN + audit event on no-op wrapper completion; audit events for `deferred_dispatch_promoted` |
| `crates/mika-agent/CLAUDE.md` | Document new tool parameters, rejection fields, log events |
| `CLAUDE.md` (root) | Add Signal J to § Post-restart safety check for `deferred_dispatch_noop_completion` |

## Risks and Mitigations

- **R1: DB query return type change (Phase 3, step 6).** Changing `has_active_callback_tasks_excluding` from 2-tuple to 3-tuple is a breaking internal API change. **Mitigation:** This is an internal method — grep for all callers and update them. There should be exactly 2 callers (the guard in executor.rs and possibly a test).

- **R2: promote method return type change (Phase 5, step 16).** Changing `promote_next_deferred_callback*` from `bool` to `Option<String>` touches the dispatcher's inline and periodic promotion paths. **Mitigation:** All callers currently only check truthiness (`Ok(true)` → log, `Ok(false)` → no-op). Changing to `Ok(Some(id))` → log with id, `Ok(None)` → no-op preserves the same control flow with richer data.

- **R3: Audit event volume (Phase 5).** Each deferred dispatch cycle writes 2-3 audit rows. **Mitigation:** Deferred dispatches are rare (only when `global_dispatch_active` fires), so this adds negligible write volume. The existing audit table has no TTL concerns at current scale.

## Open Questions

None — all five items are well-specified in the ticket body with concrete code sites.

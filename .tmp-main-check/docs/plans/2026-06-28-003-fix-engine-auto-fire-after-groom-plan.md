# Plan: fix(engine): auto-fire-after-groom deterministic dispatch (mika#1614)

## Problem

`try_dispatch_pilot_after_groom_success()` (dispatcher.rs:1784) fires after a groom callback delivers with `Outcome: PLAN_GROOMED`. Its current mechanism is **indirect**: it runs `gh issue edit --add-label ready` to re-add the `ready` label, then relies on:

1. GitHub webhook round-trip back to the server
2. `try_handle_ready_label_dispatch()` intercepting the webhook
3. That handler spawning the dev-pilot subprocess

This indirection is vulnerable to **fabrication**: the LLM in the silent callback turn can emit a log line claiming dispatch fired without the `gh issue edit` tool ever running. Observed on mika#1609 (2026-06-28 at 11:57:09Z) — the engine logged "auto-fired dev-pilot dispatch" but no claude-pilot task was created. Recovery required manual ready-label re-toggle.

## Root cause

The `gh issue edit --add-label ready` command runs from within the engine's `try_dispatch_pilot_after_groom_success()` function, which is correct (engine-side, not LLM-mediated). However, the actual dev-pilot subprocess spawn depends on the GitHub webhook round-trip completing successfully and the server receiving it. The observed failure mode on mika#1609 suggests either the `gh` subprocess failed silently, the webhook was lost/delayed, or the label was already present (GitHub doesn't re-fire a webhook for a no-op label add).

## Proposed fix

Replace the indirect `gh issue edit --add-label ready` → webhook round-trip → `ready_label_handler` chain with a **direct engine-side subprocess spawn** — the same `spawn_long_running_exec("run_claude_pilot", ...)` path that `ready_label_handler` step 9 (lines 235–432) already uses for the initial dispatch.

The auto-fire function becomes a self-contained dispatcher: it creates the parent task, the callback child task, and spawns the subprocess — all within the same synchronous engine call. No GitHub API call, no webhook dependency, no round-trip.

## Scope

### In scope

- Rewrite `try_dispatch_pilot_after_groom_success()` to spawn dev-pilot directly
- Expand function signature to accept `skills: &SkillRegistry` (already on `TaskDispatcher`)
- Add dispatch-readiness validation (reuse `validate_dispatch_readiness`)
- Add audit events for the direct dispatch path

### Out of scope

- `ready_label_handler` itself (already deterministic per mika#1572)
- The LLM-mediated prompt-level path in `self-dev-callback` (defense-in-depth, unchanged)
- Deferred dispatch registration (if slot is occupied, the direct spawn should fail gracefully and the existing prompt-level path or next heartbeat picks it up)

## Implementation

### Step 1: Expand `try_dispatch_pilot_after_groom_success` signature

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`

Add `skills: &SkillRegistry` parameter to the function signature. The caller at line 429 already has access to `self.skills` on `TaskDispatcher`.

### Step 2: Rewrite function body — direct subprocess spawn

Replace the `gh issue edit --add-label ready` mechanism (lines 1831–1898) with a direct dispatch path mirroring `ready_label_handler` steps 9a–9i:

1. **Keep existing preconditions** (lines 1790–1829): dispatch_class == "groom", result contains "Outcome: PLAN_GROOMED", parent has parseable reference_url, GitHub token present. These remain the trigger gate.

2. **Resolve dispatch target** (new): Since we know grooming succeeded, the target is always `("run_claude_pilot", "dev-pilot", "implement")` — the implement-class dispatch. No need to re-check grooming state.

3. **Resolve tool from SkillRegistry** (mirror 9a): `skills.resolve_tool_by_name("run_claude_pilot")`. On failure, log WARN and return (fire-and-forget — the prompt-level path remains as defense-in-depth).

4. **Extract command + estimated_duration** (mirror 9b): Match `ToolHandler::Exec { long_running: true, .. }`. On mismatch, log WARN and return.

5. **Build dispatch input** (mirror 9c):
   ```rust
   let dispatch_input = serde_json::json!({
       "skill": "dev-pilot",
       "prompt": format!("{}#{}", repo, issue_num),
       "task_id": parent_task_id,  // reuse the existing parent task
   });
   ```

6. **Create implement-class parent task** (new): Create a new `NewTask` with `dispatch_class: Some("implement")`, `trigger_type: "manual"`, `source: Some("self_dev")`, `reference_url` from the parent's reference_url. This is a new parent task for the implement dispatch (the groom parent task is already completed/completing). Use label format: `"auto-fire: {repo}#{issue_num}"`.

7. **Validate dispatch readiness** (mirror 9d): Call `validate_dispatch_readiness()` with the new parent task ID. On rejection, log WARN and return. This catches slot contention, active callbacks, and blockedBy issues. The grooming-marker check (#919) will pass because the groom just completed successfully.

8. **Create callback child task** (mirror 9e): Use `build_callback_task()` with `tool_name = "run_claude_pilot"`, `parent_task_id = new_parent_id`. Timeout from estimated_duration_secs.

9. **Verify handler script exists** (mirror 9f): Check `cmd_path.exists()`. On failure, mark callback failed and return.

10. **Auto-transition parent to in_progress** (mirror 9g).

11. **Inject subprocess metadata** (mirror 9h): `__mika_task_id`, `__mika_agent`.

12. **Spawn subprocess** (mirror 9i): `spawn_long_running_exec(...)`.

13. **Audit event**: Write `tool_name = "task_engine_groom_pilot_dispatcher"` with `before_value = "groom_delivered"`, `after_value = "implement_dispatched"` (distinguish from the old `ready_label_re_added`).

### Step 3: Update caller

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`

Update the call site at line 429 to pass `&self.skills`:

```rust
try_dispatch_pilot_after_groom_success(
    &self.db,
    task,
    self.github_token.as_deref(),
    &self.skills,  // new
).await;
```

### Step 4: Handle task reuse vs. new task creation

The groom callback's parent task has `dispatch_class: Some("groom")`. For the implement dispatch, we need a new parent task with `dispatch_class: Some("implement")`. This matches the existing pattern where `ready_label_handler` creates a fresh parent task for each dispatch.

Key detail: use the same `reference_url` from the groom parent so the grooming-marker check (#919) in `validate_dispatch_readiness` can fetch the issue body and verify the Plan callout is present.

### Step 5: Add/update tests

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs` (inline `#[cfg(test)]` module)

1. **Test: auto-fire creates implement-class parent + callback and would spawn subprocess** — mock the SkillRegistry to return a dev-pilot tool, verify task creation with correct dispatch_class.

2. **Test: auto-fire skips when dispatch slot is occupied** — verify WARN log and graceful return when `validate_dispatch_readiness` rejects.

3. **Test: auto-fire skips for non-groom callbacks** — existing test coverage, verify it still passes.

4. **Test: auto-fire skips when result doesn't contain PLAN_GROOMED** — existing test coverage.

5. **Remove or update existing test that verifies `gh issue edit --add-label ready`** — the old mechanism is replaced.

## Signal verification

After deploy, verify with:

```bash
# New signal: direct dispatch after groom success
grep "engine: auto-fired dev-pilot dispatch after groom success" server.log | jq 'select(.after_value == "implement_dispatched")'

# Confirm no webhook round-trip needed
grep "ready_label_engine_dispatched" server.log  # Should NOT appear for auto-fire cases (only for webhook-triggered dispatches)
```

## Risk assessment

- **Low risk**: The `ready_label_handler` dispatch path (steps 9a–9i) is battle-tested — this change mirrors it exactly.
- **Defense-in-depth preserved**: The prompt-level `self-dev-callback` path still runs and can re-add the ready label as a fallback. The `ready_label_handler` still handles manual ready-label additions.
- **Rollback**: If the direct spawn has issues, the old `gh issue edit --add-label ready` mechanism can be restored by reverting this change. The webhook path will continue to work.

## Acceptance criteria

1. After a successful groom callback (`Outcome: PLAN_GROOMED`), the engine spawns `run_claude_pilot` directly — no `gh issue edit` call, no webhook round-trip
2. The spawned subprocess creates a real claude-pilot task (verifiable via `list_tasks` or dashboard)
3. Dispatch-readiness checks (slot availability, grooming markers, blockedBy) are respected
4. When the dispatch slot is occupied, the auto-fire logs a WARN and returns gracefully (deferred dispatch or prompt-level path picks it up)
5. Existing tests pass; new tests cover the direct dispatch path
6. `cargo clippy` and `cargo test` pass

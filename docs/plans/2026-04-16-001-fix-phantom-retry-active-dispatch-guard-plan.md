---
title: "fix: Prevent phantom pipeline retries during active dispatch"
type: fix
status: active
date: 2026-04-16
---

# fix: Prevent phantom pipeline retries during active dispatch

## Overview

Add a code-level guard to `update_work_item_status` that rejects retry-semantic metadata writes when the work item has an active callback child task (i.e., a claude-pilot session is still running). Harden the self-dev skill prompt as defense-in-depth. This prevents the LLM from fabricating pipeline failures and consuming retry budget before any callback has returned.

## Problem Frame

At 13:54:51 UTC, mika-dev sent "Pipeline produced no commits for mika#334 — retrying (1/2)" only 67 seconds after launching claude-pilot. No callback had returned — the pipeline was still running. The LLM hallucinated a failure, wrote fabricated `pipeline_retry_count` metadata, and attempted a re-dispatch. The re-dispatch was blocked by the existing `validate_dispatch_readiness` guard (#525), but the fabricated metadata persisted — polluting the retry budget for subsequent real callbacks.

This is the same class of LLM nonconformance documented in `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md` (#525) and `docs/solutions/architecture-patterns/fabricated-action-claim-guard.md` (#308): prompt-level contracts are not reliable under model drift. The established pattern is code guards at the tool boundary.

## Requirements Trace

- R1. Retry-semantic metadata writes on a work item are rejected when an active callback child task exists
- R2. Non-retry metadata writes (notes, branch, pr_url) are unaffected during active dispatch
- R3. Engine-internal metadata writes (`try_extract_callback_metadata` in dispatcher.rs) are unaffected
- R4. Self-dev prompt contains explicit anti-hallucination guardrails for retry behavior
- R5. Guard returns structured JSON errors consistent with existing dispatch guards

## Scope Boundaries

- The re-dispatch itself is already blocked by `validate_dispatch_readiness` (#525) — this fix addresses the metadata pollution vector
- No schema changes required — `pipeline_retry_count` is a JSON metadata field, not a DB column
- No changes to the task engine or callback lifecycle

### Deferred to Separate Tasks

- Telemetry audit of the 13:53-13:55 UTC window: separate investigation comment on #579
- General LLM fabrication detection framework: future iteration

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/tools/update_work_item_status.rs` — the tool where the guard will be added
- `crates/mika-agent/src/skills/executor.rs:562` — `validate_dispatch_readiness()` with active-callback-child query pattern to reuse
- `crates/mika-agent/src/server/webhook_queue.rs:126` — `has_active_callback_child()` helper (same query pattern)
- `crates/mika-agent/src/task_engine/dispatcher.rs` — engine-level `try_extract_callback_metadata()` writes to parent work item metadata (must NOT be affected)
- `mika-skills/self-dev/system_prompt.md` — retry instructions at "On pipeline failure" section

### Institutional Learnings

- **Code guards over prompt instructions** — dominant pattern across 6 solutions (#308, #483, #525, #531, #377, callback-task-loop). Prompt enforcement is fragile under model drift.
- **Structured JSON errors** — LLMs ignore plain-text advisories. Structured errors like `validate_dispatch_readiness` produces are more reliably consumed.
- **Active-child detection** — `get_child_tasks()` + filter on `trigger_type="callback"` and `status in (pending, in_progress)` is the established query pattern.

## Key Technical Decisions

- **Guard in `update_work_item_status` tool, not in a shared layer:** Engine-internal metadata writes (`try_extract_callback_metadata`) must remain unaffected. The tool is the only LLM-facing metadata write path.
- **Regex-based retry-semantic key detection:** Match metadata keys containing `retry` (case-insensitive) rather than checking only `pipeline_retry_count` exactly. The LLM could invent `retry_attempt`, `retry_count`, etc. A broader match with `/retry/i` catches these while remaining targeted.
- **Error, not silent drop:** Return a structured JSON error consistent with the `work_item_active_dispatch` pattern. The LLM receives a clear signal to stop retrying.
- **Reuse `get_child_tasks` query pattern:** Same async DB method already used by `validate_dispatch_readiness` and `has_active_callback_child`. No new DB queries needed.

## Open Questions

### Resolved During Planning

- **Should the guard block all metadata writes during active dispatch?** No — only retry-semantic keys. Legitimate metadata writes (branch, notes, pr_url) must continue working. The engine also writes metadata on the parent work item during active dispatches.
- **Where does the guard live?** In the `update_work_item_status` tool's `execute()` method, after fetching the task but before the metadata merge. This is the only LLM-facing path.
- **Race condition: callback completes milliseconds before metadata write?** The guard sees no active child and allows the write. This is correct — the callback has actually returned, so a retry metadata write at that point is legitimate.

### Deferred to Implementation

- Exact regex pattern for retry-semantic key detection — may need tuning after seeing real metadata patterns

## Implementation Units

- [x] **Unit 1: Active-dispatch retry guard in update_work_item_status**

**Goal:** Reject metadata writes containing retry-semantic keys when the work item has an active callback child task.

**Requirements:** R1, R2, R3, R5

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/tools/update_work_item_status.rs`
- Test: `crates/mika-agent/src/tools/update_work_item_status.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- After fetching the task (line 147) and before the same-status/transition-validation logic, add a new check:
  1. If `metadata_input` is provided and contains any key matching `/retry/i` (case-insensitive substring on top-level keys)
  2. Query `db.get_child_tasks(task_id)` and check for active callback children (same pattern as `validate_dispatch_readiness`)
  3. If active callback child exists, return structured JSON error with `"error": "retry_metadata_rejected_active_dispatch"`
- The check only fires when retry-semantic keys are present in the metadata — non-retry metadata writes are completely unaffected
- Use `db.get_child_tasks()` (already available on `AsyncDatabase`) — no new DB methods needed

**Patterns to follow:**
- `validate_dispatch_readiness()` in `crates/mika-agent/src/skills/executor.rs:562` — same query pattern and structured JSON error format
- Existing metadata validation in `update_work_item_status.rs:130-144` — same error handling style

**Test scenarios:**
- Happy path: metadata write with `pipeline_retry_count` succeeds when no active callback child exists
- Happy path: metadata write with non-retry keys (e.g., `claude_pilot.branch`) succeeds even when active callback child exists (R2)
- Error path: metadata write with `pipeline_retry_count` rejected when active callback child is pending
- Error path: metadata write with `pipeline_retry_count` rejected when active callback child is in_progress
- Edge case: metadata write with `retry_attempt` (variant key) rejected when active callback child exists
- Edge case: metadata write with no metadata field at all succeeds during active dispatch (status-only update)
- Edge case: metadata write with `pipeline_retry_count` succeeds when callback child is completed (no longer active)
- Integration: same-status path (line 159) also enforces the guard — metadata-only updates on an in_progress work item are caught

**Verification:**
- All existing tests pass (no regression)
- New tests cover each scenario above
- `cargo test -p mika-agent -- update_work_item_status` passes

- [x] **Unit 2: Self-dev prompt hardening**

**Goal:** Add explicit anti-hallucination guardrails to the self-dev skill prompt to reduce the probability of phantom retries.

**Requirements:** R4

**Dependencies:** None (can be done in parallel with Unit 1)

**Files:**
- Modify: `mika-skills/self-dev/system_prompt.md` (in the mika-skills repo at `/data/workspace/mika-platform/mika-skills/`)

**Approach:**
- In the "Step 4 — Wait for the completion callback" section, add a prominently formatted guardrail block:
  - "NEVER claim a pipeline has failed, mention retry counts, or write `pipeline_retry_count` to metadata unless a callback message containing 'PIPELINE FAILURE:' has been delivered in this conversation. Any retry decision before callback arrival is fabrication."
- In the "On pipeline failure" section, add a prerequisite check:
  - "PREREQUISITE: This section ONLY applies when you have received a callback result message in this conversation that contains the literal text 'PIPELINE FAILURE:'. If no such callback has been delivered, do NOT enter this section."
- Add a new Calibration Rule (Rule 10) documenting the incident and the anti-pattern

**Patterns to follow:**
- Existing Calibration Rules format (Rules 4-9) — incident citation, concrete "Wrong/Right" examples
- Existing "Do NOT" guardrails in Step 4 section

**Test scenarios:**
Test expectation: none -- prompt-only change, no code to test. Effectiveness is verified by the code guard in Unit 1 catching any prompt nonconformance.

**Verification:**
- Prompt file is valid markdown
- New guardrail text is present in the expected sections
- No existing instructions were accidentally removed

- [x] **Unit 3: Eval test for phantom retry rejection**

**Goal:** Add an agent loop eval test that exercises the full phantom retry scenario end-to-end via MockLlmProvider.

**Requirements:** R1, R5

**Dependencies:** Unit 1

**Files:**
- Create: `crates/mika-agent/tests/eval/test_phantom_retry_guard.rs`
- Modify: `crates/mika-agent/tests/eval/main.rs` (add `mod test_phantom_retry_guard;`)

**Approach:**
- Use `EvalHarness` builder with `MockLlmProvider` to set up:
  1. A work item in `in_progress` status with an active callback child task (pending)
  2. An LLM response sequence where the agent attempts to call `update_work_item_status` with `pipeline_retry_count` in metadata
  3. Verify the tool returns a structured error with `retry_metadata_rejected_active_dispatch`
- This tests the guard in the full `run_agent()` path, not just the unit tool test

**Patterns to follow:**
- Existing eval tests in `crates/mika-agent/tests/eval/` — `EvalHarness` builder pattern, `MockLlmProvider` sequence setup
- `test_webhook_queue.rs` — similar pattern of pre-creating tasks and verifying guard behavior

**Test scenarios:**
- Integration: mock LLM attempts `update_work_item_status` with `pipeline_retry_count` during active callback -> guard rejects -> LLM receives error in tool result
- Happy path: same scenario but callback child is completed -> guard allows the write

**Verification:**
- `cargo test -p mika-agent --test eval` passes
- New test file is included in the eval test runner

## System-Wide Impact

- **Interaction graph:** The guard adds one async DB query (`get_child_tasks`) per `update_work_item_status` call that includes retry-semantic metadata keys. This is the same query already used by `validate_dispatch_readiness` — no new DB load pattern.
- **Error propagation:** Structured JSON error returned to the LLM, same format as existing dispatch guards. The LLM receives a clear signal and should stop the retry attempt.
- **State lifecycle risks:** None — the guard prevents bad metadata from being written, which is the correct behavior. The same-status path (metadata-only updates) is also guarded.
- **API surface parity:** Only the `update_work_item_status` tool is affected. Engine-internal metadata writes (`try_extract_callback_metadata`) bypass this guard entirely since they go through `update_work_item_metadata` directly on the DB.
- **Unchanged invariants:** All existing metadata write behavior for non-retry keys is unchanged. All existing status transition logic is unchanged. The dispatch guard in `validate_dispatch_readiness` remains the primary defense against re-dispatch.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| LLM invents a non-matching key name (e.g., `failed_attempts`) to store retry state | Regex `/retry/i` is the initial scope; can be broadened if new evasion patterns emerge. The dispatch guard (#525) remains the backstop. |
| False positive: legitimate metadata key containing "retry" blocked | Review existing metadata patterns — no current keys match `/retry/i` except `pipeline_retry_count`, `qa_retry_count`, `ci_fix_count`. The `ci_fix_count` key does not match the regex so it's unaffected. |
| Performance: extra DB query on every metadata write with retry keys | `get_child_tasks` is a simple indexed query. Only fires when retry-semantic keys are present — the vast majority of metadata writes don't match. |

## Sources & References

- Related issue: #579
- Dispatch readiness guard: #525 — `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md`
- Fabricated action claim guard: #308 — `docs/solutions/architecture-patterns/fabricated-action-claim-guard.md`
- Completion claim guard: #483 — `docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md`
- LLM nonconformance incident: `docs/solutions/integration-issues/2026-04-15-claude-pilot-relay-over-escalation-and-llm-nonconformance.md`

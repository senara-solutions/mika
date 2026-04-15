---
title: "fix: Prevent webhook handlers from dispatching unrelated backlog work"
type: fix
status: active
date: 2026-04-15
---

# fix: Prevent webhook handlers from dispatching unrelated backlog work

## Overview

When mika-dev receives a GitHub webhook that doesn't keyword-match a specific webhook handler skill (e.g., `self-dev-webhook-qa`, `self-dev-webhook-ci`), the `self-dev` skill's always-on generic workflow takes over, causing the agent to scan the backlog via `list_work_items` and dispatch `run_claude_pilot` on unrelated tickets. This fix adds a webhook fallthrough section to the self-dev prompt, an engine-level global dispatch guard, and a per-turn dispatch cap.

## Problem Frame

The `self-dev` skill prompt references a "Webhook Entry Point — PR Review Received" section that doesn't exist — it was decomposed into the separate `self-dev-webhook-qa` skill. When a webhook arrives and no keyword-matched webhook skill activates, the always-on `self-dev` prompt's generic workflow (Step 1: understand issue, Step 2: track work item, Step 3: launch claude-pilot) fires. This caused mika-dev to dispatch claude-pilot on unrelated issues #571 and #572 when processing a `pull_request_review.submitted` webhook for PR #38.

Secondary issues: nothing prevents multiple `run_claude_pilot` calls in a single turn, and nothing enforces the CLAUDE.md invariant "never run parallel claude-pilot sessions against different repos."

## Requirements Trace

- R1. Webhook turns that don't keyword-match a specific handler must NOT scan the backlog or dispatch claude-pilot
- R2. A single agent turn must not dispatch `run_claude_pilot` more than once
- R3. The engine must reject `run_claude_pilot` dispatch when another dispatch is already active on a DIFFERENT work item (cross-repo parallel guard)
- R4. The dangling reference at line 141 of `self-dev/system_prompt.md` must be fixed
- R5. Stale `pending` work items (created but never dispatched) must be detected and surfaced

## Scope Boundaries

- Prompt changes are scoped to `mika-skills/self-dev/system_prompt.md` only — no changes to `self-dev-webhook-qa` or `self-dev-webhook-ci`
- Engine guards target `crates/mika-agent/src/skills/executor.rs` only — no changes to `validate_work_item()` or the task engine scheduler
- The `issues.assigned` and `issue_comment.created` event types are handled by the prompt-level fallthrough section, not by new dedicated skills (creating new skills is out of scope for this fix)

### Deferred to Separate Tasks

- Dedicated `self-dev-webhook-assign` and `self-dev-webhook-comment` skills with keyword triggers: future iteration when event-specific behavior is defined
- Gateway-level 429 retry mechanism for lost webhooks: separate infrastructure issue
- Verdict handler status completion (currently relies on LLM to call `update_work_item_status` after structural merge): tracked separately

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/skills/executor.rs`: `validate_dispatch_readiness()` (line 558) — existing per-work-item double-dispatch guard; `execute_long_running()` (line 666) — the dispatch entry point
- `crates/mika-agent/src/server/handlers.rs`: `handle_message()` — webhook processing, verdict handler integration
- `crates/mika-agent/src/skills/matcher.rs`: `match_skills()` — returns `MatchedSkill` with `MatchReason` (Keyword, AlwaysOn, Dependency)
- `crates/mika-agent/src/agent.rs`: `collect_required_tools()` — constraints only enforced for Keyword-matched skills
- `mika-skills/self-dev/system_prompt.md`: generic workflow (line 23), callback entry point (line 79), dangling reference (line 141)
- `mika-skills/self-dev-webhook-qa/system_prompt.md`: existing "Webhook Entry Point — PR Review Received" section with SCOPE RULE and EVENT IDENTITY CHECK patterns

### Institutional Learnings

- `docs/solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md`: "Tool boundaries are the only reliable enforcement — soft advisory strings from tools are ignored by LLMs under recovery load." Motivates engine-level guards over prompt-only fixes.
- `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md`: "Code guards over prompt instructions for any action with real resource cost." Existing pattern to extend.
- `docs/solutions/workflow-patterns/2026-04-10-self-dev-skill-decomposition.md`: The self-dev skill was decomposed into focused keyword-triggered skills. This fix operates within that architecture by adding a fallthrough section to the always-on parent, not by creating new skills.
- `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`: Four-layer defense-in-depth model. The new guards must complement, not conflict with, existing callback loop prevention.

## Key Technical Decisions

- **Prompt fallthrough section over new skills**: Adding a "Webhook Fallthrough" section to self-dev is simpler and more robust than creating new skills for every unhandled webhook type. The fallthrough section acts as a catch-all that explicitly tells the agent to stop, rather than relying on the absence of instructions.
- **Global active-dispatch guard in executor**: Checking for ANY active callback child across ALL work items (not just the target work item) enforces single-session-at-a-time structurally. This is a strict interpretation of the CLAUDE.md invariant but prevents the observed parallel dispatch failure mode.
- **Per-turn dispatch counter over per-work-item cap**: A simple per-turn counter of 1 is sufficient because: (a) sprint mode step-6-to-step-1 transitions happen across callback boundaries (separate turns), (b) pipeline retries happen in callback turns, (c) there is no legitimate case for dispatching two different work items in a single turn. The counter lives in `LongRunningContext` which is per-turn.
- **Stale pending detection in heartbeat**: The existing `get_task_health_summary()` already detects anomalies. Adding a "stale pending" check (pending > 24h with no callback child) is the natural extension.

## Open Questions

### Resolved During Planning

- **Should the global dispatch guard be in `validate_dispatch_readiness` or `execute_long_running`?** In `validate_dispatch_readiness` — it's the validation layer, and the guard is a validation concern (is it safe to dispatch?). Adding it there means all callers benefit automatically.
- **Should the per-turn cap be 1 or configurable?** Fixed at 1. There is no current use case for >1 dispatch per turn, and the observed failure mode is exactly >1 dispatch per turn.
- **What should the agent do when a webhook doesn't match any handler?** Acknowledge the event, correlate to an existing work item if possible, and stop. No backlog scan, no dispatch.

### Deferred to Implementation

- Exact wording of the prompt fallthrough section — needs to balance being directive enough for weak-adherence models while not bloating the prompt
- Whether the `dispatch_count` field on `LongRunningContext` should be an `AtomicU32` or a simple counter (depends on whether `LongRunningContext` is shared across concurrent tool calls within a turn)

## Implementation Units

- [x] **Unit 1: Add webhook fallthrough section to self-dev prompt**

**Goal:** Prevent the generic workflow from firing on webhook turns that don't keyword-match a specific handler.

**Requirements:** R1, R4

**Dependencies:** None

**Files:**
- Modify: `mika-skills/self-dev/system_prompt.md` (note: this is in the mika-skills repo, but the file is referenced from the mika worktree context)

**Approach:**
- Add a new `### Webhook Fallthrough (no keyword-matched handler)` section between the existing `### Callback Entry Point` section (line 79) and `### Block Resumption Commands` (line 145). Place it after the Step 5 removed note (line 141) so the dangling reference is replaced.
- The section must include a SCOPE RULE mirroring lines 77 and 85: "Do NOT call `list_work_items` to scan the backlog, do NOT create new work items, do NOT call `run_claude_pilot`."
- Include an EVENT IDENTITY CHECK pattern (established in self-dev-webhook-qa) that tells the agent to identify the event type and acknowledge it.
- Fix the dangling reference at line 141: replace the sentence referencing "Webhook Entry Point — PR Review Received" section with correct wording pointing to `self-dev-webhook-qa` as the handler for PR review verdicts.
- Add a calibration rule (Rule 9) encoding this incident: "When you receive a GitHub webhook event and no webhook-specific skill (self-dev-webhook-qa, self-dev-webhook-ci) activated, this turn is informational. Acknowledge the event and stop."

**Patterns to follow:**
- SCOPE RULE pattern from callback entry point (line 77, 85)
- EVENT IDENTITY CHECK pattern from `self-dev-webhook-qa/system_prompt.md` (line 7)
- Calibration rule format with incident citation (Rules 4-8 in self-dev)

**Test scenarios:**
- Test expectation: none — prompt-only change, no Rust code modified

**Verification:**
- The dangling reference at line 141 no longer points to a non-existent section
- The webhook fallthrough section has a SCOPE RULE that explicitly prohibits `list_work_items` backlog scans and `run_claude_pilot` dispatch
- The section is placed such that it applies to webhook turns where self-dev is the only active skill (AlwaysOn match, no keyword match)

- [x] **Unit 2: Add global active-dispatch guard to `validate_dispatch_readiness`**

**Goal:** Reject `run_claude_pilot` dispatch when another work item already has an active callback child, enforcing single-session-at-a-time.

**Requirements:** R3

**Dependencies:** None (can be implemented in parallel with Unit 1)

**Files:**
- Modify: `crates/mika-agent/src/skills/executor.rs`
- Test: `crates/mika-agent/src/skills/executor.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- In `validate_dispatch_readiness()`, after the per-work-item callback child check (line 601-636), add a global check: query all `trigger_type='callback'` tasks in `pending` or `in_progress` status across ALL work items. If any exist and they belong to a DIFFERENT work item than the one being dispatched, reject with a structured JSON error `global_dispatch_active`.
- The query can use the existing `db.get_tasks_by_status()` filtered to callback trigger type, or a new focused query. Prefer a new focused method `db.has_active_callback_tasks_excluding(work_item_id)` to keep the query efficient and specific.
- The error message should name the blocking work item and its active callback, so the LLM can understand why dispatch was rejected.
- This guard is additive to the existing per-work-item guard — it does not replace it.

**Patterns to follow:**
- Existing `validate_dispatch_readiness()` guard structure: fetch from DB, check condition, return structured JSON error on rejection
- Fail-closed pattern: if the DB query fails, reject dispatch (lines 627-635)

**Test scenarios:**
- Happy path: dispatch succeeds when no other work item has an active callback child
- Happy path: dispatch succeeds when the same work item has a completed (non-active) callback child
- Error path: dispatch rejected when a different work item has an active `pending` callback child — error includes `global_dispatch_active` and the blocking work item ID
- Error path: dispatch rejected when a different work item has an active `in_progress` callback child
- Edge case: dispatch succeeds when the only active callback belongs to the SAME work item being dispatched (already caught by existing per-work-item guard, but should not trigger the global guard)
- Error path: DB query failure returns `dispatch_check_failed` error (fail-closed)

**Verification:**
- `cargo test -p mika-agent` passes with new tests
- The guard fires before the callback task creation in `execute_long_running`, preventing the race

- [x] **Unit 3: Add per-turn dispatch counter to `LongRunningContext`**

**Goal:** Limit each agent turn to at most one `run_claude_pilot` dispatch, preventing the multi-dispatch pattern observed in the incident.

**Requirements:** R2

**Dependencies:** None (can be implemented in parallel with Units 1-2)

**Files:**
- Modify: `crates/mika-agent/src/skills/executor.rs`
- Modify: `crates/mika-agent/src/agent.rs` (where `LongRunningContext` is constructed)
- Test: `crates/mika-agent/src/skills/executor.rs` (inline tests)

**Approach:**
- Add a `dispatch_count: Arc<AtomicU32>` field to `LongRunningContext`. Initialize to 0 when the context is created at the start of each agent turn.
- In `execute_long_running()`, after `validate_dispatch_readiness()` succeeds, check `dispatch_count`. If > 0, reject with a structured JSON error `dispatch_limit_exceeded` explaining that only one long-running dispatch is permitted per turn.
- On successful dispatch (after callback task creation), increment the counter.
- Use `AtomicU32` with `Ordering::SeqCst` for correctness under potential concurrent tool execution within a turn. Wrap in `Arc` since `LongRunningContext` may be cloned.

**Patterns to follow:**
- Existing `LongRunningContext` field patterns (it already carries `db`, `session_id`, `trace_id`, etc.)
- Structured JSON error format from `validate_dispatch_readiness`

**Test scenarios:**
- Happy path: first dispatch in a turn succeeds, counter incremented to 1
- Error path: second dispatch in the same turn rejected with `dispatch_limit_exceeded` error
- Edge case: counter resets between turns (verified by constructing a new `LongRunningContext`)

**Verification:**
- `cargo test -p mika-agent` passes with new tests
- `cargo clippy` clean

- [x] **Unit 4: Add `has_active_callback_tasks_excluding` DB method**

**Goal:** Provide an efficient database query for the global dispatch guard in Unit 2.

**Requirements:** R3

**Dependencies:** Must be implemented before or alongside Unit 2

**Files:**
- Modify: `crates/mika-agent/src/db.rs` or `crates/mika-agent/src/db/tasks.rs` (wherever `get_child_tasks` is defined)
- Test: same file (inline tests)

**Approach:**
- Add a method `has_active_callback_tasks_excluding(&self, excluded_parent_id: &str) -> Result<Option<(String, String)>>` that returns `Some((parent_task_id, callback_task_id))` if any callback task in `pending`/`in_progress` status exists whose `parent_task_id` differs from `excluded_parent_id`. Returns `None` if no such task exists.
- SQL: `SELECT parent_task_id, id FROM tasks WHERE trigger_type = 'callback' AND status IN ('pending', 'in_progress') AND parent_task_id != ?1 LIMIT 1`
- Add corresponding async wrapper in `AsyncDatabase`.

**Patterns to follow:**
- Existing `get_child_tasks()` query pattern
- `AsyncDatabase` wrapper pattern with `with_db` closure dispatch

**Test scenarios:**
- Happy path: returns `None` when no active callback tasks exist
- Happy path: returns `None` when the only active callback belongs to the excluded parent
- Happy path: returns `Some(parent_id, task_id)` when an active callback exists for a different parent
- Edge case: returns `None` when callback tasks exist but are in terminal states (`completed`, `failed`, `cancelled`)

**Verification:**
- `cargo test -p mika-agent` passes
- Query executes efficiently (uses existing indexes on `trigger_type` and `status`)

- [x] **Unit 5: Add stale pending work item detection to task health summary**

**Goal:** Surface work items stuck in `pending` status for >24 hours with no callback child, so the heartbeat can alert the operator.

**Requirements:** R5

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/task_engine/health.rs` (or wherever `get_task_health_summary` is defined)
- Test: same file (inline tests)

**Approach:**
- Add a new anomaly type `stale_pending` to the health summary. Query: manual tasks in `pending` status where `created_at < now - 24h` and no child callback task exists.
- Include the stale item count and their labels in the health summary text, so the heartbeat prompt can instruct the agent to notify the operator or auto-cancel.
- Use the existing `get_child_tasks()` or a JOIN query to check for callback children efficiently.

**Patterns to follow:**
- Existing anomaly detection patterns in `get_task_health_summary()` (5 existing anomaly types)
- Timestamp comparison using `crate::timestamp` helpers

**Test scenarios:**
- Happy path: no stale pending items — anomaly not included in summary
- Happy path: pending item created 25 hours ago with no callback child — detected as stale
- Edge case: pending item created 25 hours ago WITH an active callback child — NOT stale (dispatch happened, just hasn't completed)
- Edge case: pending item created 23 hours ago — NOT stale (under threshold)

**Verification:**
- `cargo test -p mika-agent` passes
- Heartbeat health summary includes stale pending detection

## System-Wide Impact

- **Interaction graph:** The global dispatch guard in `validate_dispatch_readiness` affects ALL callers of `execute_long_running`, not just `run_claude_pilot`. Any future long-running handler will also be subject to the single-active-dispatch constraint. This is intentional — the CLAUDE.md invariant applies to all long-running dispatches.
- **Error propagation:** New guard rejections return structured JSON errors to the LLM, which can parse the error and understand why dispatch was rejected. The error format matches existing `validate_dispatch_readiness` errors.
- **State lifecycle risks:** The per-turn counter resets on each new agent turn (new `LongRunningContext`). No persistent state added. The global dispatch guard reads callback task status, which is already maintained by the task engine.
- **API surface parity:** No API changes. The guards are internal to the executor.
- **Unchanged invariants:** The existing per-work-item double-dispatch guard remains. The callback loop prevention (silent mode `long_running_ctx = None`) remains. Sprint mode step-6-to-step-1 transitions remain unaffected because they occur across callback boundaries (separate turns with separate `LongRunningContext` instances).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Global dispatch guard blocks legitimate retry dispatches | Retry dispatches target the SAME work item, which is excluded from the global check. Only cross-work-item parallel dispatch is blocked. |
| Per-turn cap of 1 blocks sprint mode progression | Sprint mode transitions happen in callback turns, not inline. Each callback turn gets a fresh `LongRunningContext` with counter=0. |
| Prompt fallthrough section ignored by weak-adherence models | Defense-in-depth: even if the prompt is ignored, the engine-level guards (Units 2-3) prevent the harmful action (unrelated dispatch). The prompt is the first line of defense; the engine guards are the backstop. |
| `has_active_callback_tasks_excluding` query performance on large task tables | Query uses LIMIT 1 and filters on indexed columns (`trigger_type`, `status`). Task table size is bounded by 30-day cleanup in `startup_recovery`. |

## Sources & References

- Related issue: #583
- Related code: `crates/mika-agent/src/skills/executor.rs` (validate_dispatch_readiness, execute_long_running)
- Related solutions: `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md`, `docs/solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md`
- Related skills: `mika-skills/self-dev/system_prompt.md`, `mika-skills/self-dev-webhook-qa/system_prompt.md`

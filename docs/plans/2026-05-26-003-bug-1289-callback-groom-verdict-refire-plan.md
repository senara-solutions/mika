---
title: "bug: Add groom callback verdict handling to self-dev-callback"
type: fix
status: active
date: 2026-05-26
---

# bug: Add groom callback verdict handling to self-dev-callback

## Overview

Add groom-specific callback handling to `self-dev-callback` so that when dev-groom returns `Verdict: GROOMED` (via dispatch-lib's `_iterate_groom_loop`), mika-dev automatically re-fires dev-pilot for implementation without requiring the operator to re-apply the `ready` label.

## Problem Frame

mika#996 specified that the auto-groom-on-dispatch flow should re-enter the dispatch flow after a successful groom: `ready` label → remove label → dispatch dev-groom → callback with GROOMED → re-enter dispatch → fire dev-pilot. In practice the re-entry never fires.

The `self-dev-webhook-ready-label` prompt (lines 35-41) documents the re-entry contract (steps 3d-3g), but callbacks are routed to the `self-dev-callback` skill via `SilentTrigger::Callback` → `callback_safe_skills()`. The `self-dev-callback` skill has zero groom-specific handling — it treats all callbacks as dev-pilot callbacks, looks for a PR URL (none for groom), and sets the task to `in_progress` with no further action. The parent task eventually transitions to `blocked` via the orphan reaper.

Two instances observed on 2026-05-25: mika#897 (23:05Z) and mika#1288 (23:32Z). Both required manual `ready` label re-apply to trigger dev-pilot.

## Requirements Trace

- R1. When dev-groom callback delivers with `Outcome: PLAN_GROOMED`, mika-dev must auto-fire dev-pilot without operator intervention
- R2. When dev-groom callback delivers with `PIPELINE FAILURE:` or failure, follow existing failure/retry paths
- R3. Parent task must NOT end in `blocked` after successful groom — it should transition through `in_progress` (during dev-pilot) to a terminal state
- R4. Must compose with the existing milestone/project context check (self-dev-callback lines 19-27)
- R5. Must satisfy the `callback_terminal_action` engine guard (requires both `update_task_status` AND `send_message`)

## Scope Boundaries

- Prompt-only fix — no Rust engine changes
- No changes to `self-dev-webhook-ready-label` — its documented contract (steps 3d-3g) is correct; the gap is in `self-dev-callback` not implementing it
- No changes to dispatch-lib — the callback result format and `_iterate_groom_loop` are working correctly
- No changes to the dev-groom skill itself

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/self-dev-callback/system_prompt.md` — the callback handler. Lines 7-11 define mandatory callback type detection via `check_task` + `label` field. Lines 29, 64-69, 71-76 define auto-skip, pipeline failure, and success paths respectively.
- `skills/bundled/self-dev-webhook-ready-label/system_prompt.md` — lines 35-41 define the groom callback re-entry contract (steps 3d-3g). Line 39 (step 3e) says: re-enter Ready-Label Dispatch at Step 1 on GROOMED; do NOT re-implement `create_task` + `run_claude_pilot` inline.
- `skills/bundled/_shared/dispatch-lib.sh` — line 553-561: on dev-groom success with valid plan, callback result contains `Outcome: PLAN_GROOMED — <plan_path>`. Line 545-548: on pipeline failure, result contains `PIPELINE FAILURE:` prefix and `Outcome: PIPELINE_INCOMPLETE`.
- `skills/bundled/self-dev-webhook-ready-label/system_prompt.md` — line 23: groom tasks use `reference_url` with `?phase=groom` suffix to distinguish from dispatch tasks.

### Callback Result Signals

The dispatch-lib appends structured `Outcome:` lines to the callback result:
- `Outcome: PLAN_GROOMED — <plan_path>` — groom succeeded, plan on branch, body callout written
- `Outcome: PIPELINE_INCOMPLETE — manual recovery needed.` — groom failed (prefixed by `PIPELINE FAILURE:`)
- `Outcome: UNKNOWN — inspect worktree manually.` — ambiguous result

### Callback Type Detection

The existing mandatory detection (line 7-11) uses `check_task(task_id)` and reads the `label` field:
- `long_running:run_claude_pilot...` → claude-pilot callback
- `long_running:deploy_mika...` → deploy hook callback

For groom callbacks, the label is `long_running:run_claude_pilot_groom...` (set by the executor for `run_claude_pilot_groom` tool calls). This is a distinct label prefix that can be matched.

### Re-entry Mechanism

The documented re-entry (step 3e) says to re-enter the Ready-Label Dispatch handler at Step 1. The simplest structural mechanism is to re-add the `ready` label to the issue via `run_gh("issue edit <n> --add-label ready")`. This naturally re-triggers the `self-dev-webhook-ready-label` handler, which will find the `Plan: docs/plans/` callout in the body (written by `_write_canonical_callout`) and proceed to Step 4 (create_task + run_claude_pilot for dev-pilot).

## Key Technical Decisions

- **Groom detection via label prefix:** Use `long_running:run_claude_pilot_groom` label prefix to detect groom callbacks, consistent with the existing `long_running:run_claude_pilot` and `long_running:deploy_mika` detection pattern at lines 7-11.
- **Re-entry via `ready` label re-add:** Per the documented contract in `self-dev-webhook-ready-label` step 3e, re-entry is achieved by re-adding the `ready` label. This keeps dispatch logic in one place (the ready-label handler) rather than duplicating `create_task` + `run_claude_pilot` inline.
- **Groom task completion before re-entry:** Mark the groom task `completed` before re-adding the label. This frees the groom dispatch slot and ensures the ready-label handler's subsequent `run_claude_pilot` call doesn't hit `global_dispatch_active` for the groom class.
- **`Outcome:` line as primary success signal:** Parse `Outcome: PLAN_GROOMED` from the callback result text rather than checking the issue body. The callback result is the authoritative signal from dispatch-lib; the body callout is a downstream artifact.
- **Failure routing:** Groom callbacks with `PIPELINE FAILURE:` prefix follow the existing pipeline failure path (lines 64-69) — same retry logic, same escalation threshold. Groom callbacks with non-PIPELINE_FAILURE failures follow the existing failure path (lines 78, 85-108).
- **Extract repo and issue number from task context:** Use `check_task(task_id)` to get `reference_url`, parse `senara-solutions/<repo>/issues/<n>` from it. The `?phase=groom` suffix confirms groom context.

## Open Questions

### Resolved During Planning

- **Q: Where does the groom detection block go in the prompt?** After the mandatory callback type detection (line 11) and before the auto-skip recognition (line 29). The groom path is a complete early-return handler — it does not fall through to the dev-pilot success/failure paths.
- **Q: How to extract repo/issue from the groom task?** Via `check_task(task_id)` → `reference_url` field, which contains `https://github.com/senara-solutions/<repo>/issues/<n>?phase=groom`. Parse repo and issue number from the URL.
- **Q: Does milestone/project context affect groom re-entry?** Yes. The existing milestone/project context check (lines 19-27) runs before the groom handler. If the groom task has a milestone parent, the re-entry must still satisfy the milestone advance guard. The ready-label re-add handles this correctly — the webhook handler creates a new dispatch task under the same parent when it re-fires.

### Deferred to Implementation

- **Exact wording of the groom callback detection clause:** The implementer will craft the prose to match the existing style of the callback type detection block.

## Implementation Units

- [ ] **Unit 1: Add groom callback detection and handling to self-dev-callback**

**Goal:** Add a groom-specific callback handling path that detects dev-groom callbacks, parses the outcome, and re-fires dev-pilot via ready-label re-add on GROOMED success.

**Requirements:** R1, R2, R3, R4, R5

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/self-dev-callback/system_prompt.md`

**Approach:**

Insert a new groom callback handler block after the callback type detection (line 11) and before the auto-skip recognition (line 29). The block should:

1. **Detection:** After `check_task(task_id)`, if the `label` field starts with `long_running:run_claude_pilot_groom`, this is a groom callback. Proceed with groom-specific handling instead of falling through to the dev-pilot paths.

2. **Outcome parsing:** Check the callback result text for `Outcome:` line signals:
   - `Outcome: PLAN_GROOMED` → success path (step 3 below)
   - `PIPELINE FAILURE:` prefix → route to existing pipeline failure handler (lines 64-69). The groom task's retry semantics are identical to dev-pilot pipeline failures.
   - Other failure signals → route to existing failure handler (lines 78+)

3. **GROOMED success path (R1, R3):**
   a. Extract repo and issue number from the groom task's `reference_url` (parse `senara-solutions/<repo>/issues/<n>` — strip `?phase=groom` suffix).
   b. Call `update_task_status(task_id, "completed")` to mark the groom task done. This frees the groom dispatch slot.
   c. Call `run_gh("issue edit <n> --add-label ready --repo senara-solutions/<repo>")` to re-add the `ready` label. This triggers the ready-label webhook handler, which will find the groomed body callout and dispatch dev-pilot.
   d. Call `send_message` to notify: "Auto-groom completed for {repo}#{n}. Re-added `ready` label to trigger dev-pilot dispatch."
   e. Stop the turn. The ready-label webhook handler takes over from here.

4. **Milestone/project composition (R4):** The existing milestone/project context check (lines 19-27) runs before this groom handler. When milestone context is detected AND the callback is a groom callback, the re-entry via ready-label re-add still works — the webhook handler creates a new dispatch task that inherits the milestone parent context through the existing `reference_url` dedup mechanism.

5. **Engine guard satisfaction (R5):** The GROOMED path calls both `update_task_status` (step 3b) and `send_message` (step 3d), satisfying the `callback_terminal_action` guard.

**Patterns to follow:**
- The existing callback type detection block at lines 7-11 for detection style
- The existing auto-skip handler at line 29 for early-return pattern
- The existing success handler at lines 71-76 for notification + status update pattern

**Test scenarios:**
- Happy path: Groom callback with `Outcome: PLAN_GROOMED` in result → groom task marked `completed`, `ready` label re-added, notification sent, turn stops
- Error path: Groom callback with `PIPELINE FAILURE:` prefix → routes to existing pipeline failure handler (retry or escalate)
- Error path: Groom callback with non-structured failure → routes to existing failure handler
- Integration: After `ready` label re-add, the ready-label webhook handler fires, finds `Plan: docs/plans/` in body, and dispatches dev-pilot (not testable in prompt-only change — verified by end-to-end observation per acceptance criteria)

**Verification:**
- The self-dev-callback prompt contains a groom callback detection clause matching `long_running:run_claude_pilot_groom`
- The GROOMED success path calls `update_task_status`, `run_gh` (label re-add), and `send_message`
- The pipeline failure path routes to the existing retry/escalate logic
- No existing dev-pilot callback handling is changed

## System-Wide Impact

- **Interaction graph:** Groom callback → self-dev-callback (new handler) → `run_gh` label re-add → GitHub webhook → self-dev-webhook-ready-label → dev-pilot dispatch. The chain is two webhook hops total.
- **Error propagation:** Groom failures route to the existing pipeline failure handler — no new failure paths introduced.
- **State lifecycle risks:** The groom task is marked `completed` before the label re-add. If the label re-add fails (GitHub API error), the groom task is done but dev-pilot never fires. The operator can manually re-add the `ready` label as the existing workaround. This is acceptable — the failure mode is no worse than today, and the common path succeeds.
- **Dispatch slot interaction:** Marking the groom task `completed` frees the groom dispatch slot before the ready-label handler fires. The ready-label handler creates a new implement-class dispatch task, using the implement slot. The two dispatch classes are independent (per mika#1001 per-class slot split).
- **Unchanged invariants:** The dev-pilot callback path (lines 31-112) is completely unchanged. The ready-label handler's dispatch logic (steps 1-5) is unchanged. The dispatch-lib callback result format is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| GitHub webhook delivery delay after label re-add could leave a gap | Acceptable — webhook delivery is near-instant in practice; worst case the operator re-adds label manually (same as current workaround) |
| Label re-add might race with another label operation | `run_gh` is idempotent for `--add-label`; the ready-label handler removes the label as its first action |

## Sources & References

- Related issues: mika#1289 (this bug), mika#996 (auto-groom on dispatch), mika#1271 (contract refactor)
- Evidence: mika#897 dispatch 2026-05-25 23:05Z, mika#1288 dispatch 2026-05-25 23:32Z
- Related code: `skills/bundled/self-dev-callback/system_prompt.md`, `skills/bundled/self-dev-webhook-ready-label/system_prompt.md`, `skills/bundled/_shared/dispatch-lib.sh`

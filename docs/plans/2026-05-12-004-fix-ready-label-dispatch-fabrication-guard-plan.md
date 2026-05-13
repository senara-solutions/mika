---
title: "fix: Tighten Ready-Label Dispatch guard to reject fabricated send_message escalations"
type: fix
status: active
date: 2026-05-12
---

# fix: Tighten Ready-Label Dispatch guard to reject fabricated send_message escalations

## Overview

The `webhook_ready_label_dispatch` engine guard uses an OR-shape satisfaction predicate: `run_claude_pilot` OR `send_message`. When the LLM fabricates a `check_task` pre-flight call (not in the handler spec), gets a stale failure, and then calls `send_message` with a fabricated "slot occupied" excuse, the guard is satisfied and dispatch never happens. The `send_message` goes to `chat_id=0` (NoChannel), so the operator receives no signal. Triple silent failure.

The fix tightens the guard predicate to require `run_claude_pilot` attempted, removes `send_message` from the OR shape, adds an explicit `check_task` prohibition to the skill prompt, and updates the stale correction message to reflect the post-#996 auto-groom world.

## Problem Frame

Three ready-label flips on mika#886 (and reproductions on mika-platform#98, mika#1088) all failed silently. Root cause: sonnet fabricates a `check_task` pre-flight call using a stale task ID, gets `success=0`, synthesizes a plausible "slot occupied" reason, and calls `send_message` to escalate. The guard accepts `send_message` as valid completion. The `send_message` hits `chat_id=0` (NoChannel sentinel on GitHub webhook sessions) and returns `ToolOutput::success` — the operator never receives the notification.

### Incident characterization

All three incidents exhibit the same failure pattern: fabricated `check_task` → stale failure → `send_message` escalation → guard satisfied → NoChannel silent loss.

| Incident | Trigger | Fabrication | Operator symptom |
|----------|---------|-------------|-----------------|
| mika#886 | `ready` label applied | `check_task` on stale task `254cf646...` (mika-platform#66 groom task, delivered 2.5h prior) | Ticket stuck `pending` in `mika tasks list` indefinitely; no claude-pilot session launched |
| mika#1088 | `ready` label applied | Same pattern — fabricated pre-flight on stale task ID | Same silent stall |
| mika-platform#98 | `ready` label applied | Same pattern | Same silent stall |

### Why `send_message` in the OR-shape is obsolete

The OR-shape was introduced by mika#907 to handle the pre-#996 grooming-rejection path (ungroomed ticket → `send_message` to reject). mika#996 (PR#1004, **merged 2026-05-08**, commit `9281b2c`) replaced that path with auto-groom via `run_claude_pilot` with `dev-groom` skill. Post-merge, all legitimate completion paths call `run_claude_pilot`. The `send_message`-only path is obsolete.

The only remaining `send_message`-only scenario on a ready-label webhook turn is Step 1 `run_gh` failure (label removal failed), which is self-recovering (label stays, webhook re-fires).

Additionally, the guard's correction message (agent.rs:4577-4586) is stale: it still instructs the pre-#996 behavior ("call send_message to notify the operator that grooming is required") instead of the current auto-groom path.

## Requirements Trace

- R1. Guard rejects turns where `send_message` is the only terminal call (closes the fabricated-escalation attack surface)
- R2. Guard continues to accept `run_claude_pilot` attempts (success or failure) as valid completion
- R3. Skill prompt explicitly prohibits `check_task` in Ready-Label Dispatch Steps 1-3
- R4. Correction message reflects the post-#996 auto-groom path (not the obsolete send_message rejection)
- R5. No change to legitimate auto-groom path (mika#996) or dispatch path
- R6. Regression test asserts the guard catches fabricated-escalation shape
- R7. Exhaustion handler text updated to remove stale `send_message` reference

## Scope Boundaries

- NoChannel gap for `send_message chat_id=0` — separate ticket (pre-existing architectural gap)
- Generalization of fabrication-class guards across other skill prompts — out of scope
- Migration of stalled pending tasks (mika#886, mika-platform#98, mika#1088) — manual recovery already applied

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs:4573-4587` — `webhook_ready_label_dispatch` IntentPrecondition definition
- `crates/mika-agent/src/agent.rs:4708-4712` — `ready_label_dispatch_satisfied()` predicate (the OR-shape)
- `skills/bundled/self-dev/system_prompt.md:239-292` — Ready-Label Dispatch handler section
- `crates/mika-agent/tests/eval/test_ready_label_grooming_guard.rs` — existing guard integration tests with stub tools
- `crates/mika-agent/tests/eval/test_auto_groom_dispatch.rs` — auto-groom path tests

### Institutional Learnings

- `docs/solutions/workflow-issues/ready-label-dispatch-requires-grooming-marker-2026-04-30.md` — documents the OR-shape as "intentionally over-broad"; mika#1089 proves this was not acceptable
- `docs/solutions/agent-quality/2026-04-09-fabricated-cantool-denial-citations.md` — LLM fabrication pattern: "prefer structural positive allowlists in code over prompt-level rules; LLMs rationalize around negative rules"
- `docs/solutions/logic-errors/groom-closing-comment-self-triggers-dispatch-2026-05-11.md` — another ready-label guard bypass via comment-event path; reinforces that engine-level guards must be tight

### External References

- None needed — this is an internal engine guard fix with clear patterns to follow

## Pinned Source (Phase 0 Pin)

### Guard predicate (`crates/mika-agent/src/agent.rs:4708-4712`)

```rust
fn ready_label_dispatch_satisfied(summaries: &[ToolCallSummary]) -> bool {
    summaries
        .iter()
        .any(|s| s.name == "run_claude_pilot" || s.name == "send_message")
}
```

The `|| s.name == "send_message"` arm is the attack surface. Remove it.

### Correction message (`crates/mika-agent/src/agent.rs:4577-4586`)

```rust
correction_message: "[Your response was rejected. The `ready` label has been \
     removed but you completed neither dispatch nor grooming-rejection \
     notification. The Ready-Label Dispatch handler requires you to: \
     (1) run_gh `issue view <n> --json title,body --repo <repo>` to fetch \
     the issue, (2) check the issue body for the grooming marker \
     `> - **Plan:**`. If the marker is PRESENT: call create_task then \
     run_claude_pilot with prompt=\"<repo>#<n>\" and task_id=<UUID>. \
     If the marker is ABSENT: call send_message to notify the operator \
     that grooming is required before dispatch. Do not end this turn \
     until you have either dispatched or notified.]",
```

Stale post-#996: "If the marker is ABSENT: call send_message to notify the operator" should say "call create_task then run_claude_pilot with skill=dev-groom" (auto-groom path). The "grooming-rejection notification" framing is obsolete.

### Exhaustion handler (`crates/mika-agent/src/agent.rs:1532-1561`)

```rust
// #846 + #907 — operator notification when the ready-label
// dispatch guard fired but neither run_claude_pilot nor
// send_message was called after the retry.
if intent_guard_retries.contains("webhook_ready_label_dispatch")
    && !ready_label_dispatch_satisfied(&all_tool_summaries)
{
    let location = parse_ready_label_location(&user_input_text)
        .unwrap_or_else(|| "<unknown>".to_string());
    error!(/* ... */);
    if let Some(ref sender) = tool_ctx.message_sender {
        let notification = format!(
            "Ready-label dispatch stalled on {location}: the `ready` \
             label was removed but neither dispatch (run_claude_pilot) \
             nor grooming-rejection notification (send_message) \
             completed. Investigate trace_id {} in \
             /var/log/mika/server.log. To retry, re-add the `ready` \
             label.",
            tool_ctx.trace_id
        );
        let _ = sender.send(&notification).await;
    }
}
```

The exhaustion handler text also references `send_message` as a legitimate path ("nor grooming-rejection notification (send_message)"). This must be updated to remove the `send_message` reference. After the fix, the notification should say "dispatch (run_claude_pilot) did not complete" without mentioning `send_message`. The comment above the block (#846 + #907) also needs the `send_message` reference removed.

### Skill prompt Steps 1-3 tool instructions (`skills/bundled/self-dev/system_prompt.md:245-266`)

Steps 1-3 use a **per-step tool prescription** pattern (not a general allowlist):
- **Step 1:** "call `run_gh`" — specific tool named
- **Step 2:** "call `run_gh`" — specific tool named
- **Step 3:** "scan the fetched issue body" (no tool call) → branch to Step 4 (groomed) or auto-groom via `create_task` + `run_claude_pilot`

The prohibition approach should be a **denylist addition** matching the existing per-step prescription style: "In Steps 1-3, do NOT call `check_task`." This is consistent with the existing prohibitions in the same handler (e.g., "Do NOT call `create_task` or `run_claude_pilot`" at line 249, "Do not call `send_message` to notify the operator" at line 266). An allowlist would be a different pattern than what the prompt uses.

## Key Technical Decisions

- **Tighten predicate to `run_claude_pilot`-only (not Option A or B from ticket):** The ticket proposed Option A (structured prefix on `send_message` text — fragile, depends on LLM emitting prefix correctly) or Option B (reject `send_message`-only when grooming marker absent — requires guard to know issue body content, which `ToolCallSummary` doesn't carry). The simpler fix: require `run_claude_pilot` attempted. Post-#996, all legitimate completion paths call `run_claude_pilot` (dispatch via dev-pilot, auto-groom via dev-groom, or terminal dispatch error after attempting). The `send_message`-only path is obsolete.

- **Step 1 `run_gh` failure edge case is acceptable:** If label removal fails, the guard rejects the turn (no `run_claude_pilot`), re-prompts once. On re-prompt, the agent retries the full sequence. If still failing, guard exhausts and logs — observable unlike the current NoChannel path. The label remains present, so the webhook will re-fire independently.

- **Prompt prohibition is defense-in-depth, not primary fix:** Per institutional learnings, "prefer structural positive allowlists in code over prompt-level rules." The engine guard tightening is the primary fix; the prompt prohibition is a belt-and-suspenders reinforcement.

## Open Questions

### Resolved During Planning

- **Should we use Option A, B, or a simpler approach?** Resolved: simpler approach — remove `send_message` from OR shape entirely. Post-#996, the `send_message`-only path is obsolete for ready-label turns.
- **Does tightening break any legitimate path?** Resolved: No. All legitimate paths call `run_claude_pilot` (dispatch, auto-groom, terminal error after attempt). The only `send_message`-only path (Step 1 `run_gh` failure) is self-recovering via webhook re-fire.
- **Should the correction_message also be updated?** Resolved: Yes — it references the pre-#996 "send_message to notify operator" path which is obsolete.
- **Should the exhaustion handler text also change?** Resolved: Yes — it references "grooming-rejection notification (send_message)" which is obsolete. See Pinned Source § Exhaustion handler.
- **Is mika#996 merged?** Resolved: Yes — PR#1004 merged 2026-05-08, commit `9281b2c`. The auto-groom path is deployed. Removing `send_message` from the OR-shape does not break any legitimate path.

### Deferred to Implementation

- Exact wording of the correction message and exhaustion notification — directional intent is clear (instruct auto-groom for ungroomed, dispatch for groomed, remove all `send_message` references), final prose is implementation-time

## Implementation Units

- [ ] **Unit 1: Tighten engine guard predicate, correction message, and exhaustion handler**

  **Goal:** Remove `send_message` from the `ready_label_dispatch_satisfied` predicate so only `run_claude_pilot` attempts satisfy the guard. Update the correction message and exhaustion handler to reflect the post-#996 auto-groom world.

  **Requirements:** R1, R2, R4, R7

  **Dependencies:** None. Prerequisite mika#996 (PR#1004) is confirmed merged (2026-05-08, commit `9281b2c`).

  **Files:**
  - Modify: `crates/mika-agent/src/agent.rs` (three sites: predicate ~4708-4712, correction message ~4577-4586, exhaustion handler ~1532-1561)

  **Approach:**
  - **Predicate:** Change `ready_label_dispatch_satisfied` to check only `s.name == "run_claude_pilot"` (remove the `|| s.name == "send_message"` arm). See Pinned Source § Guard predicate.
  - **Correction message:** Remove "grooming-rejection notification" language. Replace the "If the marker is ABSENT: call send_message" instruction with "call create_task then run_claude_pilot with skill=dev-groom". The correction should steer the agent toward retrying the full Steps 1-5 sequence (including `run_gh` retry on Step 1 failure), not toward `send_message` reporting. See Pinned Source § Correction message.
  - **Exhaustion handler:** Update notification text to remove "(send_message)" parenthetical and "nor grooming-rejection notification" language. Post-fix, the exhaustion message should say "dispatch (run_claude_pilot) did not complete" without mentioning `send_message`. Update the accompanying comment (#846 + #907 → add #1089). See Pinned Source § Exhaustion handler.
  - **Doc comment:** Update the doc comment on `ready_label_dispatch_satisfied` to reflect the predicate change, reference #1089, and explain why `send_message` was removed (post-#996 auto-groom made it obsolete; over-broad match enabled fabrication attacks)

  **Patterns to follow:**
  - `webhook_no_unauthorized_dispatch_satisfied` (agent.rs:4727-4731) — single-tool-name predicate pattern
  - Existing correction message style in the same `INTENT_GUARDS` array

  **Test scenarios:**
  - Happy path: turn with successful `run_claude_pilot` call → predicate returns true
  - Happy path: turn with failed `run_claude_pilot` call (terminal error) → predicate returns true (attempts count)
  - Edge case: turn with only `send_message` call → predicate returns false (the fix)
  - Edge case: turn with both `run_claude_pilot` and `send_message` → predicate returns true
  - Edge case: turn with no tool calls → predicate returns false

  **Verification:**
  - `cargo test -p mika-agent` passes
  - Unit tests for the predicate function cover all five scenarios above

- [ ] **Unit 2: Add `check_task` prohibition to skill prompt**

  **Goal:** Add an explicit prohibition against calling `check_task` in Steps 1-3 of the Ready-Label Dispatch handler.

  **Requirements:** R3

  **Dependencies:** None (can be done in parallel with Unit 1)

  **Files:**
  - Modify: `skills/bundled/self-dev/system_prompt.md` (Ready-Label Dispatch section, ~lines 239-292)

  **Approach:**
  - Add a prohibition note after the Step 3 grooming pre-flight description, before Step 4. The prohibition should be structural ("In Steps 1-3, do NOT call `check_task`") with a brief rationale ("The engine enforces per-class dispatch slot availability via `run_claude_pilot`'s deferred-status return path; pre-flight slot-checks are not in this handler's contract").
  - Place it as a callout block for visibility, not buried in prose

  **Patterns to follow:**
  - Existing prohibition style in the same handler: "Do NOT call `create_task` or `run_claude_pilot`" (line 249), "Do not call `send_message` to notify the operator" (line 266)

  **Test scenarios:**
  - Test expectation: none — this is a prompt-only change; behavioral verification is via Unit 3's integration test

  **Verification:**
  - The prohibition text is present in the Ready-Label Dispatch section
  - No other sections of the prompt are affected

- [ ] **Unit 3: Integration test for fabricated-escalation rejection**

  **Goal:** Add an integration test proving the guard rejects a turn where the LLM calls only `send_message` (fabricated escalation) without calling `run_claude_pilot`.

  **Requirements:** R6

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/tests/eval/test_ready_label_grooming_guard.rs` (add new test case)

  **Approach:**
  - Add a test case using the existing `EvalHarness` + `MockLlmProvider` pattern from the same file
  - Mock LLM sequence: (1) call `run_gh` to remove label (success), (2) call `check_task` (success=0), (3) call `send_message` (success), (4) EndTurn — guard should reject, (5) on re-prompt, call `run_claude_pilot` (success), (6) EndTurn — guard should accept
  - Reuse existing stub tools (`StubSendMessage`, `StubRunGh`, `StubRunClaudePilot`) from the test file
  - Add a `StubCheckTask` stub that returns failure

  **Patterns to follow:**
  - Existing test `grooming_rejection_via_send_message_satisfies_guard` in the same file — this is the test that verified the OLD behavior; the new test proves the opposite
  - `test_auto_groom_dispatch.rs` for auto-groom path test patterns

  **Test scenarios:**
  - Integration: Ready-label webhook turn with only `send_message` (no `run_claude_pilot`) → guard rejects, re-prompts agent
  - Integration: Ready-label webhook turn with `run_claude_pilot` after re-prompt → guard accepts
  - Edge case: verify existing `run_claude_pilot`-only path still works (may already be covered by existing tests)

  **Verification:**
  - `cargo test -p mika-agent --test eval test_ready_label` passes
  - The new test explicitly asserts guard rejection on `send_message`-only turns

- [ ] **Unit 4: Update existing test expectations and solution doc**

  **Goal:** Update any existing tests that assert `send_message`-only satisfies the guard (they now should assert rejection). Update the solution doc to reflect the narrowed predicate.

  **Requirements:** R5 (verify no regression)

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/tests/eval/test_ready_label_grooming_guard.rs` (update existing test expectations)
  - Modify: `docs/solutions/workflow-issues/ready-label-dispatch-requires-grooming-marker-2026-04-30.md` (update guidance)

  **Approach:**
  - Find existing tests that assert `send_message`-only satisfies the guard. These tests verified the #907 OR-shape behavior — they should now assert the OPPOSITE (guard rejects `send_message`-only)
  - Update the solution doc: (a) add a "Superseded by #1089" section noting the OR-shape was narrowed, (b) update the "After" code example to show `run_claude_pilot`-only, (c) update the "intentionally over-broad" section to note it was narrowed after fabrication incidents
  - Verify all auto-groom path tests in `test_auto_groom_dispatch.rs` still pass without modification

  **Patterns to follow:**
  - Solution doc update pattern: add supersession note rather than rewriting history

  **Test scenarios:**
  - Regression: existing `run_claude_pilot`-only test continues to pass (guard accepts)
  - Regression: auto-groom path tests continue to pass (auto-groom calls `run_claude_pilot` with dev-groom)
  - Updated: `send_message`-only test now asserts guard rejection

  **Verification:**
  - `cargo test -p mika-agent --test eval` passes (full eval suite)
  - Solution doc reflects the narrowed predicate with #1089 context

## System-Wide Impact

- **Interaction graph:** The `webhook_ready_label_dispatch` guard interacts with: (a) `webhook_no_unauthorized_dispatch` guard (mutually exclusive triggers — not affected), (b) `webhook_zero_tools` guard (evaluated after — not affected since `run_claude_pilot` counts as a tool call), (c) the `callback_terminal_action` guard (different trigger — callback turns, not affected), (d) the self-dev skill prompt (defense-in-depth alignment)
- **Error propagation:** Guard rejection → re-prompt with correction message → agent retries. Guard exhaustion → `error!` log + operator notification via `message_sender.send()`. The exhaustion handler's notification text is updated (R7) but its control flow is unchanged.
- **State lifecycle risks:** None — the guard is stateless per-turn. The `intent_guard_retries` set tracks retry state within a single turn.
- **API surface parity:** The correction message is the only surface that communicates the guard's expectations to the LLM. Updating it is essential for the fix to work end-to-end.
- **Unchanged invariants:** (a) `webhook_no_unauthorized_dispatch` guard is untouched. (b) Auto-groom path via `run_claude_pilot(dev-groom)` continues to satisfy the guard. (c) Terminal dispatch errors after `run_claude_pilot` attempt continue to satisfy the guard (attempts count, not just successes). (d) The `NoChannel` gap on `send_message chat_id=0` is a pre-existing issue not addressed here.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Step 1 `run_gh` failure causes guard rejection loop | Acceptable: label stays present → webhook re-fires. Guard gives one retry. Guard exhaustion logs observably. |
| Existing tests assert `send_message`-only satisfies guard | Unit 4 explicitly updates these expectations |
| LLM ignores prompt prohibition on `check_task` | Engine guard is the primary fix; prompt prohibition is defense-in-depth only |

## Sources & References

- Related issues: mika#1089 (this ticket), mika#907 (introduced OR-shape), mika#996 (auto-groom), mika#841 (ready label), mika#846 (guard), mika#886 (incident)
- Solution doc: `docs/solutions/workflow-issues/ready-label-dispatch-requires-grooming-marker-2026-04-30.md`
- Institutional learning: `docs/solutions/agent-quality/2026-04-09-fabricated-cantool-denial-citations.md`
- Agent.rs guard: `crates/mika-agent/src/agent.rs:4573-4587, 4708-4712`
- Self-dev prompt: `skills/bundled/self-dev/system_prompt.md:239-292`

---
title: "fix: Require grooming marker before run_claude_pilot dispatch on ready label"
type: fix
status: active
date: 2026-04-30
---

# fix: Require grooming marker before run_claude_pilot dispatch on ready label

## Overview

Add a two-layer grooming check that prevents `run_claude_pilot` dispatch when a `ready`-labeled issue lacks the `> - **Plan:**` callout in its body. Layer 1 (skill prompt) is the primary gate; Layer 2 (engine guard) is the structural backstop.

## Problem Frame

mika#901 was dispatched to claude-pilot with no grooming. The operator applied the `ready` label to an ungroomed ticket, mika-dev launched claude-pilot, and the session edited files without architect input. The `ready` label gate (mika#841) enforces consent but not grooming. Vincent's stated rule: `ready` should require grooming AND consent. Each slip costs a wasted claude-pilot session (~$1.70, ~5 min) and risks off-target commits.

Related: mika#841 (introduced `ready`), mika#846 (engine guard for `webhook_ready_label_dispatch`), mika#901 (the incident).

## Requirements Trace

- R1. Skill-prompt PRE-FLIGHT check rejects dispatch when issue body lacks `> - **Plan:**` callout. Operator notified via `send_message`. No `create_task` or `run_claude_pilot` calls in that turn.
- R2. Engine guard modification allows the grooming-rejection path (operator notification without dispatch) as a valid completion shape.
- R3. Exhaustion handler produces accurate notification text for both dispatch and rejection paths.
- R4. Correction message guides the LLM toward the correct behavior on both paths (dispatch OR reject).
- R5. Existing dispatches (groomed issues with the callout) continue to fire — no regression.
- R6. Behavioral tests cover: ungroomed rejection at both layers, groomed happy path, guard satisfaction predicates.

## Scope Boundaries

- The grooming marker is `> - **Plan:**` prefix presence in the issue body. Plan file existence is NOT validated here — that check belongs to `/mika` (the dispatch handler).
- Not changing the `ready` label semantics in `.github/labels.yml`.
- Not auto-running `/mika-groom-ticket` on behalf of the operator.
- Not addressing the `NoChannel` gap (GitHub webhook sessions with `chat_id=0` swallow `send_message`) — pre-existing architectural gap affecting all notification-as-rejection paths, not new to this change.
- Not adding a `groomed` label — the body callout is the source of truth (matches `dev-groom`'s output contract).

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/self-dev/system_prompt.md` lines 238-264 — Ready-Label Dispatch handler: remove label → fetch issue → create_task → run_claude_pilot
- `crates/mika-agent/src/agent.rs` lines 4232-4297 — `INTENT_GUARDS` registry, four entries
- `crates/mika-agent/src/agent.rs` lines 4319-4333 — `ready_label_dispatch_trigger()` and `ready_label_dispatch_satisfied()`
- `crates/mika-agent/src/agent.rs` lines 1470-1496 — exhaustion handler notification for the ready-label guard
- `crates/mika-agent/src/agent.rs` lines 1261-1302 — guard evaluation loop
- `crates/mika-agent/tests/eval/test_intent_precondition_guard.rs` — eval test pattern: stub tools, `EvalHarness`, `MockLlmProvider`
- `skills/bundled/dev-groom/system_prompt.md` line 68 — produces the `> - **Plan:**` callout
- `callback_terminal_action` guard (lines 4291-4296) — AND-shape precedent requiring both `update_task_status` AND `send_message`

### Institutional Learnings

- **Engine guards vs prompt rules** (`docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`): "Don't dispatch without grooming" is against-gradient behavior — the LLM's default is to dispatch when triggered. Engine-level enforcement required.
- **Intent-precondition registry** (`docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md`): Adding new guard entries is a data-declaration task in `INTENT_GUARDS`.
- **Ready-label dispatch regression** (`docs/solutions/workflow-issues/ready-label-dispatch-handler-regression-2026-04-27.md`): Prose-routing is structurally weak — inline steps + engine guard is the proven pattern.
- **Grooming-branch callout** (`docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md`): Issue body is the canonical contract surface for grooming state.

## Key Technical Decisions

- **Modify existing guard, don't add a new one:** The `webhook_ready_label_dispatch` guard already triggers on the same marker. Its `satisfied` predicate should be widened to accept the grooming-rejection path rather than adding a second guard with the same trigger — two guards on the same trigger would be confusing and only one fires per evaluation.
- **`send_message` as valid satisfaction (OR-shape):** Accept `run_claude_pilot` OR `send_message` as guard satisfaction. This is intentionally over-broad — `send_message` for any reason satisfies the guard, not just grooming rejection. This is acceptable because: (a) the prompt is the primary control layer, the guard is the backstop; (b) the trigger is very specific (only ready-label webhook events); (c) making `send_message` more specific would require inspecting truncated `input_summary` which is fragile. The `callback_terminal_action` guard uses the same `send_message` matching without content discrimination.
- **Grooming check is prefix-only:** Match `> - **Plan:**` prefix in the issue body. No validation of file path, plan existence, or grooming-pass count. The callout's presence is sufficient evidence that the grooming pipeline ran.
- **Correction message presents both paths:** After guard rejection, the LLM must be guided toward EITHER dispatch (if groomed) or notification (if ungroomed) — a dispatch-only correction message would override the grooming check.

## Open Questions

### Resolved During Planning

- **How to distinguish grooming-rejection `send_message` from unrelated `send_message`?** Accept the over-broad predicate. The prompt is the primary gate; the engine guard is backstop. Attempting content-based discrimination on truncated `input_summary` is fragile and adds complexity for marginal gain.
- **Should the `run_gh issue view` failure path (Step 1 of existing handler) explicitly require `send_message`?** No — the existing prompt says "stop" on failure. The updated guard (accepting `send_message`) means that if the LLM does send a notification on fetch failure, the guard is satisfied. If it doesn't, the guard fires once and the correction message instructs it. No additional prompt change needed for this path.

### Deferred to Implementation

- Exact `send_message` notification text (specific wording to be determined during implementation, must include issue reference, reason, and recovery instruction).

## Implementation Units

- [ ] **Unit 1: Widen the engine guard's `satisfied` predicate and update correction message**

**Goal:** Modify `ready_label_dispatch_satisfied()` to accept `run_claude_pilot` OR `send_message` as valid completion. Update the correction message to guide the LLM toward both paths.

**Requirements:** R2, R4

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`
- Test: `crates/mika-agent/src/agent.rs` (inline unit tests)

**Approach:**
- Change `ready_label_dispatch_satisfied()` to return `true` when either `run_claude_pilot` or `send_message` is found in summaries.
- Update the `correction_message` in the `webhook_ready_label_dispatch` `IntentPrecondition` entry to present both valid paths: "Verify the issue body contains the grooming marker `> - **Plan:**`. If present, proceed with dispatch (create_task, run_claude_pilot). If absent, call send_message to notify the operator that grooming is required."
- Update the `INTENT_GUARDS` entry comment to reflect the OR-shape change and reference #907.

**Patterns to follow:**
- `callback_terminal_action_satisfied()` — uses `send_message` matching without content discrimination (same OR-accepting pattern)
- Existing `ready_label_dispatch_satisfied()` — simple predicate on `ToolCallSummary`

**Test scenarios:**
- Happy path: `run_claude_pilot` present in summaries → `satisfied` returns `true`
- Happy path: `send_message` present in summaries → `satisfied` returns `true`
- Happy path: both `run_claude_pilot` and `send_message` present → `satisfied` returns `true`
- Edge case: only `run_gh` present (no dispatch, no notification) → `satisfied` returns `false`
- Edge case: empty summaries → `satisfied` returns `false`

**Verification:**
- Existing unit tests for `ready_label_dispatch_trigger()` still pass
- New unit tests for the OR-shape predicate pass
- `cargo test -p mika-agent` passes

- [ ] **Unit 2: Update the exhaustion handler notification text**

**Goal:** Make the exhaustion handler notification accurate for both the dispatch and grooming-rejection paths.

**Requirements:** R3

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`

**Approach:**
- Update the exhaustion handler block (lines 1470-1496) to use the widened `ready_label_dispatch_satisfied()` predicate (it already does — no code change needed for the condition).
- Update the notification message text: change "run_claude_pilot was never called" to "neither dispatch nor grooming-rejection notification completed" so it's accurate for both branches.
- The exhaustion handler fires only when the guard was retried AND the widened predicate is still unsatisfied — meaning neither `run_claude_pilot` nor `send_message` was called. This is the correct behavior for both paths.

**Patterns to follow:**
- Existing exhaustion handler structure at lines 1470-1496

**Test scenarios:**
- Happy path: guard fires, retry produces `send_message` → exhaustion handler does NOT fire (predicate satisfied)
- Happy path: guard fires, retry produces `run_claude_pilot` → exhaustion handler does NOT fire (predicate satisfied)
- Edge case: guard fires, retry produces neither → exhaustion handler fires with updated message text

**Verification:**
- Exhaustion handler notification text mentions both dispatch and notification paths
- `cargo test -p mika-agent` passes

- [ ] **Unit 3: Add grooming-marker PRE-FLIGHT check to self-dev skill prompt**

**Goal:** Insert a grooming check between Steps 2 and 3 of the Ready-Label Dispatch handler. If the issue body lacks `> - **Plan:**`, the agent should notify the operator and stop — no `create_task`, no `run_claude_pilot`.

**Requirements:** R1

**Dependencies:** Unit 1 (the engine guard must already accept `send_message` as valid satisfaction)

**Files:**
- Modify: `skills/bundled/self-dev/system_prompt.md`

**Approach:**
- Insert a new numbered step between current Steps 2 and 3 (renumber subsequent steps).
- After fetching the issue body via `run_gh("issue view ...")`, instruct the agent to scan the body for `> - **Plan:**`.
- If the callout is NOT found: call `send_message` notifying the operator that the issue lacks a grooming marker, specifying the issue reference, the reason, and the recovery instruction (run `/mika-groom-ticket`, then re-add `ready`). Do NOT proceed to `create_task` or `run_claude_pilot`. EndTurn.
- If the callout IS found: proceed to `create_task` (renumbered Step 4) and `run_claude_pilot` (renumbered Step 5).
- Add a note in the engine-enforcement callout (the blockquote at line 242) that the guard now accepts EITHER `run_claude_pilot` OR `send_message` for grooming rejection.

**Patterns to follow:**
- Existing Step 1 failure handling ("On `run_gh` failure: Do NOT call `create_task` or `run_claude_pilot`. Send the operator a `send_message`...")
- The imperative, inline-step style of the current handler (not cross-section routing)

**Test scenarios:**
- Test expectation: none — prompt changes are not unit-testable. Behavioral coverage provided by Unit 4 eval tests.

**Verification:**
- The grooming check is between the issue-fetch step and the create_task step
- The check instruction is imperative and inline (not routing to another section)
- The `send_message` template includes issue reference, reason, and recovery instruction
- The handler's subsequent steps are correctly renumbered

- [ ] **Unit 4: Add eval integration tests for the grooming-marker gate**

**Goal:** Add behavioral tests verifying that the ready-label dispatch gate correctly handles groomed and ungroomed issues at both layers.

**Requirements:** R5, R6

**Dependencies:** Units 1, 2, 3

**Files:**
- Create: `crates/mika-agent/tests/eval/test_ready_label_grooming_guard.rs`
- Modify: `crates/mika-agent/tests/eval/mod.rs` (register new test module)

**Approach:**
- Create stub tools: `StubRunClaudePilot`, `StubSendMessage`, `StubRunGh`, `StubCreateTask` — each records the call in summaries.
- Test 1 (ungroomed rejection): Mock LLM responds with `send_message` (no `run_claude_pilot`). Feed a ready-label webhook message. Verify the guard is satisfied (turn completes without correction re-prompt).
- Test 2 (groomed happy path): Mock LLM responds with `run_gh`, `create_task`, `run_claude_pilot`. Feed a ready-label webhook message. Verify the guard is satisfied.
- Test 3 (guard fires on zero relevant tools): Mock LLM responds with text only (or only `run_gh`). Feed a ready-label webhook message. Verify the guard fires (correction message injected, step count > 1).
- Test 4 (guard single-retry exhaustion): Mock LLM responds with text-only twice. Verify the guard fires once and the turn eventually completes (exhaustion path).

**Patterns to follow:**
- `test_intent_precondition_guard.rs` — stub tools, `EvalHarness::builder().responses().tools().build()`, `assert_exact_steps`, `assert_has_output`, `assert_output_contains`
- `test_webhook_zero_tools_guard.rs` — webhook-specific guard tests

**Test scenarios:**
- Happy path: `send_message`-only turn on ready-label webhook → guard satisfied, 2 steps (tool call + final text)
- Happy path: `run_claude_pilot` in turn → guard satisfied
- Error path: text-only response on ready-label webhook → guard fires, correction re-prompt injected
- Edge case: guard fires once, retry still has no qualifying tool → exhaustion path, turn completes

**Verification:**
- All four tests pass
- `cargo test -p mika-agent --test eval` passes
- No regressions in existing intent-guard tests

## System-Wide Impact

- **Interaction graph:** The `webhook_ready_label_dispatch` guard's `satisfied` predicate is widened. The exhaustion handler at lines 1470-1496 already uses this predicate — no additional wiring needed. The `webhook_zero_tools` guard (evaluated after) is unaffected because `send_message` is a successful tool call.
- **Error propagation:** Guard rejection → correction message → retry. If retry also fails → exhaustion handler fires operator notification via `message_sender.send()`. Same two-step escalation as current behavior.
- **State lifecycle risks:** Label removal happens first (Step 1, unchanged). If the grooming check rejects, the label is already removed — the operator must re-add it after grooming. This is consistent with the existing "label removal first" design.
- **Unchanged invariants:** The `ready` label semantics in `.github/labels.yml` are unchanged. The `dev-groom` output contract (producing the `> - **Plan:**` callout) is unchanged. The `dispatch-lib.sh` shared handler is unchanged. The `validate_dispatch_readiness()` checks in the executor are unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Over-broad `send_message` satisfaction — an unrelated `send_message` call could satisfy the guard even if grooming was never checked | Acceptable: prompt is primary gate, guard is backstop. Trigger is very specific (ready-label events only). Same pattern as `callback_terminal_action` guard. |
| `send_message` hits `NoChannel` on GitHub webhook sessions — operator never receives notification | Pre-existing gap. Defer to separate issue. The `run_gh` failure path at Step 1 has the same problem today. |
| Grooming marker presence doesn't guarantee plan quality or existence | Out of scope — plan file validation is `/mika`'s responsibility, not the dispatch gate's. |

## Sources & References

- Related issues: #841 (introduced `ready`), #846 (engine guard), #901 (the incident), #907 (this fix)
- Related code: `crates/mika-agent/src/agent.rs` (guard registry), `skills/bundled/self-dev/system_prompt.md` (dispatch handler)
- Related learnings: `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`, `docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md`

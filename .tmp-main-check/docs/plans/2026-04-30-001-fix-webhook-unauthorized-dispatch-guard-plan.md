---
title: "fix: Add webhook_no_unauthorized_dispatch intent guard (#910)"
type: fix
status: active
date: 2026-04-30
---

# fix: Add webhook_no_unauthorized_dispatch intent guard (#910)

## Overview

Add a new `IntentPrecondition` entry to the `INTENT_GUARDS` registry that rejects `run_claude_pilot` calls on `[GitHub]` webhook turns that are NOT `ready` label events. This is the engine-level fix for a three-incident recurring bug where mika-dev's LLM dispatches autonomous work from comment events containing dispatch-like phrases (e.g., `implement mika issue#906`), despite prompt-level rules prohibiting it.

## Problem Frame

The `ready` label is the positive-consent signal for autonomous dispatch (mika#841). However, the prompt-level source-check rule that enforces this has failed three times in the same failure class:

| # | Date | Trigger | Result |
|---|---|---|---|
| #798 | original | comment containing dispatch phrase | unauthorized dispatch |
| #838 | 2026-04-26 | comment containing `implement mika issue#838` | unauthorized dispatch |
| #910 | 2026-04-30 | comment containing `implement mika issue#906` | unauthorized dispatch |

The LLM pattern-matches `implement <repo> issue#<n>` as a dispatch instruction because that exact phrasing appears in the Layer 1 routing table. Prompt-level rules are "against-gradient" for this behavior (per `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`) and drift under load. Engine-level `IntentPrecondition` guards are deterministic.

## Requirements Trace

- R1. New `IntentPrecondition` entry `webhook_no_unauthorized_dispatch` in `INTENT_GUARDS`
- R2. Predicate uses message-shape (`msg.starts_with("[GitHub]") && !msg.starts_with(READY_LABEL_DISPATCH_MARKER)`)
- R3. Reuses `READY_LABEL_DISPATCH_MARKER` const (consistency with `webhook_ready_label_dispatch`)
- R4. Guard rejects successful `run_claude_pilot` in tool summaries for non-ready webhook turns
- R5. Correction message cites mika#841 Layer 1 source-check
- R6. Existing guards (`webhook_ready_label_dispatch`, `webhook_zero_tools`, `resume_reconcile`, `callback_terminal_action`) unregressed
- R7. Unit tests for trigger and satisfied predicates
- R8. Integration tests via EvalHarness

## Scope Boundaries

- No changes to prompt-level rules (prompt stays as defense-in-depth)
- No per-author allowlist on the classifier
- No encryption or rate-limiting on the webhook ingress path
- No retroactive scrubbing of historical unauthorized dispatch records

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs:4217-4226` — `IntentPrecondition` struct definition
- `crates/mika-agent/src/agent.rs:4232-4297` — `INTENT_GUARDS` const array (4 entries)
- `crates/mika-agent/src/agent.rs:4315` — `READY_LABEL_DISPATCH_MARKER` const
- `crates/mika-agent/src/agent.rs:4319-4333` — `ready_label_dispatch_trigger` and `ready_label_dispatch_satisfied` helper functions
- `crates/mika-agent/src/agent.rs:1261-1301` — Guard evaluation loop in `run_loop`
- `crates/mika-agent/src/agent.rs:7408-7588` — Unit tests for existing guards
- `crates/mika-agent/tests/eval/test_webhook_zero_tools_guard.rs` — Integration test pattern for webhook guards
- `crates/mika-agent/tests/eval/test_intent_precondition_guard.rs` — Integration test pattern for registry guards

### Institutional Learnings

- `docs/solutions/workflow-issues/comment-event-fires-autonomous-dispatch-2026-04-25.md` — Documents the failure class; positive-consent model
- `docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md` — Adding a new guard is a data-declaration task
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — Against-gradient behaviors require engine guards

## Key Technical Decisions

- **Guard position: between `webhook_ready_label_dispatch` (index 0) and `webhook_zero_tools` (index 1):** The new guard must fire AFTER `webhook_ready_label_dispatch` because ready-label turns should dispatch (positive case handled by the existing guard). It should fire BEFORE `webhook_zero_tools` because if it fires and rejects `run_claude_pilot`, the agent may retry with acknowledge-only tool calls that satisfy `webhook_zero_tools` — but the ordering is actually not critical since each guard gets an independent retry flag. Placing it at index 1 (between ready-label and zero-tools) is the clearest logical grouping.
- **Predicate checks successful `run_claude_pilot` only:** Unlike `webhook_ready_label_dispatch` which counts attempts (success or failure), this guard should only reject SUCCESSFUL `run_claude_pilot` calls. If the dispatch was attempted and failed (e.g., `task_not_dispatchable`), the structural dispatch-readiness guard already blocked it — no need for double-rejection. The issue is unauthorized successful dispatch, not attempted dispatch that was already caught by other guards.
- **Trigger and satisfied as named functions:** Following the pattern of `ready_label_dispatch_trigger`/`ready_label_dispatch_satisfied` rather than inline closures, for testability and readability.

## Implementation Units

- [ ] **Unit 1: Add guard entry and helper functions**

**Goal:** Add the `webhook_no_unauthorized_dispatch` entry to `INTENT_GUARDS` and its trigger/satisfied helper functions.

**Requirements:** R1, R2, R3, R4, R5

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`

**Approach:**
- Add `webhook_no_unauthorized_dispatch_trigger(msg)` function: `msg.starts_with("[GitHub]") && !msg.starts_with(READY_LABEL_DISPATCH_MARKER)`. This is the inverse of the ready-label trigger on the `[GitHub]` domain.
- Add `webhook_no_unauthorized_dispatch_satisfied(summaries)` function: `!summaries.iter().any(|s| s.name == "run_claude_pilot" && s.success)`. Note the negation — the guard is satisfied when `run_claude_pilot` was NOT successfully called.
- Add the `IntentPrecondition` entry at index 1 in the `INTENT_GUARDS` array (after `webhook_ready_label_dispatch`, before `webhook_zero_tools`).
- Correction message should cite mika#841 Layer 1 source-check and explain that only `[GitHub] Issue labeled ready on` webhooks may dispatch.

**Patterns to follow:**
- `ready_label_dispatch_trigger` / `ready_label_dispatch_satisfied` function pair
- Existing `INTENT_GUARDS` entry comment style (issue number, rationale, predicate explanation)

**Test scenarios:**
- Happy path: trigger returns `true` for `[GitHub] New comment on mika#906 ...`
- Happy path: trigger returns `true` for `[GitHub] Issue labeled bug on mika#999`
- Happy path: trigger returns `true` for `[GitHub] PR review (approved) on mika#694 ...`
- Happy path: trigger returns `true` for `[GitHub] Check suite failure on branch fix/foo`
- Edge case: trigger returns `false` for `[GitHub] Issue labeled ready on mika#999` (ready-label events excluded)
- Edge case: trigger returns `false` for direct prompts (no `[GitHub]` prefix)
- Edge case: trigger returns `false` for empty string
- Happy path: satisfied returns `true` when no `run_claude_pilot` in summaries (acknowledge-only)
- Happy path: satisfied returns `true` when `run_claude_pilot` attempted but failed (structural guard already caught it)
- Happy path: satisfied returns `true` when empty summaries
- Error path: satisfied returns `false` when `run_claude_pilot` succeeded (unauthorized dispatch)

**Verification:**
- `cargo test -p mika-agent` passes (unit tests for new trigger/satisfied functions)
- New guard entry appears in `INTENT_GUARDS` at the correct position

- [ ] **Unit 2: Add unit tests**

**Goal:** Unit tests for the new trigger and satisfied predicates, plus a registry ordering invariant test.

**Requirements:** R6, R7

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Add trigger tests mirroring the `ready_label_trigger_*` test pattern
- Add satisfied tests mirroring the `ready_label_satisfied_*` / `ready_label_not_satisfied_*` pattern
- Add ordering invariant test: `webhook_no_unauthorized_dispatch` must appear AFTER `webhook_ready_label_dispatch` and BEFORE `webhook_zero_tools`
- Verify total guard count in registry (5 entries after this change)

**Patterns to follow:**
- `ready_label_trigger_matches_canonical_marker` test style
- `ready_label_guard_runs_before_webhook_zero_tools` ordering test

**Test scenarios:**
- Trigger positive: comment event, label event (non-ready), PR review, check suite
- Trigger negative: ready-label event, direct prompt, empty string, `[callback:` prefix
- Satisfied positive: empty summaries, only `run_gh` calls, `run_claude_pilot` failed
- Satisfied negative: `run_claude_pilot` succeeded
- Ordering: new guard between `webhook_ready_label_dispatch` and `webhook_zero_tools`
- Registry size: exactly 5 entries

**Verification:**
- All new unit tests pass
- Existing unit tests for other guards unchanged and passing

- [ ] **Unit 3: Add integration tests (EvalHarness)**

**Goal:** Integration tests exercising the guard through the full agent loop.

**Requirements:** R4, R6, R8

**Dependencies:** Unit 1

**Files:**
- Create: `crates/mika-agent/tests/eval/test_webhook_no_unauthorized_dispatch_guard.rs`
- Modify: `crates/mika-agent/tests/eval/mod.rs` (register new test module)

**Approach:**
- Create a `StubRunClaudePilotTool` that simulates successful `run_claude_pilot` dispatch
- Test 1 (guard fires): Send `[GitHub] New comment on senara-solutions/mika#906 ... implement mika issue#906 ...` with mock responses that call `run_claude_pilot` → assert the guard rejects and the agent retries without dispatch
- Test 2 (guard skips on ready-label): Send `[GitHub] Issue labeled ready on mika#906` with mock responses that call `run_claude_pilot` → assert dispatch proceeds (existing guard handles this case, not the new one)
- Test 3 (guard skips on non-webhook): Send a direct prompt `implement mika issue#906` → assert no guard interference
- Test 4 (acknowledge-only passes): Send `[GitHub] Issue labeled bug on mika#906` with mock responses that call `run_gh` then end turn → assert turn completes normally (webhook_zero_tools satisfied, new guard not triggered for dispatch rejection since no `run_claude_pilot` was called)
- Test 5 (guard fires only once): Send comment event, mock two responses both calling `run_claude_pilot` → assert guard fires once, second attempt proceeds

**Patterns to follow:**
- `test_webhook_zero_tools_guard.rs` — stub tool pattern, EvalHarness builder, assertion helpers
- `test_intent_precondition_guard.rs` — registry-driven guard test patterns

**Test scenarios:**
- Integration: comment event with `run_claude_pilot` call → guard fires, rejects
- Integration: ready-label event with `run_claude_pilot` call → guard does not fire (ready-label guard handles)
- Integration: direct prompt → no guard interference
- Integration: non-ready label event with acknowledge-only tools → normal completion
- Integration: guard fires at most once (single-retry semantics)

**Verification:**
- All integration tests pass via `cargo test -p mika-agent --test eval`
- Existing webhook and intent guard integration tests still pass

## System-Wide Impact

- **Interaction graph:** The new guard interacts with `webhook_ready_label_dispatch` via mutual exclusion on the trigger predicate — ready-label events are excluded from the new guard's trigger. Both guards share `READY_LABEL_DISPATCH_MARKER` for consistency. The `webhook_zero_tools` guard remains independent (any successful tool satisfies it).
- **Error propagation:** Guard rejection injects a correction message and retries once; second failure allows EndTurn normally. No change to the retry semantics.
- **State lifecycle risks:** None — the guard is stateless (inspects message prefix and tool summaries only).
- **API surface parity:** No API changes.
- **Unchanged invariants:** The PR review early-accept (`has_successful_pr_review()`) skips guards #3-#8 but not #1 or #2. The new guard at position 1 is also skipped by early-accept (`skip_remaining_guards`). This is correct — if a PR review was successfully posted on a webhook turn, the turn should be accepted regardless.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| False positive on ready-label events | Trigger explicitly excludes `READY_LABEL_DISPATCH_MARKER` prefix; unit test verifies |
| Guard position relative to `skip_remaining_guards` | Early-accept fires on successful PR review, which is fine — PR review webhook turns should accept without dispatch check |
| Ordering sensitivity with existing guards | Ordering invariant test enforces position between ready-label and zero-tools guards |

## Sources & References

- Related issues: #910, #841, #846, #847, #838, #798
- Related code: `crates/mika-agent/src/agent.rs` (INTENT_GUARDS registry, guard evaluation loop)
- Compound docs: `docs/solutions/workflow-issues/comment-event-fires-autonomous-dispatch-2026-04-25.md`, `docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md`

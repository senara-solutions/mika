---
title: "feat: Reject EndTurn on webhook turns with zero tool calls"
type: feat
status: active
date: 2026-04-20
issue: 696
---

# feat: Reject EndTurn on webhook turns with zero tool calls

## Overview

Add a new post-condition guard (#7) in the agent loop's EndTurn chain that rejects turns where the user message starts with `[GitHub]` but zero successful tool calls were made. This structurally enforces the invariant that webhook events require action — currently only a text-only prompt rule that the LLM violates under cognitive load.

## Problem Frame

When mika-dev receives a GitHub webhook (prefixed `[GitHub]`) and the LLM responds with text-only (zero tool calls), the response is fabrication — unrelated to the actual event. The existing prompt rule at `self-dev/system_prompt.md:99` was violated in production (session `049e853b`, 2026-04-20). The agent narrated a Rust tutorial instead of processing a PR approval webhook, silently dropping the event. This class of failure is silent (the agent LOOKS like it responded) and leaves downstream state unupdated.

Engine-level guards are the correct architectural layer for behavioral invariants that fight trained model gradients (per `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`).

## Requirements Trace

- R1. Pre-termination guard: webhook-triggered turns (user message starts with `[GitHub]`) with zero successful tool calls are rejected; agent is re-invoked with corrective context
- R2. Single-retry semantics: guard fires at most once per turn (consistent with all existing guards)
- R3. Eval harness test: mock a `[GitHub]` webhook message, mock LLM to return text-only response, verify engine rejects EndTurn and forces continuation
- R4. `self-dev/system_prompt.md:99` updated to reference the structural guard
- R5. Guard is not global — only fires when user message starts with `[GitHub]` (regular user messages without tool calls remain valid)

## Scope Boundaries

- The guard checks for zero *successful* tool calls (via `all_tool_summaries`), not just any tool invocation
- Only `[GitHub]` prefix triggers the guard (not `[claude-pilot]` callbacks — those have their own enforcement via `is_callback_turn`)
- No changes to webhook routing, gateway formatting, or skill matching
- No model calibration or prompt-only fixes

## Context & Research

### Relevant Code and Patterns

- **Guard chain location:** `crates/mika-agent/src/agent.rs` lines 744–1061 (6 existing guards + 1 early-accept)
- **State variables:** Lines 591–636 — each guard has a `*_retry_done` boolean, initialized `false`
- **User input capture:** `user_input_text` (line 618) — extracted once before the loop, always reflects the real user message
- **Guard #5 (fabricated action-claim):** Lines 976–1008 — closest pattern to follow (zero tool calls + content detection)
- **Eval test examples:** `crates/mika-agent/tests/eval/test_completion_claim_guard.rs`, `test_fabricated_action_guard.rs`
- **Self-dev prompt:** `skills/bundled/self-dev/system_prompt.md` line 99

### Institutional Learnings

- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — confirms engine guards are the right layer for against-gradient invariants
- `docs/solutions/architecture-patterns/fabricated-action-claim-guard.md` — documents the exact implementation pattern to follow
- `docs/solutions/prompt-engineering/2026-04-12-tighten-webhook-qa-pass-entry-point.md` — the prompt fix for the same failure mode; confirms structural guard is the deeper solution
- `docs/solutions/prompt-enforcement-structural-guards.md` — documents early-accept pattern and re-execution risks

## Key Technical Decisions

- **Position in guard chain:** After guard #5 (fabricated-action-claim), before #6 (persistence-eval). Rationale: The webhook guard fires on a narrower condition (specific prefix) than #5 (any URL claim), so placing it after avoids redundant checks when #5 already fires. It must be after text-tool-call and prose-tool-call detectors since those might convert text into actual calls on retry.
- **Success check uses `all_tool_summaries`:** Check `!all_tool_summaries.iter().any(|s| s.success)` rather than `tools_called.is_empty()`. This distinguishes between "called tools but they all failed" (let through — the agent tried) vs "never called any tool" (reject). The issue specifies "zero successful tool calls" as the trigger.
- **Respects `skip_remaining_guards`:** If the PR review early-accept fires (#3b), this guard is skipped — consistent with guards #4–#6.
- **No regex needed for detection:** `user_input_text.starts_with("[GitHub]")` is a trivial fast-path check. No `LazyLock<Regex>` required.
- **Guard number:** This becomes guard #6 in the chain; the existing persistence-eval becomes #7.

## Open Questions

### Resolved During Planning

- **Should `[claude-pilot]` callbacks also trigger this guard?** No — callback turns already have enforcement via `is_callback_turn` and the separate callback lifecycle. The issue explicitly scopes this to `[GitHub]` webhooks.
- **Should the guard check `tools_called.is_empty()` or `all_tool_summaries` for success?** Use `all_tool_summaries` for successful calls. If the agent attempted tools but they all failed, the agent DID try to process the webhook — the guard should not re-reject for external failures.

### Deferred to Implementation

- Exact wording of the corrective message — should follow the established `[Your response was rejected because...]` format but specific text will be refined during implementation.

## Implementation Units

- [ ] **Unit 1: Add webhook zero-tools guard to agent loop**

**Goal:** Implement the new post-condition guard that rejects EndTurn when a `[GitHub]`-prefixed message has zero successful tool calls.

**Requirements:** R1, R2, R5

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`
- Test: `crates/mika-agent/tests/eval/test_webhook_zero_tools_guard.rs`

**Approach:**
- Add `webhook_zero_tools_retry_done: bool = false` alongside existing guard flags (~line 607)
- Insert the guard block after the fabricated-action-claim guard (~line 1008), before the persistence-eval guard (~line 1014)
- Condition: `!skip_remaining_guards && EndTurn && !webhook_zero_tools_retry_done && user_input_text.starts_with("[GitHub]") && !all_tool_summaries.iter().any(|s| s.success)`
- Push assistant response + corrective user message, then `continue`
- Log `warn!` with `step`, `label = mode.label()`

**Patterns to follow:**
- Guard #5 (fabricated-action-claim, lines 976–1008) — identical structure minus the content detection regex
- All guards use single-retry via `_retry_done` flag
- Corrective message format: `[Your response was rejected because ...]`

**Test scenarios:**
- Happy path: `[GitHub]` message + text-only LLM response (zero tools) → guard fires, 2 steps total, second response includes tool calls
- Happy path: `[GitHub]` message + LLM calls tools successfully → guard does NOT fire, 1 step
- Edge case: `[GitHub]` message + tool called but failed (no successful calls) → guard fires (zero *successful* calls)
- Edge case: `[GitHub]` message + tool called and succeeded → guard does NOT fire
- Edge case: Regular user message (no `[GitHub]` prefix) + zero tool calls → guard does NOT fire
- Edge case: Guard already fired once (`_retry_done = true`) + second response still has zero tools → guard does NOT fire again (single retry)
- Integration: `skip_remaining_guards` is true (PR review early-accept) + `[GitHub]` message + zero tools → guard does NOT fire

**Verification:**
- All new eval tests pass
- Existing guard tests still pass (no regression)
- `cargo clippy` clean

- [ ] **Unit 2: Register eval test module**

**Goal:** Wire up the new test file in the eval test root.

**Requirements:** R3

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/tests/eval.rs`

**Approach:**
- Add `mod test_webhook_zero_tools_guard;` to the eval test module declarations

**Test expectation:** none — this is test infrastructure wiring

**Verification:**
- `cargo test -p mika-agent --test eval test_webhook_zero_tools` runs and passes

- [ ] **Unit 3: Update self-dev system prompt**

**Goal:** Update the self-dev prompt line 99 to reference the structural guard, reinforcing that this is now engine-enforced.

**Requirements:** R4

**Dependencies:** Unit 1

**Files:**
- Modify: `skills/bundled/self-dev/system_prompt.md`

**Approach:**
- Amend the CRITICAL block at line 99 to note that this invariant is now structurally enforced by the engine (not just a text rule)
- Keep the existing text for LLM guidance but add a note like: "This rule is enforced by the engine — webhook turns with zero tool calls will be rejected."

**Test expectation:** none — prompt text change only

**Verification:**
- The prompt still reads clearly and accurately describes the enforced behavior
- `cargo build` succeeds (build.rs re-discovers bundled skills)

## System-Wide Impact

- **Interaction graph:** The guard interacts with the `skip_remaining_guards` flag from guard #3b (PR review early-accept). No other cross-guard dependencies.
- **Error propagation:** Guard rejection is a re-prompt, not an error. The agent gets one retry to call tools. If it still doesn't, the turn ends normally (text is saved).
- **State lifecycle risks:** None — the guard only adds a retry opportunity, never blocks termination permanently.
- **API surface parity:** No API changes. The guard is internal to the agent loop.
- **Unchanged invariants:** All existing guards remain in their positions and logic. The persistence-eval guard shifts from position #6 to #7 in documentation numbering but its code is untouched.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| False positive on user messages that happen to start with `[GitHub]` | Extremely unlikely — user messages don't start with this literal prefix; only the gateway's `format_event_text()` produces it |
| Guard fires unnecessarily when the agent legitimately has nothing to do | The corrective message allows the agent to explain it cannot act; single-retry ensures it won't loop |
| Ordering interaction with existing guards | Placed after all tool-call-detection guards (#1, #2) which might convert text to actual calls, and after fabricated-action (#5) which already catches URL claims with zero tools |

## Sources & References

- Related issue: #696
- Related code: `crates/mika-agent/src/agent.rs` (guard chain)
- Related learnings: `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`
- Related learnings: `docs/solutions/architecture-patterns/fabricated-action-claim-guard.md`

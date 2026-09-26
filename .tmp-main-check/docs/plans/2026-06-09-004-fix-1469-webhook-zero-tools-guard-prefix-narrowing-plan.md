---
title: "fix(engine): webhook_zero_tools guard prefix-narrowing for no-correlation events"
status: active
created: 2026-06-09
type: fix
origin: GitHub issue mika#1469
groom_session_id: 557a7808-17f2-4f7e-bcfe-25e8df3021d9
---

# fix(engine): webhook_zero_tools guard prefix-narrowing for no-correlation events

## Summary

Narrow the `webhook_zero_tools` intent-precondition guard (`crates/mika-agent/src/agent.rs:5852`) so it stops firing on three documented no-op event classes: `Check suite success`, `PR closed`, and `discussion.*`. These events have no actionable response — the agent's correct behavior is text-only acknowledgement — but the current guard's trigger (`msg.starts_with("[GitHub]")`) fires on every webhook and re-prompts the LLM, producing the 25+ misfires documented in `current_priorities` core memory as of 2026-06-09.

The fix is a single-function-pointer change to the trigger predicate plus regression tests. Scope is intentionally minimal: high-confidence prefix-based skip-list using only message-text signals already available to the trigger's `fn(&str) -> bool` signature. Correlation-aware filtering (which would require widening the trigger signature or adding DB lookups at message construction) is out of scope — that's a follow-up if these three prefix skips don't cover the long tail.

## Problem frame

**Current behavior:**

The `webhook_zero_tools` intent-precondition guard in `INTENT_GUARDS` (`agent.rs:5852-5862`):

```rust
IntentPrecondition {
    label: "webhook_zero_tools",
    trigger: |msg| msg.starts_with("[GitHub]"),
    satisfied: |summaries| summaries.iter().any(|s| s.success),
    correction_message: "[mika-engine] A GitHub webhook event was received but the \
         response was text-only with zero tool calls. Webhook events require \
         action — the engine expects at least one tool call ...",
},
```

fires on any user message whose text begins with `[GitHub]`. When fired, the engine injects the correction message and re-prompts the LLM once (single-retry, tracked via `intent_guard_retries: HashSet<&'static str>`).

**Misfire patterns (from issue body):**

1. `check_suite.completed(success)` on `main` — no in-progress tasks → guard fires
2. `check_suite.completed(failure)` on `main` for recurring Release Please failure with no correlated task → guard fires
3. `pull_request.closed(merged: true)` for out-of-band PRs (no sprint work item) → guard fires
4. `pull_request_review.submitted` (QA pass) for out-of-band PRs → guard fires
5. `issue_comment.created` closure comments on already-merged PRs → guard fires

**Empirical evidence:** 25+ documented misfires as of 2026-06-09 in `current_priorities` core memory. Each misfire pressures the agent to call a tool (`list_tasks`, `send_message`, etc.) just to satisfy the guard, violating dispatch discipline.

## Scope

### In scope (this plan, this PR)

- Narrow the `webhook_zero_tools` trigger function to **skip three prefix classes** that are always informational and never action-bearing:
  - `[GitHub] Check suite success on ...`
  - `[GitHub] PR closed: ...`
  - `[GitHub] discussion.*`
- Regression tests in `crates/mika-agent/tests/eval/grounding_regressions/` covering:
  - Each of the three skipped prefix classes → guard does NOT fire → text-only response accepted
  - Each retained prefix class (`Issue labeled ready`, `Check suite failure`, `PR opened`, `PR review`, `New comment on`, `Issue labeled <other>`) → guard STILL fires → text-only response rejected (or accepted iff the agent called a tool)
- A short documentation note in the guard source comment explaining the prefix skip rationale + the empirical-misfire reference.

### Deferred to Follow-Up Work

- **Correlation-aware filtering** — if the long-tail of misfires includes `Check suite failure` on untracked branches, `PR review` on out-of-band PRs, or `New comment on` already-merged PRs, those require knowing whether the event is "correlated with an active task." The trigger function's signature today is `fn(&str) -> bool` — it doesn't see DB state. A correlation-aware approach would either (a) widen the trigger signature to accept context (invasive across 10+ guards), or (b) add a DB lookup at the webhook router's message-construction site to encode correlation into the prefix (e.g., `[GitHub:no-correlation] ...`). Both are larger changes than this slice and should be triaged after observing whether the three-prefix skip is sufficient.
- **Prompt-side language** — the agent's system prompt has language about "webhook events require action" that should be aligned with the narrower guard. Out of scope for this slice; flag for the next prompt-cleanup pass.

### Outside this fix's identity

- Restructuring the `INTENT_GUARDS` registry framework itself.
- Removing `webhook_zero_tools` entirely — the guard still has a load-bearing role on action-bearing events (CI failure on a tracked PR, etc.).

## Requirements

- **R1.** The guard MUST NOT fire on `[GitHub] Check suite success on ...` messages.
- **R2.** The guard MUST NOT fire on `[GitHub] PR closed: ...` messages.
- **R3.** The guard MUST NOT fire on `[GitHub] discussion.<anything>` messages.
- **R4.** The guard MUST continue to fire on `[GitHub] Issue labeled ready on ...` messages (regression — the existing `webhook_ready_label_dispatch` more-specific guard at agent.rs:5805 handles this case first, but `webhook_zero_tools` is the fallback if ready-label-dispatch is somehow skipped).
- **R5.** The guard MUST continue to fire on `[GitHub] Check suite failure on ...` messages — failures are potentially action-bearing.
- **R6.** The guard MUST continue to fire on `[GitHub] PR review (...) on ...` messages — reviews may carry QA verdicts requiring action.
- **R7.** The guard MUST continue to fire on `[GitHub] New comment on ...` messages — comments may carry directives requiring action.
- **R8.** The guard MUST continue to fire on `[GitHub] PR opened: ...`, `[GitHub] Issue opened: ...`, `[GitHub] Issue assigned: ...`, `[GitHub] Issue labeled <non-ready>` — all are potentially action-bearing.

## Key Technical Decisions

### KTD1. Prefix-based skip, not correlation lookup

The architect's pass-2 review (session `557a7808-17f2-4f7e-bcfe-25e8df3021d9`) noted that the proposed `[GitHub:no-correlation]` marker approach (approach α from the brief) would require the webhook router to perform a DB lookup at message-construction time to determine correlation — a larger change than initially scoped. Source reading in `crates/mika-agent/src/webhook_dispatch.rs:30-184` confirmed:

- The current webhook message text has NO `task_id` field, NO `correlated_task_id`, and NO marker shape suitable for correlation signaling.
- For issue/PR/comment/review events, the message contains `<repo>#<n>` (parseable, but the lookup is still DB-side).
- For `Check suite` events, the message contains only `(branch: <name>)` — no `<repo>#<n>` — and determining "is this branch tracked by an active task" requires a branch→SHA→PR→task chain of DB lookups.

This is significant DB plumbing for a single guard. **This plan adopts the simpler shape: prefix-based skip on the three highest-frequency no-op classes.** The three classes (`Check suite success`, `PR closed`, `discussion.*`) are always informational regardless of correlation state — there's nothing useful the agent can do with them even when correlated. The minority remaining misfires (failure events on untracked branches, etc.) are deferred to the follow-up correlation-aware pass when warranted.

### KTD2. Keep the trigger function's signature unchanged

`IntentPrecondition::trigger: fn(&str) -> bool` is used by 10+ guards in the registry. Widening the signature to accept context state would force a no-op edit to every other guard. The fix stays within the existing function-pointer shape: a slightly more elaborate predicate that inspects multiple prefix patterns.

### KTD3. Single-function-pointer change, no new types

The fix is a single `trigger` predicate replacement on the existing `IntentPrecondition` entry. No new types, no new traits, no new modules. This is intentionally minimal — the change is small enough that the test suite is more code than the production change.

### KTD4. Test fixtures use the existing webhook message shapes from `webhook_dispatch::tests`

`webhook_dispatch.rs:60-184` already maintains a test corpus of canonical webhook message shapes for all event classes. Tests in this plan use the same string literals so the test corpus stays in lock-step with the routing layer. If `webhook_dispatch.rs` changes a prefix shape in a future ticket, the regression tests here surface the divergence.

## High-Level Technical Design

```
                          BEFORE                                AFTER
                                                                
agent.rs:5852                                       agent.rs:5852
INTENT_GUARDS entry                                 INTENT_GUARDS entry
   trigger: |msg|                                      trigger: |msg|
     msg.starts_with("[GitHub]")  ───┐                   webhook_zero_tools_trigger(msg)
                                     │                                 │
   ──── fires on EVERY                                   ──── fires only on action-bearing
        [GitHub]-prefixed                                     [GitHub]-prefixed events
        message                                               (excludes 3 known no-op prefixes)
                                                                
                                                    fn webhook_zero_tools_trigger(msg: &str) -> bool {
                                                        if !msg.starts_with("[GitHub]") { return false; }
                                                        // Skip: always-informational event classes
                                                        if msg.starts_with("[GitHub] Check suite success on")
                                                            || msg.starts_with("[GitHub] PR closed:")
                                                            || msg.starts_with("[GitHub] discussion.")
                                                        {
                                                            return false;
                                                        }
                                                        true
                                                    }
```

Directional only — exact identifier/lint compliance resolved at execution time.

## Implementation Units

### U1. Extract trigger to a named function + add skip-list

**Goal:** Replace the closure `|msg| msg.starts_with("[GitHub]")` on the `webhook_zero_tools` registry entry with a named function that adds the three-prefix skip list.

**Requirements:** R1, R2, R3 (the skips); R4–R8 (regressions: all other prefixes still fire).

**Dependencies:** none.

**Files:**
- `crates/mika-agent/src/agent.rs` — modify the `webhook_zero_tools` IntentPrecondition entry at line 5852; add a new `fn webhook_zero_tools_trigger(msg: &str) -> bool` near the other guard predicates (sibling to `ready_label_dispatch_trigger` at line ~5900).

**Approach:**
- Extract the inline closure into a named function — better for testability + matches the existing pattern for the more-specific guards (`webhook_ready_label_dispatch` uses `ready_label_dispatch_trigger`, etc.).
- The new function body checks the `[GitHub]` prefix, then short-circuits on the three documented no-op prefixes.
- Update the registry entry to reference the new function pointer.
- Update the doc comment above the entry to cite mika#1469 and reference the three skipped prefix classes.

**Patterns to follow:**
- `ready_label_dispatch_trigger` at `agent.rs:5930` — the existing named-trigger pattern for the sibling more-specific guard.
- `webhook_no_unauthorized_dispatch_trigger` (search for it) — another sibling named trigger.

**Test scenarios** (inline `#[cfg(test)] mod tests` in `agent.rs`):
- `webhook_zero_tools_trigger` returns `false` for `[GitHub] Check suite success on senara-solutions/mika (branch: main)`.
- `webhook_zero_tools_trigger` returns `false` for `[GitHub] PR closed: senara-solutions/mika#1000 — title (branch: foo)`.
- `webhook_zero_tools_trigger` returns `false` for `[GitHub] discussion.created on senara-solutions/mika`.
- `webhook_zero_tools_trigger` returns `true` for `[GitHub] Check suite failure on senara-solutions/mika (branch: fix/foo)` (regression — failures still fire).
- `webhook_zero_tools_trigger` returns `true` for `[GitHub] Issue labeled ready on senara-solutions/mika#933 — title` (regression).
- `webhook_zero_tools_trigger` returns `true` for `[GitHub] PR review (approved) on senara-solutions/mika#1000 (title) by @reviewer` (regression).
- `webhook_zero_tools_trigger` returns `true` for `[GitHub] New comment on senara-solutions/mika#933 (title) by @samidarko` (regression).
- `webhook_zero_tools_trigger` returns `true` for `[GitHub] Issue labeled bug on senara-solutions/mika#999` (regression — non-ready labels still fire).
- `webhook_zero_tools_trigger` returns `false` for `[Slack] message` (regression — non-[GitHub] prefix never fires).

**Verification:** `cargo test -p mika-agent agent::tests::webhook_zero_tools_trigger` passes all 9 scenarios.

### U2. End-to-end eval test in `tests/eval/grounding_regressions/`

**Goal:** Prove the guard-narrowing fix prevents the documented misfire pattern end-to-end through the full agent loop, not just at the predicate level.

**Requirements:** R1, R2, R3.

**Dependencies:** U1.

**Files:**
- `crates/mika-agent/tests/eval/grounding_regressions/webhook_zero_tools_no_correlation.rs` (NEW) — sibling to `engine_correction_rejection.rs`.

**Approach:**
- Use the existing `EvalHarness` builder (`crates/mika-agent/tests/eval/harness.rs` — pattern documented in mika/CLAUDE.md) with `MockLlmProvider` to drive a synthetic agent turn.
- The mock LLM returns a text-only EndTurn for a `[GitHub] Check suite success on ...` user message.
- Assert that the engine does NOT inject the `webhook_zero_tools` correction message — i.e., no second LLM call is made.
- Repeat for `[GitHub] PR closed:` and `[GitHub] discussion.` prefixes.
- Negative-case test: a text-only EndTurn for `[GitHub] Check suite failure on ...` STILL triggers the correction injection (regression check on R5).

**Patterns to follow:**
- `crates/mika-agent/tests/eval/grounding_regressions/engine_correction_rejection.rs` — sibling test that exercises the `webhook_ready_label_dispatch` intent-guard's correction path. The new test inverts that pattern: assert correction is NOT injected for the three skipped prefixes.
- `EvalHarness` builder documented in mika/CLAUDE.md § Testing.

**Test scenarios:**
- `[GitHub] Check suite success on senara-solutions/mika (branch: main)` + text-only EndTurn → no correction injection, no second LLM call.
- `[GitHub] PR closed: senara-solutions/mika#999 — title (branch: foo)` + text-only EndTurn → no correction injection.
- `[GitHub] discussion.created on senara-solutions/mika` + text-only EndTurn → no correction injection.
- **Regression:** `[GitHub] Check suite failure on senara-solutions/mika (branch: fix/foo)` + text-only EndTurn → correction IS injected → second LLM call attempted.

**Verification:** `cargo test -p mika-agent --test eval -- grounding_regressions::webhook_zero_tools_no_correlation` passes all four scenarios.

### U3. Update guard doc comment + reference mika#1469

**Goal:** Future maintainers reading the guard registry understand the prefix skip-list rationale without needing to re-derive it.

**Requirements:** documentation hygiene.

**Dependencies:** U1.

**Files:**
- `crates/mika-agent/src/agent.rs` — extend the doc comment above the `webhook_zero_tools` IntentPrecondition entry.

**Approach:**
- One-paragraph addition citing mika#1469 (this fix), naming the three skipped prefix classes, and noting that correlation-aware filtering for the long tail is a deferred follow-up.

**Test expectation: none — pure doc comment change.**

**Verification:** the comment is present and grammatically clean; covered by review-time read.

## Risks & Dependencies

- **Risk: A new event class is added in the future** that ALSO is always informational, and contributors forget to add it to the skip list. **Mitigation:** the skip list lives next to the trigger predicate function with a clear comment; the eval-test corpus uses the same string literals as `webhook_dispatch::tests`, so any new event class added to the routing layer will surface in the test corpus at the same time.
- **Risk: A skipped prefix matches an action-bearing event** the issue body didn't list. **Mitigation:** the three skipped prefixes (`Check suite success`, `PR closed`, `discussion.*`) are by construction informational — there's no agent action that should follow a successful CI or a closed PR or a discussion event. If a counter-example surfaces in production, the fix is to remove that prefix from the skip list and re-tighten with correlation-aware filtering.
- **Dependency: `webhook_dispatch::tests` corpus is stable.** The test fixtures use the canonical message shapes from `webhook_dispatch.rs:60-184`. If those shapes change without a sibling test update, the regression tests here may falsely pass against a stale corpus. **Mitigation:** the test file imports or copies the literal strings directly from `webhook_dispatch`'s test module where possible.

## Open Questions (deferred to execution)

- Whether the `webhook_zero_tools_trigger` function should live in `agent.rs` alongside the other named triggers, or be extracted to a sibling module if `agent.rs` is already at its line-count budget. Resolved at execution time by checking file size — current `agent.rs` is 10k lines per mika#1259, so a small addition here is acceptable.
- Exact wording of the correction message — current message stays as-is for this slice; future prompt-cleanup pass owns rewording.

## Sources & Research

- **Origin:** GitHub issue mika#1469 — `bug(engine): engine guard misfires on webhook events with no correlated work item`.
- **Pinned guard source:** `crates/mika-agent/src/agent.rs:5852-5862` (the `webhook_zero_tools` IntentPrecondition entry).
- **Pinned routing source:** `crates/mika-agent/src/webhook_dispatch.rs:30-184` (the message prefix shapes + the existing `is_unauthorized_webhook_dispatch` / `is_ready_label_dispatch_marker` predicates).
- **Sibling guards:** `webhook_ready_label_dispatch` (agent.rs:5805), `webhook_no_unauthorized_dispatch` (agent.rs:5820), `resume_reconcile`, `callback_terminal_action`, `deferred_dispatch_action`.
- **Test pattern reference:** `crates/mika-agent/tests/eval/grounding_regressions/engine_correction_rejection.rs`.
- **Orthogonal sibling ticket:** mika#1324 — `dispatch_arg_match` reframed in grooming as tool-fabrication at request-assembly. Fix layer: tool-filter, not guard-trigger. Confirmed orthogonal to this fix; both can ship independently.
- **Grooming history:**
  - First-pass review session: `557a7808-17f2-4f7e-bcfe-25e8df3021d9`
  - Brief iterations: initial brief (ITERATE — 4 findings) → revised brief with pinned source (ITERATE — 2 sharpening findings) → final pin via direct source reading (this plan)

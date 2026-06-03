# Plan: Fix asserted_unavailability bypassed by has_successful_pr_review skip path

**Ticket:** mika#1178
**Type:** bug fix
**Risk:** low — narrowing an existing skip path, no new features

## Problem

When `has_successful_pr_review()` returns true on an EndTurn, `skip_remaining_guards` is set to `true`, which bypasses guards #4 through the end of the chain. This includes the asserted_unavailability guard (6c) and the assert-grounded guard (6d), which are fabrication-class guards orthogonal to the PR-review completion semantics.

The original skip intent was: "the primary action (PR review) completed; forced continuation risks duplicate submissions." This reasoning applies to completion-claim (#4), milestone-close-claim (#4b), fabricated-action-claim (#5), dev-groom fabrication (#5b), and intent-precondition guards (#6, #6b) — all of which check whether the agent *did its job*. It does NOT apply to:

- **6c (asserted_unavailability):** Detects fabricated claims that a tool is unavailable. A successful PR review does not grant license to claim other tools are uncallable.
- **6d (assert-grounded):** Detects affirmative state claims about resources without grounding tool calls. A successful PR review does not ground claims about unrelated issues/PRs.

## Chosen approach: Option A — narrow the skip

Move the asserted_unavailability guard (6c) and assert-grounded guard (6d) OUT of the `skip_remaining_guards` gate. These two guards will fire regardless of whether a PR review was posted.

This is the right structural fix because:
1. These guards detect a different failure family (claim-without-evidence) than the guards the skip was designed for (action-without-completion).
2. The two-layer enforcement pattern (#862) requires the asserted_unavailability guard to always run as the second layer behind required-tools.
3. No false-positive risk: if the agent posted a PR review AND also claimed a tool is unavailable, the guard correctly forces verification.

## Implementation

### File: `crates/mika-agent/src/agent.rs`

**Change 1 — Remove `!skip_remaining_guards` from guard 6c (asserted_unavailability)**

At line ~1707, change:

```rust
if !skip_remaining_guards
    && matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !intent_guard_retries.contains(ASSERTED_UNAVAILABILITY_LABEL)
```

to:

```rust
if matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !intent_guard_retries.contains(ASSERTED_UNAVAILABILITY_LABEL)
```

**Change 2 — Remove `!skip_remaining_guards` from guard 6d (assert-grounded)**

At line ~1757, change:

```rust
if !skip_remaining_guards
    && matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !intent_guard_retries.contains(ASSERT_GROUNDED_LABEL)
```

to:

```rust
if matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !intent_guard_retries.contains(ASSERT_GROUNDED_LABEL)
```

**Change 3 — Update the log message at line ~1288**

Change:

```rust
"PR review already posted — accepting EndTurn (skipping guards #4-#6)"
```

to:

```rust
"PR review already posted — accepting EndTurn (skipping guards #4-#6b)"
```

This accurately reflects that guards 6c and 6d are no longer skipped.

**Change 4 — Update the comment at line ~1276-1279**

Change:

```rust
// PR review early-accept: if the turn already contains a
// successful `gh pr review` call, skip guards #4–#6. The primary
// action completed — forced continuation would risk duplicate
// review submissions. See #695.
```

to:

```rust
// PR review early-accept: if the turn already contains a
// successful `gh pr review` call, skip guards #4–#6b. Guards 6c
// (asserted_unavailability) and 6d (assert-grounded) are NOT
// skipped — they detect a different failure family
// (claim-without-evidence vs action-without-completion). The primary
// action completed — forced continuation would risk duplicate
// review submissions. See #695, #1178.
```

### File: `crates/mika-agent/CLAUDE.md`

**Change 5 — Update the CLAUDE.md guard documentation**

In the guard 3b documentation, update the description of which guards are skipped. The current text says:

> When true, guards #3 (required-tools, #821), #4–#8 are all skipped

Change to:

> When true, guards #3 (required-tools, #821), #4–#6b are skipped. Guards 6c (asserted_unavailability) and 6d (assert-grounded) are NOT skipped (#1178) — these detect claim-without-evidence, orthogonal to the PR-review completion semantics.

Also update guard 6c documentation. The current text says:

> Uses `intent_guard_retries` with label `"asserted_unavailability"` for single-retry semantics.

After that sentence, add:

> Not skipped by `skip_remaining_guards` (#1178) — a successful PR review does not grant license to fabricate tool unavailability claims.

Also update guard 6d documentation. The current text says:

> Skipped behind `skip_remaining_guards`.

Change to:

> Not skipped by `skip_remaining_guards` (#1178) — a successful PR review does not ground affirmative claims about unrelated resources.

### File: `crates/mika-agent/tests/eval/grounding_regressions/` (new test file)

**Change 6 — Add eval test: asserted_unavailability fires despite PR review early-accept**

Create `asserted_unavailability_pr_review_composition.rs` with a scenario where:
- The LLM emits a successful `run_gh` call with `gh pr review --approve` (triggering `has_successful_pr_review() == true`)
- The same EndTurn response text claims "gh_read is not callable"
- `gh_read` is in the enabled tool set
- Assert: the guard fires (response contains the correction prompt or the agent retries)

This tests the exact composition gap described in the ticket. The fixture should use the `MockLlmProvider` pattern from existing grounding regression tests:
1. First response: tool_use `run_gh` with PR review args + EndTurn text claiming `gh_read` not callable
2. Second response (after guard fires): clean EndTurn without the fabrication

Register the test module in `grounding_regressions/mod.rs`.

Update `tests/eval/grounding_regressions/README.md` scenario table with the new entry.

### File: `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`

**Change 7 — Update the evasion patterns doc**

The existing entry at line ~118 references mika#1178 as a tracked follow-up. Update it to note the fix: the `has_successful_pr_review` skip path no longer bypasses the asserted_unavailability guard.

## Test plan

1. `cargo test -p mika-agent` — all existing tests pass (no behavioral regression for the guards that remain behind `skip_remaining_guards`)
2. New eval test `asserted_unavailability_pr_review_composition` passes — proves the guard fires when PR review early-accept is active
3. Existing `has_successful_pr_review` tests at agent.rs:~9830 still pass (the helper function is unchanged)
4. Existing asserted_unavailability detection tests still pass (detection logic unchanged)
5. Existing grounding regression eval tests still pass
6. `cargo clippy` clean

## Risks

- **False positives on legitimate PR review turns:** Extremely unlikely. The asserted_unavailability guard requires: (a) the text to contain a snake_case tool name matching a specific unavailability claim pattern, AND (b) the named tool to be in the enabled set, AND (c) no call to that tool was attempted. A legitimate PR review turn would not normally contain such claims.
- **Guard ordering change:** No. The guards stay in the same position in the chain. Only the `skip_remaining_guards` gate is removed from two of them.

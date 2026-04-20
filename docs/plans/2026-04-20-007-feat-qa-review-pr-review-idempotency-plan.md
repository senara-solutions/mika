# Plan: qa-review — enforce PR review idempotency via tool-call history (#695)

## Problem

mika-qa posted two approved reviews on the same PR in a single turn (mika#694). The root cause is twofold:

1. **Post-condition guard false-positive:** After a legitimate `EndTurn` (step 3), the completion-claim guard (guard #4) fired because the response text contained completion keywords — but it keys on text, not tool-call history. The agent had already successfully called `run_gh("pr review --approve ...")`, so EndTurn should have been accepted.

2. **No tool-side duplicate prevention:** When the engine forced continuation, the LLM re-executed `run_gh("pr review --approve ...")` a second time for the same PR. The existing per-turn dedup in `process_tool_calls()` (#582) only catches identical `(name, arguments)` pairs in a *single LLM response* — it doesn't prevent the same semantic action across multiple steps within a turn.

## Solution

Two-layer fix — structural, not text:

### Layer 1: Post-condition early-accept for qa-review turns

**Location:** `crates/mika-agent/src/agent.rs`, post-condition chain (after guard #3, before guard #4).

**Logic:** If `all_tool_summaries` contains a successful `run_gh` call whose input contains `"pr"` and `"review"` (the two positional args that identify a `gh pr review` invocation), accept EndTurn immediately — skip guards #4–#6. This makes the engine structurally aware that the qa-review workflow's primary action completed.

**Implementation:**
- New helper function: `fn has_successful_pr_review(summaries: &[ToolCallSummary]) -> bool`
- Checks `summaries` for any entry where `name == "run_gh"` AND `success == true` AND `input_summary` contains both `"pr"` and `"review"` (substring match on the truncated input JSON).
- Insert check between guard #3 (required-tools) and guard #4 (completion-claim). When it returns `true`, log at `info!` level and fall through to the normal EndTurn acceptance path (skip remaining guards).

**Why this is safe:** The guard only fires when a `run_gh` pr-review call *already succeeded* — it doesn't suppress guards for turns where no review was posted. The completion-claim guard (#4) exists to catch fabricated claims; if the tool actually succeeded, there's nothing fabricated.

### Layer 2: Turn-scoped `pr review` dedup in `run_gh`

**Location:** `crates/mika-agent/src/skills/builtin_handlers.rs`, inside `run_gh()`.

**Logic:** Track `pr review` calls per-turn. If `run_gh` is called with args matching `["pr", "review", ...]` and a previous successful `run_gh` call in the same turn already posted a review for the same PR number, return a structured error instead of executing.

**Implementation:**
- Add a new field to `ToolContext`: `pr_review_posted: &std::sync::atomic::AtomicBool` (or use a `Mutex<HashSet<String>>` for PR-number granularity).
- Actually, simpler: use the existing `all_tool_summaries` passed indirectly. But `run_gh` only has access to `ToolContext`, not the summaries.
- **Better approach:** Add a `Arc<Mutex<HashSet<String>>>` field `pr_reviews_posted` to `ToolContext`. When `run_gh` detects args `["pr", "review", <number>, ...]`, check if `<number>` is already in the set. If yes, return `ToolOutput::error(...)`. If no, execute the command, and on success insert `<number>` into the set.
- The set is created fresh per `run_agent()` call (one per turn), so it resets between turns.

**Error format:**
```json
{"error": "duplicate_pr_review", "pr_number": "455", "message": "A PR review was already posted for PR #455 in this turn. Duplicate reviews create duplicate webhooks. End your turn — the review is already submitted."}
```

### Layer 3: Prompt update (documentation only)

**Location:** `skills/bundled/qa-review/system_prompt.md`, line 245.

**Change:** Update the idempotency note to reference the structural guard:

> **Idempotency (enforced):** The engine rejects duplicate `pr review` calls for the same PR within a single turn. If you attempt a second review, the tool will return a `duplicate_pr_review` error. This is expected — your first review was already posted. End your turn normally.

## Files to Change

1. `crates/mika-agent/src/agent.rs`
   - Add `has_successful_pr_review()` helper (~10 lines)
   - Insert early-accept check in post-condition chain (~15 lines)

2. `crates/mika-agent/src/skills/builtin_handlers.rs`
   - Add dedup check at top of `run_gh()` (~20 lines)
   - On success of a `pr review` command, record in context (~5 lines)

3. `crates/mika-agent/src/tools/mod.rs` (or wherever `ToolContext` is defined)
   - Add `pr_reviews_posted: Arc<Mutex<HashSet<String>>>` field to `ToolContext`

4. All `ToolContext` construction sites (agent.rs, team engine, silent mode, tests)
   - Initialize the new field with `Arc::new(Mutex::new(HashSet::new()))`

5. `skills/bundled/qa-review/system_prompt.md`
   - Update line 245 idempotency rule

6. `crates/mika-agent/tests/eval/` — New eval test
   - Mock qa-review turn where LLM tries to post two reviews
   - Verify second call returns error and turn ends cleanly

## Risks & Mitigations

- **False positive on Layer 1:** Could an agent legitimately say "review" and "pr" in `run_gh` input without it being a `pr review`? The check requires both keywords in the `input_summary` AND `name == "run_gh"` AND `success == true`. The args array for `gh pr review` always starts with `["pr", "review", ...]`. False positives from e.g. `gh pr list` (which has "pr" but not "review") are eliminated.
- **ToolContext lifetime:** The `Arc<Mutex<HashSet>>` is per-turn, created in `run_agent()`. No cross-turn leakage.
- **Team/delegate agents:** They also get `ToolContext` with this field. If a delegate posts a PR review, it correctly deduplicates within that delegate's turn.

## Test Plan

1. Unit test for `has_successful_pr_review()` — positive and negative cases
2. Unit test for `run_gh` dedup — mock the set, verify error returned
3. Eval harness integration test — full agent loop with `MockLlmProvider` that emits two `run_gh pr review` calls across steps; verify only one executes

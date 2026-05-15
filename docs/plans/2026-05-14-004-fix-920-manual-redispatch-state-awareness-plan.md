---
type: fix
module: mika-agent/skills/executor + self-dev
tags: [dispatch, task-state, engine-guard, re-dispatch, iteration-context]
issue: 920
companion_to: 919
---

# Plan: Manual Re-Dispatch State Awareness (mika#920)

## Problem

When the operator manually re-dispatches a ticket via `mika ask --agent mika-dev "<issue-ref>"` against a task that already exists with `status=in_progress` and an open PR with known blockers (QA verdict, CI failure, sibling-ticket dependency), mika-dev fires `run_claude_pilot` with a terse `{"prompt":"mika#N","skill":"dev-pilot","task_id":"..."}` — no state check, no context enrichment. The receiver re-runs `/mika` and mostly no-ops on a fully-groomed branch.

The autonomous retry paths (`verdict_handler`, `ci_failure_handler`) already handle this correctly with pre-digested context, retry counters, and circuit breakers. The manual path was never extended with the same state-awareness.

## Design Decision

**Engine-level guard at `validate_dispatch_readiness()`** (Guard #7), positioned after the per-class slot guard (#3) and before the grooming-marker check (#5). Same structural pattern as existing guards — machine-checkable, can't drift. Per `feedback_prompt_enforcement_fragile.md`, engine-level gates are preferred over skill-prompt rules.

The guard does NOT reject dispatch outright — it enriches the rejection with a structured state summary so the LLM can surface it to the operator and ask for confirmation. The operator can bypass via `iteration_context` (explicit re-dispatch with context) on the `run_claude_pilot` call.

## Implementation Units

### Unit 1: Engine guard — `dispatch_task_has_open_pr` check in `validate_dispatch_readiness()`

**File:** `crates/mika-agent/src/skills/executor.rs`

**Position:** After Guard #4 (per-turn dispatch counter) and before Guard #5 (grooming-marker check). This ordering is intentional: the state-awareness check is a DB + GitHub API hybrid (moderate cost), cheaper than the grooming-marker check (which fetches the full issue body) but more expensive than the pure-DB guards (#1–#4).

**Logic:**

```
1. Skip if iteration_context is present in tool_input (explicit re-dispatch — operator knows what they're doing)
2. Skip if skill != "dev-pilot" (dev-groom dispatches are fresh grooming, not re-dispatch)
3. Check task.metadata for $.claude_pilot.pr_url
4. If pr_url is present:
   a. Fetch PR state via gh REST API: state, latest reviews, mergeStateStatus
   b. Build a structured rejection with:
      - error: "dispatch_task_has_open_pr"
      - pr_number, pr_state, pr_url
      - latest_qa_verdict (if any review from mika-qa bot exists)
      - iteration_context_hint: instruction to re-dispatch with iteration_context
   c. record_dispatch_rejection() and return Err
5. If no pr_url: proceed (fresh dispatch or PR not yet created)
```

**Bypass conditions (any one skips the guard):**
- `iteration_context` field is present in tool_input (explicit re-dispatch)
- Skill is not `dev-pilot` (grooming dispatches don't have this problem)
- Task has no `$.claude_pilot.pr_url` in metadata (no prior PR to conflict with)

**Helper function:** `extract_iteration_context(tool_input: Option<&serde_json::Value>) -> Option<&str>` — extracts `iteration_context` from the tool input JSON.

**Helper function:** `fetch_pr_summary(token: &str, owner: &str, repo: &str, pr_number: u64) -> Result<PrSummary>` — fetches PR state, latest reviews, and merge status via REST API. Returns a struct:
```rust
struct PrSummary {
    state: String,         // "open", "closed", "merged"
    latest_verdict: Option<String>,  // parsed from mika-qa review body
    merge_state: Option<String>,     // "clean", "blocked", "behind", etc.
}
```

**PR number extraction:** Parse from `pr_url` using the existing URL pattern (the URL is `https://github.com/{owner}/{repo}/pull/{number}`). Reuse the pattern already in `parse_github_ref()` or extract with a simple regex.

### Unit 2: Structured rejection message for LLM consumption

**File:** `crates/mika-agent/src/skills/executor.rs`

The rejection JSON must be actionable for the self-dev skill prompt — it tells the LLM exactly what state the task is in and what options the operator has:

```json
{
  "error": "dispatch_task_has_open_pr",
  "task_id": "<uuid>",
  "pr_url": "https://github.com/senara-solutions/mika/pull/915",
  "pr_number": 915,
  "pr_state": "open",
  "latest_qa_verdict": "block[ac]",
  "merge_state": "blocked",
  "recovery": "This task already has an open PR. Options: (a) re-dispatch with iteration_context to address specific feedback, (b) wait for the blocker to resolve, (c) check PR status manually. To bypass: pass iteration_context in the run_claude_pilot call.",
  "reason": "Task has an open PR (#915) with QA verdict 'block[ac]'. Re-dispatching without iteration_context would re-run the full pipeline against a mostly-complete branch — likely a no-op."
}
```

### Unit 3: Self-dev prompt update — surface state-awareness rejection

**File:** `skills/bundled/self-dev/system_prompt.md`

Add a defense-in-depth section after Step 2 (Track the task) that instructs the LLM to check state before dispatching. This is prompt-level (can drift), but the engine guard is the primary defense.

Add to the "Rules" section under Step 3:

```
- **State-awareness on re-dispatch:** If `run_claude_pilot` returns `dispatch_task_has_open_pr`,
  surface the state summary to the operator via `send_message` and wait for explicit instructions.
  Do NOT retry without the operator's explicit go-ahead. Include the PR number, QA verdict,
  and suggested options (iterate with context, wait for blocker, skip).
```

This is ~3 lines of prompt text. The engine guard does the heavy lifting; this just tells the LLM how to handle the structured rejection.

### Unit 4: Eval test — dispatch_task_has_open_pr guard

**File:** `crates/mika-agent/tests/eval/test_dispatch_task_has_open_pr_guard.rs`

Three scenarios using `EvalHarness` + `MockLlmProvider`:

1. **Re-dispatch with open PR and no iteration_context → rejection**
   - Set up: create task with `pr_url` in metadata, status `in_progress`
   - Assert: `run_claude_pilot` returns `dispatch_task_has_open_pr` error
   - Assert: `tasks.result` contains the rejection JSON

2. **Re-dispatch with open PR AND iteration_context → allowed**
   - Set up: same task, but pass `iteration_context: "Fix the failing test"`
   - Assert: dispatch proceeds (no rejection from this guard; may hit other guards)

3. **Fresh dispatch (no pr_url in metadata) → allowed**
   - Set up: create task with no `pr_url`, status `in_progress`
   - Assert: dispatch proceeds past this guard

**GitHub API mocking:** The eval harness doesn't have a live GitHub token in CI. The guard should fail-open when no GitHub token is available (consistent with the grooming-marker and blocked-by guards). The test verifies the DB-level check (pr_url presence) without needing the API call by confirming the guard fires the rejection before the API call when pr_url is extractable from metadata.

Actually, re-reading the guard logic: the guard checks `task.metadata` for `pr_url` first (pure DB, no API), then optionally enriches with PR state from the API. The core rejection decision is based on the DB metadata alone — the API call adds detail. So the test can verify the rejection fires with just the DB fixture, and the API enrichment is a best-effort addition that degrades gracefully.

### Unit 5: Acceptance criteria verification

Per the ticket's AC:

- [x] AC1: When `run_claude_pilot` is called with an existing `in_progress` task that has `pr_url` metadata → guard rejects with state summary (Unit 1)
- [x] AC2: State summary includes PR number, state, QA verdict, branch, grooming status (Unit 2)
- [x] AC3: Behavioral test for rejection (Unit 4, scenario 1)
- [x] AC4: Behavioral test for iteration_context bypass (Unit 4, scenario 2)
- [x] AC5: Behavioral test for fresh dispatch passthrough (Unit 4, scenario 3)
- [x] AC6: Operator escape hatch documented — iteration_context field on run_claude_pilot input (Unit 3 prompt, Unit 2 recovery message)

## Sequencing

The ticket says "should ship after mika#919" since both touch `validate_dispatch_readiness()`. Check whether #919 is merged:
- If merged: branch from main, add Guard #7 after Guard #5 (grooming-marker)
- If open: this plan is designed to be position-independent within the guard chain. The guard uses a new error key (`dispatch_task_has_open_pr`) that doesn't conflict with #919's `dispatch_no_grooming_marker`.

The guard's position (after #4, before #5) is chosen for cost ordering, but it works correctly at any position in the chain because all guards are independent checks.

## Out of scope

- Changing the autonomous retry path (verdict_handler, ci_failure_handler) — those are correct as-is
- `--force-redispatch` CLI flag on `mika ask` — the `iteration_context` field on `run_claude_pilot` serves as the bypass mechanism; a CLI flag adds no value since the operator doesn't control the `run_claude_pilot` call directly
- Early-exit detection in claude-pilot when the branch is fully groomed — separate optimization

## Risk assessment

**Low risk.** The guard is a new check in an existing chain of 7 checks, following the identical pattern (structured JSON rejection, `record_dispatch_rejection`, fail-open on missing token). The bypass via `iteration_context` ensures no operator workflow is blocked. The eval tests cover the three critical paths.

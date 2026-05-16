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

**Engine-level guard at `validate_dispatch_readiness()`**, positioned **after** `global_dispatch_active` (per-class slot, #1001) and **before** `dispatch_no_grooming_marker` (#919, merged via PR #1101). Same structural pattern as existing guards — machine-checkable, can't drift. Per `feedback_prompt_enforcement_fragile.md`, engine-level gates are preferred over skill-prompt rules.

The guard does NOT reject dispatch outright in all cases — it rejects only operator-initiated dispatches that lack state-awareness, and lets engine-initiated recovery paths and webhook-driven positive-consent triggers through untouched. The operator can bypass via `iteration_context` (explicit re-dispatch with context) on the `run_claude_pilot` call.

## Phase 0 — Pin

All references below are taken on branch `fix/920/self-dev-agent-manual-re-dispatch-via` (which tracks `main`'s post-#1101 state; main HEAD `73db5fad`, #1101 merged at `a542e456`). Line numbers are anchored to this branch's commit `6990da0e`; reread before editing.

### Anchor A — `validate_dispatch_readiness()` insertion point

The new guard is inserted between the end of the per-class slot guard and the `let github_ref = ...` hoist. Verbatim slice (executor.rs:978-1006) — the new guard goes after line 989 (the close of the per-class slot guard's match arm) and before line 991 (the github_ref hoist comment):

```rust
        Ok(None) => { /* No conflicting dispatch in this class — proceed */ }
        Err(e) => {
            // Fail-closed: if we can't check global state, reject dispatch
            return Err(serde_json::json!({
                "error": "dispatch_check_failed",
                "task_id": task_id,
                "reason": format!("Failed to check global dispatch state: {e}")
            })
            .to_string());
        }
    }

    // <<< NEW GUARD INSERTS HERE — before the github_ref hoist >>>

    // Hoist the GitHub ref parse above both the grooming-marker check (#919)
    // and the blocked-by check (#713) so they share the binding.
    let github_ref = task.reference_url.as_deref().and_then(parse_github_ref);
```

Insertion rationale: the new guard does not need `github_ref` (it operates on `task.metadata.claude_pilot.pr_url`), but a future revision might. Inserting before the hoist keeps the option open without re-binding.

### Anchor B — `record_dispatch_rejection()` signature

The structured-rejection write-through helper exists at executor.rs:810-823. Verbatim:

```rust
/// Best-effort write of a dispatch-rejection reason to `tasks.result` (#1108).
///
/// Fire-and-forget: logs a warning on failure but never propagates the error.
/// This surfaces rejection reasons to operator-visible surfaces (`mika tasks list`,
/// dashboard task detail) without requiring DB-level inspection.
async fn record_dispatch_rejection(db: &AsyncDatabase, task_id: &str, reason_json: &str) {
    if let Err(e) = db.write_task_dispatch_rejection(task_id, reason_json).await {
        warn!(
            task_id = task_id,
            error = %e,
            "failed to write dispatch-rejection reason to tasks.result"
        );
    }
}
```

The new guard calls `record_dispatch_rejection(db, task_id, &rejection.to_string()).await` on each rejection path, identical to the existing four guards (lines 859, 899, 926, 976, 1061, 1116).

### Anchor C — `extract_pr_url()` already exists

The PR URL extraction helper the prior draft proposed to "add" already exists at executor.rs:1356-1375. Verbatim:

```rust
fn extract_pr_url(metadata: &Option<String>) -> Option<String> {
    let meta = metadata.as_deref()?;
    let parsed: serde_json::Value = serde_json::from_str(meta).ok()?;

    // Try nested claude_pilot.pr_url first
    if let Some(url) = parsed
        .get("claude_pilot")
        .and_then(|cp| cp.get("pr_url"))
        .and_then(|v| v.as_str())
    {
        return Some(url.to_string());
    }

    // Fallback to top-level pr_url
    parsed
        .get("pr_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
```

The new guard **reuses** this helper — no new helper to add. It's already called from the existing `task_not_dispatchable` and `task_active_dispatch` rejection sites.

### Anchor D — `claude_pilot.pr_url` provenance

The `pr_url` field under `claude_pilot` is written by `extract_callback_fields()` at `crates/mika-agent/src/task_engine/dispatcher.rs:1368-1421` via regex `(?m)^PR:\s+(https?://github\.com/\S+)` against claude-pilot subprocess stdout. The emitter is the shared `skills/bundled/_shared/dispatch-lib.sh` (mika#871 R4 contract). The guard's correctness depends on this contract: any change to the `^PR:` emission discipline downstream forces a corresponding update to either `extract_callback_fields()` (preferred) or this guard's pr_url extraction site.

### Anchor E — `register_deferred_callback()` for F3 resolution

Existing function at executor.rs:1277-1340 already preserves the original tool input:

```rust
let action_config = serde_json::json!({
    "trigger_kind": "deferred_dispatch",
    "original_call": input,
})
.to_string();
```

When `DeferredDispatch` fires, the engine replays `original_call` as the tool input. F3's fix injects a sentinel into the saved `original_call` at registration time (see Unit 1 § Bypass condition for DeferredDispatch).

## Implementation Units

### Unit 1: Engine guard — `dispatch_task_has_open_pr` check in `validate_dispatch_readiness()`

**File:** `crates/mika-agent/src/skills/executor.rs`

**Position:** Inserted at the location described in Anchor A — after the per-class slot guard, before the `github_ref` hoist.

**Logic:**

```
1. Skip if any bypass condition is satisfied (see Bypass below).
2. Read pr_url via the existing extract_pr_url(&task.metadata) helper (Anchor C).
3. If pr_url is None: proceed (fresh dispatch or PR not yet created).
4. If pr_url is Some:
   a. (Optional, best-effort) Fetch PR state via GitHub REST: state, latest mika-qa
      review, mergeStateStatus. On API failure: log warn, omit enriched fields,
      still reject — the DB-level pr_url presence is sufficient signal.
   b. Build a structured rejection with:
      - error: "dispatch_task_has_open_pr"
      - pr_url, pr_number (parsed from URL), pr_state (best-effort), latest_qa_verdict
        (best-effort), merge_state (best-effort)
      - recovery: instruction text on how to bypass via iteration_context
      - reason: human-readable explanation
   c. record_dispatch_rejection() and return Err.
```

**Bypass conditions (any one skips the guard, evaluated in order):**

1. **`iteration_context` field is present in tool_input** — explicit re-dispatch path. Used by autonomous handlers (`verdict_handler`, `ci_failure_handler`) and manual operator-with-context dispatches.
2. **Skill is not `dev-pilot`** — `dev-groom` dispatches are fresh grooming, not implementation re-runs.
3. **Task has no `claude_pilot.pr_url` in metadata** — fresh-dispatch path, no prior PR to conflict with.
4. **(F2) `originating_message` matches the ready-label webhook signature** — `Some(msg)` where `msg.contains("[GitHub] Issue labeled ready on")`. This is the operator's positive-consent signal per mika#841; re-applying `ready` on an issue with an open PR is explicitly "go work on this again." Blocking this would fight the autonomous-loop dispatch contract.
5. **(F3) Calling turn is `SilentTrigger::DeferredDispatch`** — engine-initiated recovery from a prior `global_dispatch_active` rejection (#1011/#1058). The deferred turn replays the original tool_input; blocking it would create a livelock (deferred fires → guard rejects → engine re-defers).

**F3 implementation mechanic.** `SilentTrigger::DeferredDispatch` produces `originating_message: None` in `LongRunningContext`, but `None` alone is not a sufficient signal — every silent trigger (callback, heartbeat, reflection) has `originating_message: None`, and only DeferredDispatch is structurally allowed to call `run_claude_pilot` (the executor's `long_running_ctx == None` rejection blocks the others except via deferred re-dispatch). The guard detects DeferredDispatch by adding a sentinel `__internal_deferred_dispatch: true` field to the saved `original_call` inside `register_deferred_callback()` (Anchor E). The guard then checks this sentinel on the inbound `tool_input` and bypasses when present. Rationale for sentinel-on-input rather than a new `LongRunningContext` field: the bypass surface stays uniform (`tool_input` JSON), one place to inspect, no API-surface expansion.

The sentinel is namespaced (`__internal_*`) to mark it as engine-internal and not part of the public tool schema. The dev-pilot skill's `tools.json` should not advertise this field — it is a transparent engine pass-through.

**Helper function:** `fetch_pr_summary(token: &str, owner: &str, repo: &str, pr_number: u64) -> Result<PrSummary>` — fetches PR state, latest reviews, and merge status via REST API. Returns a struct:

```rust
struct PrSummary {
    state: String,         // "open", "closed", "merged"
    latest_verdict: Option<String>,  // parsed from mika-qa review body
    merge_state: Option<String>,     // "clean", "blocked", "behind", etc.
}
```

API enrichment is best-effort. The core rejection decision is based on `pr_url` presence in DB metadata alone — the API call only adds detail for the structured-rejection body. On API failure: log a `warn!`, omit the enriched fields, still reject.

**PR number extraction:** Parse from `pr_url` using `https://github.com/{owner}/{repo}/pull/{number}` shape. The existing `extract_pr_url` in `crates/mika-agent/src/skills/context.rs:196` already does this — reuse it via `pub use` re-export, or inline a simple regex if the visibility lift is awkward.

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

Optional fields (`pr_state`, `latest_qa_verdict`, `merge_state`) are populated when the GitHub REST enrichment succeeds; omitted otherwise. The `recovery` and `reason` strings remain stable so the LLM's prompt can pattern-match on them.

### Unit 3: Self-dev prompt update — surface state-awareness rejection

**File:** `skills/bundled/self-dev/system_prompt.md`

Add a defense-in-depth section after Step 2 (Track the task) that instructs the LLM to surface the rejection. This is prompt-level (can drift per `feedback_prompt_enforcement_fragile.md`); the engine guard is the primary defense.

Add to the "Rules" section under Step 3:

```
- **State-awareness on re-dispatch (engine guard — see executor.rs):**
  If `run_claude_pilot` returns `dispatch_task_has_open_pr`, surface the state
  summary to the operator via `send_message` and wait for explicit instructions.
  Do NOT retry without the operator's explicit go-ahead. Include the PR number,
  QA verdict, and suggested options (iterate with context, wait for blocker, skip).
  The engine guard in `validate_dispatch_readiness()` is the authoritative
  enforcement point; this rule is defense-in-depth.
```

~5 lines of prompt text including the engine-guard cross-reference (per NF3 recommendation).

### Unit 4: Eval test — `dispatch_task_has_open_pr` guard

**File:** `crates/mika-agent/tests/eval/test_dispatch_task_has_open_pr_guard.rs`

Five scenarios using `EvalHarness` + `MockLlmProvider` (originals 1–3 plus F2/F3 regression coverage):

1. **Re-dispatch with open PR and no `iteration_context` → rejection** (primary positive case)
   - Set up: create task with `claude_pilot.pr_url` in metadata, status `in_progress`
   - Assert: `run_claude_pilot` returns `dispatch_task_has_open_pr` error
   - Assert: `tasks.result` contains the rejection JSON

2. **Re-dispatch with open PR AND `iteration_context` → allowed**
   - Set up: same task, but pass `iteration_context: "Fix the failing test"`
   - Assert: dispatch proceeds (no rejection from this guard; may hit other guards)

3. **Fresh dispatch (no `pr_url` in metadata) → allowed**
   - Set up: create task with no `pr_url`, status `in_progress`
   - Assert: dispatch proceeds past this guard

4. **(F2 regression) Ready-label webhook with open PR → allowed**
   - Set up: task with `pr_url` in metadata; `originating_message` populated with
     `[GitHub] Issue labeled ready on senara-solutions/mika#920`
   - Assert: dispatch proceeds (operator positive-consent bypass active)

5. **(F3 regression) DeferredDispatch with open PR → allowed**
   - Set up: task with `pr_url` in metadata; tool_input includes
     `__internal_deferred_dispatch: true` (simulating replay of `original_call`)
   - Assert: dispatch proceeds (engine-initiated recovery bypass active)

**GitHub API mocking:** The eval harness doesn't have a live GitHub token in CI. The guard's core decision (rejection on DB-level `pr_url` presence) is independent of the API enrichment. Tests verify the DB-level rejection fires without requiring the API call; API enrichment is best-effort and degrades gracefully when no token is configured (log `warn!`, omit enriched fields, still reject).

### Unit 5: Acceptance criteria verification

Per the ticket's AC:

- [ ] AC1: When `run_claude_pilot` is called with an existing `in_progress` task that has `pr_url` metadata → guard rejects with state summary (Unit 1)
- [ ] AC2: State summary includes PR number, state, QA verdict, branch, grooming status (Unit 2)
- [ ] AC3: Behavioral test for rejection (Unit 4, scenario 1)
- [ ] AC4: Behavioral test for `iteration_context` bypass (Unit 4, scenario 2)
- [ ] AC5: Behavioral test for fresh dispatch passthrough (Unit 4, scenario 3)
- [ ] AC6: Operator escape hatch documented — `iteration_context` field on `run_claude_pilot` input (Unit 3 prompt, Unit 2 recovery message)

AC checkboxes are unchecked — they are verification targets, not pre-claimed results. They flip to `[x]` only on the implementation PR after each scenario passes.

## Operator bypass UX (per NF4)

The bypass surface is the `iteration_context` field on the `run_claude_pilot` tool input. The operator does not call this tool directly — the LLM does, on the operator's behalf. So the operator's natural-language prompt must push the LLM to include the field.

**Example operator prompts that trigger the bypass:**

> "Re-dispatch mika#908 with iteration context: address the qa block[ac] on Unit 5 — the sibling ticket #918 just merged so the AC dependency is unblocked."

> "Force re-run mika#908: the prior pipeline run wedged on a network blip, retry."

The LLM should map these natural-language re-dispatch requests to `run_claude_pilot` calls with an `iteration_context` field populated from the operator's reasoning. The self-dev skill prompt (Unit 3) calls this out as the canonical bypass shape.

No CLI flag is added — the LLM tool-call boundary is the right gate point, and adding `--force-redispatch` to `mika ask` would require plumbing through `mika ask` → agent session → tool_input layers without solving any new failure class (the bypass already exists; only its discoverability is the question, and that is a prompt/documentation concern, not an API one).

## Guard ordering rationale (per NF1)

The new guard sits after `global_dispatch_active` and before `dispatch_no_grooming_marker`. The ordering is **scope-narrowness, not cost**:

- `dispatch_task_has_open_pr` fires only for `dev-pilot` skill + existing `claude_pilot.pr_url` in metadata. This is a narrower filter than grooming-marker (which fires for all `dev-pilot` dispatches on issue-typed tasks).
- Both guards make one GitHub REST call when active (issue body fetch vs PR/reviews fetch); latency cost is comparable.
- Narrower-before-broader is acceptable here because the guards are independent (no shared API call) and rejecting on either path is correct.

Reordering is safe if empirical rejection rates favor it. No reordering required by this plan.

## Sequencing

This ticket builds on top of **merged** #919 (PR #1101). The new guard's position (between `global_dispatch_active` and `dispatch_no_grooming_marker`) is stable on current main. No reordering of existing guards required.

## Out of scope

- Changing the autonomous retry path (`verdict_handler`, `ci_failure_handler`) — those are correct as-is
- `--force-redispatch` CLI flag on `mika ask` — `iteration_context` field on `run_claude_pilot` is the bypass mechanism; CLI flag adds no value
- Early-exit detection in claude-pilot when the branch is fully groomed — separate optimization
- Environment-aware (prod vs dev) fail-open semantics for missing GitHub token — accept the existing fail-open pattern for consistency with sibling guards (#919 grooming-marker, #713 blocked-by). If the token-absent hole becomes a real problem, address it as a cross-cutting concern for all guards in a separate ticket.

## Risk assessment

**Low risk.** The guard is a new check in an existing chain of 5 dispatch-readiness guards, following the identical structural pattern (DB-first decision, optional API enrichment, `record_dispatch_rejection()` write-through, fail-open on missing token, `dispatch_check_failed` on API error). Five bypass conditions ensure no operator workflow is blocked (manual with context, dev-groom, fresh dispatch, ready-label webhook, deferred-dispatch recovery). Five eval scenarios cover the rejection path and four bypass paths.

**Coupling surface.** The guard depends on the `claude_pilot.pr_url` provenance contract (Anchor D). Any downstream change to claude-pilot's `^PR:` emission discipline must update `extract_callback_fields()` or this guard's pr_url read site in lockstep.

## Architect review trail

- 2026-05-16 first-pass (session `0a98390f-cc65-44ff-aed5-83bf37b7de28`): Disposition ITERATE.
  - F1 (Phase 0 Pin absent) — addressed via new Phase 0 § Anchors A–E.
  - F2 (ready-label webhook interaction) — addressed via bypass condition #4.
  - F3 (DeferredDispatch livelock) — addressed via bypass condition #5 + `__internal_deferred_dispatch` sentinel in `register_deferred_callback`.
  - NF1 (guard ordering rationale) — folded into § "Guard ordering rationale."
  - NF2 (fail-open on missing token) — accepted per architect; added to § "Out of scope."
  - NF3 (Unit 3 prompt code-comment) — folded into Unit 3 with engine-guard cross-reference.
  - NF4 (operator bypass UX example prompts) — folded into § "Operator bypass UX."

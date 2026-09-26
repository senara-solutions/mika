---
title: "feat: mika-dev auto-retry on fixable QA blocks"
type: feat
status: completed
date: 2026-04-02
issue: "#377"
repos: [mika-skills]
---

# feat: mika-dev auto-retry on fixable QA blocks

## Overview

When mika-qa returns a `block` verdict with a fixable finding (e.g., "CI check failed: docs-sync"), mika-dev currently just reports the finding and stops. The work item stays `in_progress` with no follow-up, requiring manual intervention from Vincent.

This feature adds block verdict classification and auto-retry to the self-dev skill, mirroring the existing `hold[review]` auto-fix pattern. Fixable CI failures (docs sync, lint, fmt, clippy) are automatically dispatched to claude-pilot for repair; non-fixable issues (security, semantic, merge conflicts) escalate as before.

## Problem Statement

From the #375 incident analysis: mika-dev hit max_steps during a callback where QA returned `BLOCK` for a Docs Sync CI failure. Even with the max_steps fix (#375, now merged), mika-dev has no logic to classify and retry fixable blocks. Every `block` verdict pauses the sprint and requires Vincent to respond with `continue`, `skip`, or `merge anyway`.

Many block verdicts are mechanically fixable:
- **Docs Sync CI failure** -- run `scripts/sync-agent-docs.sh`, commit, push
- **Clippy/lint failure** -- run `cargo clippy --fix`, commit, push
- **Format failure** -- run `cargo fmt`, commit, push

These should not require human intervention.

## Proposed Solution

### Two-layer approach

1. **QA review skill** (`mika-skills/qa-review/`): Add `block[ci]` sub-type for CI failures so classification is structured, not heuristic
2. **Self-dev skill** (`mika-skills/self-dev/`): Add block classification and auto-retry logic, mirroring the existing `hold[review]` retry pattern

### QA review changes (`mika-skills/qa-review/system_prompt.md`)

Extend the verdict taxonomy with block sub-types:

| Current verdict | New verdict | When |
|-----------------|-------------|------|
| `block` (CI failure) | `block[ci]` | Any GitHub CI check failed |
| `block` (security) | `block[security]` | Hardcoded secrets, SQL injection, eval/exec |
| `block` (pipeline) | `block[pipeline]` | Missing plan doc, no source changes |
| `block` (bare, legacy) | `block` | Fallback -- self-dev treats as non-fixable |

Changes needed in QA review prompt:
- Step 2 CI status: change `block` to `block[ci]` with reason "CI check failed: {check_name}"
- Step 3b security checks: change `block` to `block[security]`
- Step 2 pipeline checks (plan doc, source changes): change `block` to `block[pipeline]`
- Verdict Output section: document new sub-types
- Verdict rules: add sub-type descriptions

### Self-dev skill changes (`mika-skills/self-dev/system_prompt.md`)

#### Verdict parsing update

Update Step 5 verdict parsing to handle block sub-types:
- Parse `block[ci]`, `block[security]`, `block[pipeline]` like `hold[ci_pending]`, `hold[review]`
- Bare `block` (no sub-type): treat as non-fixable (backward compatible)
- Unknown block sub-types: treat as non-fixable

#### Block classification and retry logic

Replace the current monolithic `block` handler with a classification step:

```
**block[ci]** -- Fixable CI failure; attempt auto-fix:

1. **Check retry budget.** Call `check_work_item` with the `task_id`. Read metadata
   for `block_retry_count` (default 0). If `block_retry_count >= 2`, skip retry --
   go to escalation (step 5 below).

2. **Notify Vincent:** "PR {repo}#{number} blocked by CI -- attempting auto-fix
   (retry {n}/2). {PR URL}" (where {n} = current `block_retry_count` + 1).

3. **Launch fix attempt.** Extract the `FINDINGS:` and `REASON:` lines from
   mika-qa's response. Call `run_claude_pilot` with a free-text prompt:

   Fix the following CI failure on branch <branch> in <repo>:

   <FINDINGS text>
   <REASON text>

   The PR is at <PR URL>. Working directory: <worktree path>.
   Push fix commits to the same branch. Do NOT create a new PR.
   Run the failing CI check locally after fixing to verify.

   Pass `task_id` from the current work item.

4. **Handle fix result.**
   - Update retry metadata: call `update_work_item_status` with `status: "in_progress"`
     and `metadata: {"block_retry_count": <current + 1>}`. On first retry, also include
     `"block_original_findings": "<findings text>"`.
   - **Verify persistence:** call `check_work_item` and confirm metadata updated.
     If not persisted, escalate immediately.
   - If claude-pilot **succeeded** (exit 0):
     - Re-run build check via `build_mika`.
     - If build passes: re-delegate to mika-qa (go back to Step 5 delegation).
       On re-review verdict:
       - `pass`: follow existing pass logic
       - `block[ci]` again: loop back to step 1 (check retry budget)
       - `block[security]`/`block[pipeline]`: follow non-fixable block logic
       - `hold[review]`/`hold[ci_pending]`: follow existing hold logic
     - If build fails: treat as fix failure, loop back to step 1.
   - If claude-pilot **failed** (non-zero exit): loop back to step 1
     or escalation if budget exhausted.

5. **Escalate (max retries exhausted).**
   - Notify Vincent: "PR {repo}#{number} blocked after {n} CI fix attempts.
     Original: {original findings}. Latest: {latest findings}. {PR URL}"
   - Leave the PR open
   - **PAUSE the sprint** (same as current block behavior)

**block[security]**, **block[pipeline]**, **block** (bare) -- Non-fixable:

- Current block behavior unchanged: extract REASON/FINDINGS, notify Vincent,
  pause sprint, wait for instruction
```

#### Notification templates update

Add to the User Notifications section:

```
- **After QA review -- block[ci] (retry starting):** "PR {repo}#{number} blocked by CI --
  attempting auto-fix (retry {n}/2). {PR URL}"
- **After QA review -- block[ci] (retries exhausted):** "PR {repo}#{number} blocked after
  {n} CI fix attempts. Original: {original}. Latest: {latest}. {PR URL}"
```

#### Step 4.5 exception update

Add `block[ci]` fix attempts to the existing exception for QA fix attempts:
```
> **Exception:** If this claude-pilot invocation is a QA fix attempt (launched from
> Step 5 hold retry OR block[ci] retry), do NOT apply Step 4.5 recovery.
```

#### Close-out metadata update (Step 6)

Add block retry fields to the metadata schema:

```json
{
  "metadata": {
    "block_retry_count": 0,
    "block_original_findings": "..."
  }
}
```

Add new outcome rows to the Step 6 table:

| Outcome | Status | Note |
|---------|--------|------|
| `block[ci]` -> retry -> `pass` + merged | `completed` | "CI blocked, auto-fixed (retry {n}), merged. PR: {url}" |
| `block[ci]` -> retry -> `pass` + not merged | remain `in_progress` | "CI blocked, auto-fixed (retry {n}), PR open. PR: {url}" |
| `block[ci]` -> retry exhausted | `blocked` | "CI blocked after {n} fix attempts. PR: {url}" |
| `block[ci]` -> retry -> `block[security]` | `blocked` | "CI fix introduced security block: {reason}. PR: {url}" |
| `block[security]`/`block[pipeline]`/`block` | `blocked` | Current behavior unchanged |

## System-Wide Impact

- **Interaction graph:** block[ci] retry triggers: check_work_item -> update_work_item_status -> run_claude_pilot (callback) -> build_mika -> delegate_task (mika-qa). Same chain as hold[review] retry.
- **Error propagation:** claude-pilot failure during block retry counts against retry budget (same as hold[review]). Metadata persistence failure short-circuits to escalation.
- **State lifecycle risks:** No new state risks. Uses existing work item metadata pattern. `block_retry_count` is independent of `qa_retry_count` -- a PR could exhaust both budgets across different QA cycles.
- **API surface parity:** No Rust API changes. Purely prompt-level behavior in skill files.

## Acceptance Criteria

### Functional Requirements

- [ ] QA review skill emits `block[ci]`, `block[security]`, `block[pipeline]` sub-types (`mika-skills/qa-review/system_prompt.md`)
- [ ] Self-dev skill parses block sub-types in Step 5 verdict parsing (`mika-skills/self-dev/system_prompt.md`)
- [ ] `block[ci]` triggers auto-retry with claude-pilot dispatch
- [ ] `block_retry_count` tracked in work item metadata, capped at 2
- [ ] `block_original_findings` preserved for escalation message
- [ ] Metadata persistence verified via `check_work_item` after each update
- [ ] `block[security]`, `block[pipeline]`, bare `block` retain current escalation behavior
- [ ] Notification templates updated for block[ci] retry/exhaustion
- [ ] Step 4.5 exception covers block[ci] fix attempts
- [ ] Step 6 close-out table includes block retry outcomes
- [ ] Sprint mode: block[ci] does NOT pause sprint during retry; pauses only on exhaustion or non-fixable block

### Testing

- [ ] Manual verification: trigger a docs-sync CI failure on a test PR and confirm mika-dev auto-retries
- [ ] Verify backward compatibility: bare `block` (from older QA skill versions) still escalates

## Dependencies & Risks

- **Depends on #375 (completed):** Callback turns now have 20 tool steps, sufficient for block retry chain
- **Cross-repo:** Changes are in `mika-skills/` (qa-review and self-dev skills). Issue is on `mika/` because it describes mika-dev agent behavior. Implementation PR goes on `mika-skills/`.
- **Risk: misclassification.** A CI failure classified as fixable might not be (e.g., a flaky test that consistently fails). Mitigation: 2-retry cap prevents infinite loops; escalation on exhaustion ensures human review.
- **Risk: block + hold interaction.** A block[ci] retry could result in a hold[review] on re-QA (e.g., the CI fix introduced a new code quality issue). This is handled: the re-QA verdict routes through existing hold/pass/block logic.
- **Risk: mixed findings.** QA could find both a CI failure and a security issue. QA's "most severe wins" rule means `block[security]` takes precedence, which correctly routes to non-fixable escalation.

## Implementation Files

| File | Repo | Change |
|------|------|--------|
| `qa-review/system_prompt.md` | mika-skills | Add block sub-types to verdict taxonomy |
| `self-dev/system_prompt.md` | mika-skills | Add block classification, retry logic, notifications, close-out outcomes |

## Sources & References

- Related issue: #375 (callback max_steps fix -- prerequisite, completed)
- Related plan: `docs/plans/2026-04-01-004-fix-callback-max-steps-exhaustion-plan.md`
- Existing pattern: `hold[review]` auto-fix retry in `mika-skills/self-dev/system_prompt.md` (lines 312-371)
- QA verdict taxonomy: `mika-skills/qa-review/system_prompt.md` (lines 238-253)

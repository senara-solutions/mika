---
title: "fix: Prevent wip-rescue draft PRs from bypassing qa recovery-skip guard"
date: 2026-06-28
sequence: 002
type: fix
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
origin: mika#1613
---

# fix: Prevent wip-rescue draft PRs from bypassing qa recovery-skip guard

## Goal Capsule

Close the gap where dispatch-lib's mika#1282 dirty-worktree rescue and mika#1396 commit-pushed-no-pr recovery paths open draft PRs without setting the `unpushed_recovery_pending` task metadata flag, allowing mika-qa to autonomously approve, un-draft, and auto-merge unreviewed code to main.

---

## Summary

When a dev-pilot session exits with a dirty worktree (or with commits pushed but no PR created), dispatch-lib rescues the content into a draft PR. The stated contract (mika#1282) is "operator must review and promote the draft PR." However, the rescue path never sets `unpushed_recovery_pending: true` in task metadata, so the qa-webhook's recovery-skip guard never fires. The standard qa-approve, un-draft, auto-merge flow treats the rescue PR like a normal pipeline-complete PR and ships unreviewed code to main.

The fix sets the recovery flag at rescue time (primary) and adds defense-in-depth guards in the qa-webhook for draft PRs and wip-commit signatures (secondary).

---

## Problem Frame

**Incident:** PR #1610 (mika#1609) — pilot died mid-pipeline before `/ce:review`; dispatch-lib's dirty-worktree rescue staged, committed with `wip()` prefix, pushed, and opened a draft PR. mika-qa approved it (while still draft), mika-dev un-drafted and armed auto-merge, and it merged to main — shipping unreviewed, broken code (a `make_interval` type mismatch that 500s on every call, plus a fail-open secret-rotation outage tracked in mika#1612).

**Root cause:** The `unpushed_recovery_pending` flag is set only by `self-dev-callback/system_prompt.md` on the `recover_unpushed_work` verdict (error_max_turns with unpushed local commits). The dispatch-lib rescue paths (dirty-worktree at lines 763-886, auto-PR-create at lines 2401-2472) open draft PRs and emit `PR: $URL` in the callback RESULT but never include a recovery marker. The qa-webhook guard at `self-dev-webhook-qa/system_prompt.md:199` checks `unpushed_recovery_pending` in task metadata before routing any verdict — without the flag, it processes the verdict normally.

---

## Requirements

- **R1.** A PR opened by dispatch-lib's dirty-worktree rescue or commit-pushed-no-pr recovery must not be autonomously un-drafted or auto-merged. The qa-webhook must skip verdict processing and escalate to the operator.
- **R2.** The recovery signal must be durable — prefer task-metadata flag set at rescue time, with defense-in-depth recognition of the `wip(` rescue-commit signature.
- **R3.** A draft PR (regardless of origin) must not be autonomously merged by the qa-webhook flow.
- **R4.** Regression coverage: a future rescue-path addition that forgets the flag should be caught by the defense-in-depth guards.

---

## Key Technical Decisions

**KTD1. Structured marker line in RESULT text, not a direct metadata write.** dispatch-lib communicates with mika-dev via the callback RESULT text (`mika ask --task-complete -- "$RESULT"`). It cannot write task metadata directly — that's an engine operation. The fix appends a `RECOVERY_PENDING: true` structured line to RESULT (same pattern as `PR: $URL`), and the callback handler parses it and writes the metadata flag. This preserves the existing callback contract and requires no new tools or APIs.

**KTD2. Universal pre-check in callback handler, not per-branch insertion.** The `RECOVERY_PENDING: true` marker is checked before the pipeline-failure/success routing split in the callback handler. This ensures the flag is set regardless of which code path the callback routes through — defense against future routing changes.

**KTD3. Draft-PR guard as independent defense-in-depth layer.** The qa-webhook already fetches PR metadata for mergeable-state checks. Adding an `isDraft` check is zero-cost and catches any future recovery path (or manual draft PR) that forgets to set the metadata flag. This is structurally independent from the metadata-flag guard.

---

## Scope Boundaries

### In scope

- dispatch-lib RESULT text: add `RECOVERY_PENDING: true` marker on rescue PR creation
- self-dev-callback prompt: parse the marker and set `unpushed_recovery_pending: true` in task metadata
- self-dev-webhook-qa prompt: add draft-PR and wip-commit defense-in-depth guards

### Out of scope

- The gateway code defects shipped by #1610 (tracked in mika#1612)
- Re-reviewing #1610's already-merged content
- Changes to the `recover_unpushed_work` verdict handler (it already works correctly)

### Deferred to Follow-Up Work

- Automated test harness for dispatch-lib shell functions (would enable AC3/AC4 as executable tests rather than prompt-level calibration rules)

---

## Implementation Units

### U1. Add `RECOVERY_PENDING: true` marker to dispatch-lib rescue RESULT

**Goal:** Make the callback text carry a structured signal that the PR was created by a recovery path, so the callback handler can set the task metadata flag.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh` (modify)

**Approach:** In the Unit 2 rescue-PR block (after line 2470 where `PR: $URL` is appended to RESULT), append an additional structured line: `RECOVERY_PENDING: true`. This line is appended inside the same `if [ -n "$RESCUED_PR_URL" ]` guard, so it only appears when a rescue PR was actually created. Both recovery classes (dirty-worktree and commit-pushed-no-pr) flow through this block, so both are covered by a single insertion point.

**Patterns to follow:** The existing `PR: ${PR_URL}` line pattern at dispatch-lib.sh:2470 — structured `KEY: VALUE` lines in RESULT that downstream handlers parse.

**Test scenarios:**
- Happy path: dirty-worktree rescue creates a draft PR — RESULT text contains `RECOVERY_PENDING: true` after the `PR:` line
- Happy path: commit-pushed-no-pr rescue creates a draft PR — RESULT text contains `RECOVERY_PENDING: true`
- Edge case: rescue PR creation fails (`gh pr create` returns empty) — `RECOVERY_PENDING: true` is NOT appended (guarded by `if [ -n "$RESCUED_PR_URL" ]`)
- Edge case: normal (non-rescue) pipeline completion — RESULT does NOT contain `RECOVERY_PENDING: true`

**Verification:** `grep -c 'RECOVERY_PENDING: true' skills/bundled/_shared/dispatch-lib.sh` returns 1. The marker is inside the `if [ -n "$RESCUED_PR_URL" ]` block.

---

### U2. Parse `RECOVERY_PENDING` in self-dev-callback and set task metadata flag

**Goal:** When the callback handler receives a RESULT containing `RECOVERY_PENDING: true`, it writes `unpushed_recovery_pending: true` to task metadata before any other processing. This reuses the existing qa-webhook guard with zero new guard surface.

**Requirements:** R1, R2

**Dependencies:** U1

**Files:**
- `skills/bundled/self-dev-callback/system_prompt.md` (modify)

**Approach:** Add a new section between the existing "Completion callback result" description (line 109) and the "On pipeline failure" routing (line 90). This section fires as a universal pre-check before the pipeline-failure/success/failure routing split:

> **Recovery-pending detection (universal pre-check, runs before pipeline-failure/success routing):**
>
> If the callback text contains the line `RECOVERY_PENDING: true`:
> 1. Write `unpushed_recovery_pending: true` to `tasks.metadata` via `update_task_status` (status stays `in_progress`). Atomicity: metadata write BEFORE any send_message or routing.
> 2. Log: "Recovery-pending flag set for task {task_id} — rescue PR will require operator review."
> 3. Continue to normal routing below. The flag does not change callback routing — it only ensures the qa-webhook will skip autonomous verdict processing when the PR is reviewed.

The key insight is that this pre-check does NOT short-circuit the callback routing. The callback still processes normally (extracts metadata, notifies Vincent about the PR, etc.). The flag's effect is downstream — when mika-qa reviews the rescue PR, the qa-webhook sees the flag and skips autonomous merge.

**Patterns to follow:** The existing `recover_unpushed_work` handler at lines 82-88 — same `update_task_status` call pattern, same atomicity discipline (metadata BEFORE send_message).

**Test scenarios:**
- Happy path: callback with `RECOVERY_PENDING: true` and `PR:` URL — flag is set in metadata, callback routes normally through pipeline-failure path, Vincent is notified about the rescue PR
- Happy path: callback without `RECOVERY_PENDING: true` (normal pipeline completion) — no flag set, normal routing
- Edge case: callback with `RECOVERY_PENDING: true` but no `PR:` URL (rescue PR creation failed) — flag is still set (defense-in-depth), pipeline-failure routing handles the missing PR normally

**Verification:** The prompt text contains the recovery-pending detection block positioned before the routing split. The `update_task_status` call matches the existing `recover_unpushed_work` handler pattern.

---

### U3. Add draft-PR and wip-commit defense-in-depth guards to qa-webhook

**Goal:** Add two independent secondary guards in the qa-webhook so that even if the metadata flag is missing, a rescue draft PR cannot be autonomously merged.

**Requirements:** R1, R3, R4

**Dependencies:** None (independent of U1/U2 — this is a defense-in-depth layer)

**Files:**
- `skills/bundled/self-dev-webhook-qa/system_prompt.md` (modify)

**Approach:** Extend the existing guard block at lines 199-204. After the current `unpushed_recovery_pending` metadata check, add two additional checks that fire with the same skip-all-verdict-processing behavior:

1. **Draft-PR guard:** Before routing any verdict, check if the PR is still a draft (`isDraft: true` from `gh pr view ... --json isDraft`). If the PR is a draft, skip all verdict processing — a draft PR should never be autonomously merged regardless of how it got there. Acknowledge the event with: "Draft PR detected — skipping autonomous verdict processing. Operator must review and promote."

2. **Wip-commit signature guard:** Check if the PR's head commit message starts with `wip(` (the mika#1282 rescue prefix). If so, skip all verdict processing. Acknowledge with: "Rescue commit signature detected (wip prefix) — skipping autonomous verdict processing. Operator must review and promote." The commit message can be obtained from the PR metadata already fetched in Step 3 (`gh pr view ... --json headRefOid` then `gh api repos/{owner}/{repo}/commits/{sha} --jq .commit.message`), or via `gh pr view ... --json commits --jq '.commits[-1].messageHeadline'`.

The three guards (metadata flag, draft status, wip-commit) are independent and OR-combined: any one of them firing is sufficient to skip verdict processing. This provides R4 regression coverage — a future rescue path that forgets the metadata flag is still caught by the draft-PR and/or wip-commit guards.

Add a calibration rule documenting this defense-in-depth:

> ### Rule N — Rescue draft PRs never auto-merge (mika#1613)
>
> Three independent guards prevent rescue draft PRs from autonomous merge:
> 1. `unpushed_recovery_pending: true` in task metadata (primary, set by callback handler)
> 2. `isDraft: true` on the PR (defense-in-depth, catches any draft PR)
> 3. Head commit message starts with `wip(` (defense-in-depth, catches rescue-commit signature)
>
> All three must be checked before routing any verdict. Any single guard firing skips all verdict processing and escalates to the operator. Incident: mika#1610 — rescue draft PR auto-merged unreviewed code because only the metadata-flag guard existed and the flag was never set.

**Patterns to follow:** The existing `unpushed_recovery_pending` guard at lines 199-204 — same skip-all-verdict-processing behavior, same escalation-to-operator messaging.

**Test scenarios:**
- Happy path: rescue draft PR with metadata flag set + isDraft true + wip commit — all three guards fire, verdict processing skipped
- Happy path: rescue draft PR with metadata flag NOT set (regression) + isDraft true — draft-PR guard catches it, verdict processing skipped
- Happy path: rescue PR that was manually un-drafted by operator but still has wip commit — wip-commit guard catches it until operator explicitly removes the wip commit (force-push with clean commit)
- Happy path: normal (non-rescue) PR, not draft, no wip commit, no recovery flag — all three guards pass, verdict processing proceeds normally
- Edge case: operator intentionally promotes a rescue draft PR (marks ready for review, pushes clean commit) — all three guards pass, autonomous flow proceeds correctly
- Edge case: PR reviewed while draft but head commit is NOT a wip commit (e.g., commit-pushed-no-pr class where pilot made a clean commit) — draft-PR guard still catches it

**Verification:** The prompt text contains all three guards before the verdict routing. Each guard independently produces the skip-all-verdict-processing outcome. A new calibration rule documents the defense-in-depth pattern with the mika#1610 incident reference.

---

## Verification Contract

1. **Structural verification:** All three files modified; `grep RECOVERY_PENDING skills/bundled/_shared/dispatch-lib.sh` finds the marker in the rescue block; `grep -c 'isDraft\|wip(' skills/bundled/self-dev-webhook-qa/system_prompt.md` confirms both defense-in-depth guards are present.
2. **Logical verification:** Trace the rescue flow end-to-end: dispatch-lib rescue -> RESULT with `RECOVERY_PENDING: true` -> callback handler sets `unpushed_recovery_pending: true` in metadata -> qa-webhook sees flag and skips verdict processing. Each link is verifiable by reading the prompt text.
3. **Defense-in-depth verification:** Remove the metadata flag mentally — the draft-PR guard still blocks. Remove the draft status mentally — the wip-commit guard still blocks. Each guard is independently sufficient.

---

## Acceptance criteria

1. A PR opened by the mika#1282 dirty-worktree rescue (or mika#1383 auto-PR-create tail) does **not** get autonomously un-drafted/auto-merged: qa-webhook skips verdict processing and escalates to the operator, matching the existing `unpushed_recovery_pending` behavior.
2. The signal is durable: prefer task-metadata flag set at rescue time; if also recognizing the `wip(...mika#1282)` commit signature, document why.
3. A test (dispatch-lib test harness and/or a qa-webhook decision test) covers: wip-rescue draft PR → qa event → verdict processing skipped + operator escalation.
4. Regression coverage so a future rescue-path addition that forgets the flag is caught.

---

## Definition of Done

- [ ] dispatch-lib appends `RECOVERY_PENDING: true` to RESULT when a rescue PR is created
- [ ] self-dev-callback parses `RECOVERY_PENDING: true` and writes metadata flag before routing
- [ ] self-dev-webhook-qa checks `isDraft` and `wip(` commit prefix as independent guards
- [ ] New calibration rule documents the three-guard pattern with mika#1610 reference
- [ ] All three files pass prompt review (no syntax errors, no broken markdown)

---

## Sources & Research

- mika#1613 — this ticket (loop ships unreviewed code)
- mika#1282 — dirty-worktree wip-rescue contract ("operator must review and promote")
- mika#1396 — commit-pushed-no-pr recovery class
- mika#1610 — incident PR (merged unreviewed code)
- mika#1612 — gateway defects shipped by #1610
- `skills/bundled/_shared/dispatch-lib.sh` lines 763-886 (rescue logic), 2401-2472 (Unit 2 draft PR creation)
- `skills/bundled/self-dev-callback/system_prompt.md` lines 78-88 (recover_unpushed_work handler)
- `skills/bundled/self-dev-webhook-qa/system_prompt.md` lines 199-204 (recovery-skip guard)

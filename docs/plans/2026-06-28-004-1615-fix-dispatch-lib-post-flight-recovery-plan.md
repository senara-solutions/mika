---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
created: 2026-06-28
issue: mika#1615
---

# fix(dispatch-lib): post-flight recovery unreachable on non-structured-output exit paths

## Goal Capsule

Make dispatch-lib's mika#1282 post-flight recovery fire on **every** pilot exit path — not just the structured-JSON-output path. Today, the dirty-worktree rescue, commit-pushed-no-pr rescue, dev-groom plan validation, and PR-existence check are all inside the `if [ -n "$STATUS" ]` branch. When claude-pilot exits without structured JSON output (STATUS empty) or exits non-zero, recovery is silently skipped and uncommitted work is lost.

---

## Problem Frame

dispatch-lib's `_run_claude_pilot()` has a three-branch exit classification:

```
if [ -n "$STATUS" ]; then          # Branch A: structured JSON output
    POST_RUN_HEAD=...              # ← computed here only
    # dirty-worktree rescue        # ← reachable here only
    # commit-pushed-no-pr rescue   # ← reachable here only
    # dev-groom plan validation    # ← reachable here only
    # PR-existence check           # ← reachable here only
elif [ "$PILOT_EXIT" -eq 0 ]; then # Branch B: exit 0, non-structured output
    # ⚠️ NO recovery, NO POST_RUN_HEAD
else                               # Branch C: non-zero exit
    # ⚠️ NO recovery, NO POST_RUN_HEAD
fi
```

Branch B and C have zero recovery logic. Additionally, `POST_RUN_HEAD` is only computed inside Branch A — so even the downstream Unit 2 draft-PR creation (line ~2413) fails silently because it checks `POST_RUN_HEAD` which is uninitialized.

**Hard evidence:** mika-cloud#133 implement session ran 90 turns / $5.31, exited with 282 lines uncommitted, and neither dirty-worktree nor commit-pushed-no-pr recovery fired. Same day, mika#1609 (dirty-worktree) and mika-cloud#134 (commit-pushed-no-pr) both fired successfully — because they had structured JSON output and stayed in Branch A.

---

## Requirements

- R1. Post-flight recovery (dirty-worktree rescue, commit-pushed-no-pr rescue) MUST fire regardless of whether claude-pilot produced structured JSON output.
- R2. `POST_RUN_HEAD` MUST be computed for all exit paths when `PRE_RUN_HEAD` and `WORKTREE_DIR` are set.
- R3. Dev-groom plan validation MUST fire regardless of output structure.
- R4. PR-existence check (mika#940) MUST fire regardless of output structure.
- R5. Existing behavior for Branch A (structured output) MUST be unchanged — no regression.
- R6. The `RESCUED_DIRTY_WORKTREE` flag MUST be set correctly so Unit 2 (draft PR creation at line ~2413) fires.

---

## Key Technical Decisions

### KTD1. Extract recovery into a post-classification function

**Decision:** Extract the four recovery blocks into a new `_post_flight_recovery()` function that runs unconditionally after the three-branch STATUS/exit-code classification.

**Rationale:** The recovery logic has no actual dependency on `STATUS` — it depends on `PRE_RUN_HEAD`, `POST_RUN_HEAD`, `WORKTREE_DIR`, `SKILL`, `REPO`, and `BRANCH`. Moving it out of the `if [ -n "$STATUS" ]` block makes it structurally impossible for future exit paths to skip recovery.

**Alternative considered:** Duplicating the recovery blocks into Branch B and Branch C. Rejected — violates DRY, triples the maintenance surface, and any future new recovery class would need to be added in three places.

### KTD2. Compute POST_RUN_HEAD unconditionally

**Decision:** Move the `POST_RUN_HEAD` computation out of the `if [ -n "$STATUS" ]` block and into the shared post-classification code, guarded only by `PRE_RUN_HEAD` and `WORKTREE_DIR`.

**Rationale:** `POST_RUN_HEAD` is needed by both the in-function recovery blocks and the downstream Unit 2 draft-PR creation. Computing it unconditionally ensures all consumers see correct state.

---

## Scope Boundaries

### In scope

- Extracting recovery logic from the STATUS-conditional into a shared post-classification path
- Moving `POST_RUN_HEAD` computation to run unconditionally
- Adding test coverage for the non-structured-output recovery path

### Out of scope

- Modifying the two recovery classes that already work (dirty-worktree, commit-pushed-no-pr)
- Changes to claude-pilot itself or its JSON output format
- Changes to the Unit 2 draft-PR creation block (lines ~2401-2472 in `dispatch_claude_pilot`)

### Deferred to Follow-Up Work

- Investigating why mika-cloud#133 specifically produced non-structured output (symptom investigation vs. structural fix)

---

## Implementation Units

### U1. Compute POST_RUN_HEAD unconditionally after pilot exit

**Goal:** Ensure `POST_RUN_HEAD` is set for all exit paths, not just the structured-output path.

**Requirements:** R2

**Dependencies:** None

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh`

**Approach:**

Move the `POST_RUN_HEAD` computation from inside the `if [ -n "$STATUS" ]` block (currently at line ~716) to immediately after the three-branch classification closes (after line ~1189, before the stderr append). Guard it with `[ -n "$PRE_RUN_HEAD" ] && [ -n "$WORKTREE_DIR" ]` — the same guard the current code uses.

The computation stays a simple `git -C "$WORKTREE_DIR" rev-parse HEAD` with `|| true` fallback. The existing Branch A code that uses POST_RUN_HEAD inline (e.g., the PIPELINE FAILURE message at line ~757 that interpolates `${POST_RUN_HEAD}`) will continue to work because POST_RUN_HEAD is now set before the branches run.

Specifically:
1. Remove the `POST_RUN_HEAD=...` assignment from line ~716 (inside Branch A).
2. Add `POST_RUN_HEAD=""` initialization near line ~664 (alongside `RESCUED_DIRTY_WORKTREE=0`).
3. After the `fi` that closes the three-branch classification (after line ~1179) and before the stderr tail append (line ~1182), insert the unconditional POST_RUN_HEAD computation:
   ```
   if [ -n "$PRE_RUN_HEAD" ] && [ -n "$WORKTREE_DIR" ]; then
       POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
   fi
   ```

Wait — this creates a sequencing issue: Branch A's internal logic at lines 717-935 uses `POST_RUN_HEAD` extensively (the HEAD-unchanged check, dirty-worktree rescue, mika#1383 trailing-dirty rescue). If we move the computation after the branches, those uses break.

**Revised approach:** Compute POST_RUN_HEAD **before** the three-branch classification, right after PILOT_OUTPUT processing (after line ~693, alongside STATUS/SESSION_ID extraction). This way all three branches and the downstream code see it. The guard is `[ -n "$PRE_RUN_HEAD" ] && [ -n "$WORKTREE_DIR" ]`.

**Patterns to follow:** The existing `PRE_RUN_HEAD` pattern at line ~625 (set in `_set_up_worktree`, used in `_run_claude_pilot`).

**Test scenarios:**
- POST_RUN_HEAD is set when PRE_RUN_HEAD and WORKTREE_DIR are present, regardless of STATUS value
- POST_RUN_HEAD remains empty when PRE_RUN_HEAD is empty (free-text mode)
- Existing Branch A behavior is unchanged — HEAD-unchanged check, dirty-worktree rescue, mika#1383 all still fire

**Verification:** `grep -n 'POST_RUN_HEAD=' dispatch-lib.sh` shows exactly one assignment site (the new unconditional one) plus the rescue-block updates (lines ~845, ~878, ~968 where rescue commits update it).

---

### U2. Extract recovery logic into _post_flight_recovery function

**Goal:** Make the four recovery blocks (dirty-worktree rescue, mika#1383 commit-pushed-no-pr, dev-groom plan validation, PR-existence check) run unconditionally after pilot exit classification.

**Requirements:** R1, R3, R4, R5, R6

**Dependencies:** U1

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh`
- `skills/bundled/_shared/test-dispatch-lib.sh`

**Approach:**

Extract the following code blocks from inside the `if [ -n "$STATUS" ]` branch into a new `_post_flight_recovery()` function:

1. **POST_RUN_HEAD computation** — already moved by U1, so this is already unconditional.
2. **HEAD-unchanged message** (lines ~717-761) — the PIPELINE FAILURE / policy-deny / dev-groom-re-dispatch classification. This is RESULT-message-shaping, not recovery per se, but it sets context for the dirty-worktree rescue that follows. Must still be guarded by `[ -n "$PRE_RUN_HEAD" ] && [ -n "$REPO" ]` and `[ "$PRE_RUN_HEAD" = "$POST_RUN_HEAD" ]`.
3. **Dirty-worktree rescue** (lines ~763-935) — the mika#1282 core recovery. Guarded by `[ "$PRE_RUN_HEAD" = "$POST_RUN_HEAD" ] && [ "$SKILL" = "dev-pilot" ] && [ -n "$WORKTREE_DIR" ]`.
4. **mika#1383 trailing-dirty + PR auto-creation** (lines ~954-1026) — guarded by `[ "$SKILL" = "dev-pilot" ] && HEAD changed`.
5. **Dev-groom plan validation** (lines ~1029-1107) — guarded by `[ "$SKILL" = "dev-groom" ]`.
6. **PR-existence check / PR URL discovery** (lines ~1109-1117) — guarded by `[ -n "$REPO" ] && [ -n "$BRANCH" ]`.
7. **mika#940 PR-existence post-flight check** (lines ~1119-1147) — guarded by `[ "$STATUS" = "success" ] && [ "$SKILL" = "dev-pilot" ]`.
8. **Outcome classification** (lines ~1140-1166) — the PIPELINE_INCOMPLETE / PR_OPENED / PLAN_COMMITTED / UNKNOWN outcome line.

The key structural change: these blocks move from being **nested inside** `if [ -n "$STATUS" ]` to being called **after** the three-branch classification. The three branches (A, B, C) continue to own RESULT-message construction (the session summary). The new function owns everything that depends on git state, not output structure.

**Guard adjustments for Branch B/C:** Some recovery guards check `$STATUS` (e.g., mika#940 at line ~1134: `[ "$STATUS" = "success" ]`). When STATUS is empty (Branch B/C), these guards naturally short-circuit — no behavior change needed. The critical recovery paths (dirty-worktree, commit-pushed-no-pr) do NOT check STATUS.

**RESCUED_DIRTY_WORKTREE:** Already initialized at line ~664 and set to 1 inside the dirty-worktree rescue block. Moving the rescue block to a shared function preserves this — the variable is function-scoped in bash (global to `_run_claude_pilot`'s caller).

**Patterns to follow:** The existing `_push_branch()`, `_check_pilot_force_push()`, and `_deliver_callback()` functions — all are extracted post-flight steps called from `dispatch_claude_pilot()`.

**Test scenarios:**
- Dirty-worktree rescue fires when STATUS is empty (Branch B): pilot exits 0 with non-JSON output, HEAD unchanged, dirty files present → files are auto-committed, RESCUED_DIRTY_WORKTREE=1
- Dirty-worktree rescue fires when exit code is non-zero (Branch C): pilot crashes with exit code 1, HEAD unchanged, dirty files present → files are auto-committed
- Commit-pushed-no-pr rescue fires when STATUS is empty: pilot exits 0 with non-JSON output, HEAD changed, no PR exists → PR auto-created
- Dev-groom plan validation fires when STATUS is empty: dev-groom pilot exits with non-JSON output, plan file present → no false PIPELINE FAILURE
- PR-existence check fires when STATUS is empty: pilot exits with non-JSON output, PR exists for branch → PR URL appended to RESULT
- Existing Branch A behavior unchanged: all existing tests pass without modification (regression guard)
- RESCUED_DIRTY_WORKTREE=1 propagates correctly to Unit 2 draft-PR creation in dispatch_claude_pilot()

**Verification:**
- Run `bash skills/bundled/_shared/test-dispatch-lib.sh` — all existing tests pass
- Structural assertion: `grep -c 'RESCUED_DIRTY_WORKTREE=1' dispatch-lib.sh` returns the same count as before (the assignment sites are in the rescue block, which moved but didn't change)
- The `_post_flight_recovery` function is called from exactly one site: after the three-branch classification in `_run_claude_pilot`

---

## Verification Contract

1. All existing tests in `test-dispatch-lib.sh` pass without modification
2. New tests cover the Branch B and Branch C recovery paths
3. Structural grep: `POST_RUN_HEAD` is computed in exactly one place (plus the rescue-block updates), outside any STATUS-conditional
4. Structural grep: dirty-worktree rescue, plan validation, and PR-existence check are NOT inside `if [ -n "$STATUS" ]`

---

## Definition of Done

- [ ] POST_RUN_HEAD computed unconditionally for all exit paths
- [ ] Recovery logic extracted from STATUS-conditional into shared post-classification path
- [ ] New tests cover dirty-worktree rescue on non-structured-output exit
- [ ] All existing test-dispatch-lib.sh tests pass
- [ ] PR opened with `Closes #1615`

## Acceptance criteria

- **AC1** — `POST_RUN_HEAD` is computed unconditionally in `_run_claude_pilot()` after the three-branch STATUS/exit-code classification, NOT inside any STATUS-conditional. Structural assertion: `grep -c 'POST_RUN_HEAD=' skills/bundled/_shared/dispatch-lib.sh` returns ≥ 2 (init + computation), with the computation outside `if [ -n "$STATUS" ]`.
- **AC2** — All post-flight recovery logic (dirty-worktree rescue, dev-groom plan validation, PR-existence check, mika#940 post-flight check, outcome classification) is extracted into a single `_post_flight_recovery()` function, called from exactly one site after exit classification.
- **AC3** — Branch B (exit 0, non-JSON output) triggers `_post_flight_recovery()` with RESULT pre-set; previously this path skipped recovery entirely.
- **AC4** — Branch C (non-zero exit) triggers `_post_flight_recovery()` with RESULT pre-set; previously this path skipped dirty-worktree rescue.
- **AC5** — Existing Branch A behavior is preserved: all existing tests in `test-dispatch-lib.sh` pass without modification.
- **AC6** — New test (Test 17, mika#1615) covers 10 structural assertions + 3 behavioral sub-tests exercising Branch B and Branch C recovery using real git repos via mktemp + subshell isolation.
- **AC7** — `RESCUED_DIRTY_WORKTREE=1` propagates correctly to Unit 2 draft-PR creation in `dispatch_claude_pilot()` from all three branches when dirty-worktree rescue fires.
- **AC8** — Plan filename matches `<date>-<seq>-<issue>-<slug>-plan.md` pattern so `_find_issue_plan` regex matches (mika#1617 backcompat).

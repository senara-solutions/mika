---
title: "fix: stop the dev-groom pilot from mis-diagnosing 'branch ahead of remote' on stale local main"
status: active
date: 2026-06-05
type: fix
issue: senara-solutions/mika#1407
branch: fix/1407/dispatch-lib-pilot-mis-diagnoses-branch
milestone: 30
---

# fix(dispatch-lib): pilot mis-diagnoses 'branch ahead of remote' when worktree's local main is stale

> **Cross-repo plan.** Primary work lands in `mika` (this repo). One companion change lands in `mika-platform` on the same branch name (`fix/1407/dispatch-lib-pilot-mis-diagnoses-branch`). Paths are repo-relative to their stated repo.

---

## Summary

The autonomous dev-groom pilot, running the `/mika-groom-plan-only` command, aborts a grooming session with `Plan committed locally — remote divergence detected; abort to dispatch-lib for reconciliation` even when there is nothing to push (HEAD already equals `origin/<branch>`). The command prompt instructs the pilot to `git push` and emit that abort string "if the remote is ahead," but the prompt's push step is **redundant** — `dispatch-lib.sh::_push_branch` already pushes structurally after the session, and its own comment declares it "the sole git-push site for dev-groom dispatches." The redundant prompt-level push is also the **fragile predicate** that mis-fires: an LLM reading a stale local `main` ref concludes "branch ahead of remote → diverged → abort," conflating three distinct git states.

The fix removes the fragile predicate rather than making it smarter: strip the pilot's push + abort protocol from the command prompt (the pilot generates the plan, commits, and exits), and let dispatch-lib's already-correct structural push own the git layer. In `mika`, the three git states are named explicitly in `_push_branch` and locked with a regression test, satisfying the issue's testable acceptance criteria in the repo where they are testable.

---

## Problem Frame

### What happened (evidence)

- **Incident:** Mika Prime's first Stage-1 dispatch (groom of mika#1255, milestone-30 P0) went to `blocked` in 31s. Pilot log `fbe1481e-...`:
  > "The branch is ahead of the remote (the local has been rebased onto updated main)… I need to push to sync the remote. Plan committed locally — remote divergence detected; abort to dispatch-lib for reconciliation."
- **Actual worktree state (verified, mika#1407 body):** `git rev-parse HEAD` and `git rev-parse origin/<branch>` both `e8a444a6…` — identical SHA. **Nothing to push.** The only real divergence was the worktree's local `main` ref at the branch merge-base (`c0ea9a01`) while `origin/main` was at `64d11cf0` — *branch base behind main*, which is orthogonal to *branch ahead of remote*.

### Root cause (verified by code reading)

- The exact abort string is emitted under instruction from `mika-platform/.claude/commands/mika-groom-plan-only.md:57` (Phase 2, step 7). The prompt tells the pilot to `git push -u origin <branch>` and, on push rejection / "remote ahead," exit with that string. It never tells the model to first check whether anything needs pushing, nor to distinguish the three states.
- The dev-groom **skill** prompt `skills/bundled/dev-groom/system_prompt.md` contains **no** push/abort/divergence instructions (grep-confirmed). The only prompt artifact driving the push is the mika-platform command.
- `skills/bundled/_shared/dispatch-lib.sh::_push_branch` (≈777–835) is **already correct and structural**: fetches `origin/$BRANCH`, computes `ahead=$(git rev-list "origin/$BRANCH..HEAD" --count)`, returns early (no-op) when `ahead==0`, and uses `--force-with-lease` only when ancestry proves genuine divergence. Its comment (≈833): *"this helper is now the sole git-push site for dev-groom dispatches."*
- Run flow (`dispatch-lib.sh` ≈1687–1719): `_set_up_worktree` (rebases the branch onto `origin/main`, owns the *base-behind-main* concern) → `_run_claude_pilot` (`/mika-groom-plan-only`) → `_iterate_groom_loop` (architect; reads the **local** committed plan; may add revise commits) → `_push_branch` (the authoritative push). The pilot's push is therefore pure redundancy that contradicts the "sole git-push site" contract — the command prompt is **stale**, left behind when `_push_branch` became authoritative (mika#1271 content/workflow split, mika#1268/#1364 push hardening).

### The three conflated states

| State | Correct detection | Correct action | Owner |
|-------|-------------------|----------------|-------|
| HEAD == `@{u}` (`origin/<branch>`) | `rev-list origin/$BRANCH..HEAD == 0` | nothing to push — **no-op, NOT a divergence** | `_push_branch` (early return) |
| HEAD ahead of `@{u}` | `rev-list origin/$BRANCH..HEAD > 0`, FF or diverged | push (`--force-with-lease` only if diverged) | `_push_branch` |
| Branch base behind `origin/main` | `rev-list HEAD..origin/main > 0` | rebase — **orthogonal to push** | `_set_up_worktree` |

The mis-diagnosis was reading the third state's symptom (stale local `main`) and firing the second state's action (push) and then the abort. The durable fix is to stop asking the LLM to make this call at all.

---

## Scope Boundaries

### In scope
- Remove the pilot's push + abort-string protocol from `/mika-groom-plan-only` (mika-platform).
- Make the three-state distinction explicit in `_push_branch` (mika) and lock it with a regression test (mika).

### Out of scope (true non-goals)
- Any change to `_push_branch`'s push **behavior** — it is already correct. Only documentation/comment clarity plus a test are added (unless the test reveals a real gap, see U2).
- `/mika-groom-ticket` (the operator-facing full pipeline) — it owns its own architect + body-callout flow and is unaffected; its Phase-2 push semantics are not in scope here.
- The `_set_up_worktree` rebase logic (it already owns base-behind-main correctly).
- Anything touching the polymorphic `/mika` repo-targeting (this is a content/contract fix only).

### Deferred to follow-up work
- A broader audit of whether `/mika-revise-plan` (the revise pilot) carries the same redundant-push pattern. Noted for a separate ticket if confirmed; not pulled into this fix.

---

## Key Technical Decisions

### KTD-1 — Remove the predicate, don't refine it (structural over prompt)
The issue's stated approach ("fix the diagnostic to compare against `@{u}`") could be satisfied by rewriting the prompt to be smarter. We reject that: a smarter prompt is still an LLM judgment over three git states, and project doctrine is structural enforcement over prompt prose (`docs/solutions` lineage; memory `feedback_prompt_enforcement_fragile`, `feedback_structural_enforcement_layer_for_tool_requirements`). Because `_push_branch` **already** performs the correct push structurally and unconditionally after the session, the pilot's push is removable with zero behavior loss. Removing it eliminates the load-bearing-but-wrong predicate entirely. This is the milestone-30 (Loop Trustworthiness) thesis applied: a dispatch must not hinge on a fragile prompt-level git judgment.

### KTD-2 — `mika` is the ticket repo and the invariant's home
mika#1407 is filed on `mika` and titled `fix(dispatch-lib)`. The behavioral bug lives in the mika-platform prompt, but the **invariant** the prompt now delegates to (`_push_branch` is the sole, correct push authority) lives in `mika`. The testable acceptance criteria — "three-state comparison named explicitly in the diagnostic logic" and "test covers the conflated case" — are satisfiable only where there is testable code: `mika`. So `mika` carries the doc-comment + regression test; `mika-platform` carries the prompt edit. Same branch name on both repos per cross-repo convention.

### KTD-3 — Test style follows the existing harness, with a behavioral fixture if cleanly feasible
`skills/bundled/_shared/test-dispatch-lib.sh` is a **source-introspection** suite — it extracts function bodies as text and asserts on structure; it explicitly does not spin up real git. The regression test (U2) is primarily **structural**: assert `_push_branch` compares against `origin/$BRANCH` (the remote-tracking ref) and not local `main`, and returns early when `ahead==0`. If `_push_branch` proves cleanly invokable against a self-contained local bare-repo fixture at work time, add a **behavioral** assertion for the conflated case (HEAD==origin/branch + stale local main → no push, no error, no abort). Behavioral feasibility is an execution-time discovery (the function reads `REPO`/`WORKTREE_DIR`/`BRANCH` globals and calls `_check_duplicate_commits` which fetches `origin main`); the structural assertions are the guaranteed floor.

---

## Implementation Units

### U1. Make the three git states explicit in `_push_branch` (mika)

**Goal:** Name the three conflated states and the ownership boundary directly in the diagnostic code, so the intent is legible and the invariant the prompt delegates to is self-documenting. Satisfies AC: "three-state comparison is named explicitly in the diagnostic logic."

**Requirements:** mika#1407 AC-2.

**Dependencies:** none.

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh` (modify — comment block within `_push_branch`, ≈777–835)

**Approach:**
- Add a concise comment block at the top of the existing-remote branch of `_push_branch` enumerating the three states from the Problem Frame table: (a) `HEAD == origin/$BRANCH` → ahead==0 → return 0, **not** a divergence; (b) `HEAD` ahead of `origin/$BRANCH` → push (FF or `--force-with-lease` per ancestry); (c) branch base behind `origin/main` → owned by `_set_up_worktree`, orthogonal to push.
- Explicitly state the comparison is against the **remote-tracking branch** (`origin/$BRANCH`), never local `main` — citing mika#1407 as the conflation this guards against.
- **No behavior change.** The code at the `ahead` computation and early `return 0` already realizes states (a) and (b); this unit only makes the intent explicit and cites the ticket.

**Patterns to follow:** the existing mika#-citation comment style already used throughout `_push_branch` (e.g. the mika#1364 KTD-1 ancestry comment).

**Test scenarios:** Covered by U2 (the comment is verified indirectly by the structural assertions there). No standalone test for a comment.

**Verification:** `_push_branch` contains an inline three-state enumeration citing mika#1407 and naming `origin/$BRANCH` as the comparison ref; `cargo`/shell behavior unchanged (no logic edited).

---

### U2. Regression test locking the remote-tracking-branch comparison (mika)

**Goal:** Prove the diagnostic distinguishes "branch equal to remote" (no-op) from any local-`main` comparison, so the conflated case can never silently regress. Satisfies AC: "test covers the conflated case."

**Requirements:** mika#1407 AC-3 (and AC-1 reproducer, to the extent shell-testable).

**Dependencies:** U1 (asserts on the clarified `_push_branch`).

**Files:**
- `skills/bundled/_shared/test-dispatch-lib.sh` (modify — add a `_push_branch` assertion group)

**Approach:**
- **Structural assertions (guaranteed floor), in the existing `assert_contains` / `assert_not_contains` style:**
  - Extract the `_push_branch` body and assert it computes `ahead` from `origin/$BRANCH..HEAD` (remote-tracking ref).
  - Assert the early `return 0` exists in the `ahead == 0` branch (no-op when HEAD==origin/branch).
  - Assert `_push_branch` does **not** compare against `main`/`origin/main` for the push decision (`assert_not_contains` the local-main comparison in the push-decision region) — i.e. base-behind-main does not drive the push decision.
  - Assert the three-state comment from U1 is present (anchor: the `#1407` citation + "remote-tracking" phrasing).
- **Behavioral assertion (add if cleanly feasible at work time — KTD-3):** build a self-contained fixture — a local bare repo as `origin`, a working clone with a branch whose HEAD equals `origin/<branch>` and whose local `main` is intentionally stale (behind `origin/main`) — source `dispatch-lib.sh`, set `REPO`/`WORKTREE_DIR`/`BRANCH`, call `_push_branch`, and assert: return code 0, no new commits on `origin/<branch>`, and `RESULT` contains no abort/divergence string. If isolation proves infeasible (global/dependency coupling), record that in the test file as a comment and rely on the structural assertions.

**Patterns to follow:** existing extraction-and-assert groups in `test-dispatch-lib.sh` (e.g. the `_detect_plan_on_branch` group ≈190–217); `set -euo pipefail`; `PASS`/`FAIL` counters.

**Test scenarios:**
- Happy path: `_push_branch` body contains `origin/$BRANCH..HEAD` ahead computation → assert present.
- Conflated case (the bug): push decision region does not reference local `main` → `assert_not_contains`.
- No-op guard: `ahead == 0` path returns 0 → assert present.
- Intent doc: three-state comment citing `#1407` present → assert present.
- (Behavioral, if feasible) `Covers AC-1/AC-3.` HEAD==origin/branch + stale local main → `_push_branch` returns 0, pushes nothing, emits no abort string.

**Verification:** `bash skills/bundled/_shared/test-dispatch-lib.sh` exits 0 with the new assertions passing; the suite still passes in full.

---

### U3. Strip the redundant pilot push + abort protocol from `/mika-groom-plan-only` (mika-platform)

> **Target repo: `mika-platform`.** Companion change on the same branch. Paths below are repo-relative to `mika-platform`.

**Goal:** Remove the fragile, redundant push from the pilot's content-only command so the pilot never makes a push-state judgment. The pilot generates the plan, commits, and exits; `dispatch-lib::_push_branch` owns the push. This is the change that actually stops the mis-diagnosis from recurring.

**Requirements:** mika#1407 AC-1, AC-2 ("…or its prompt").

**Dependencies:** Conceptually relies on the mika invariant (U1) being true — it already is in shipped code; U1 documents it. No build-time dependency.

**Files:**
- `.claude/commands/mika-groom-plan-only.md` (modify)

**Approach (edits, each removing a push-judgment surface):**
- **Exit contract (≈22–31):** drop item 2 ("The branch pushed to origin"). New exit contract: a plan file committed on the grooming branch. Add one line stating dispatch-lib's `_push_branch` performs the push after the session — the pilot must not push.
- **Phase 2 step 7 (≈52–57):** remove the `git push -u origin <branch>` step and the entire "remote ahead / abort to dispatch-lib for reconciliation" protocol paragraph, including the abort string. Replace with an explicit instruction: after committing, **exit without pushing** — pushing is dispatch-lib's job (`_push_branch`, the sole git-push site for dev-groom dispatches). Keep the existing "never force-push" prohibition framed as "the pilot performs no pushes at all."
- **Phase 3 step 8 (≈59–61):** adjust so the clean-exit text no longer presumes a prior `git push` ("After `git push` succeeds…" → "After committing the plan…").
- **"What this command does NOT do" (≈64–71):** update the force-push bullet to state the pilot performs **no** git push of any kind; reconciliation and push both belong to dispatch-lib.
- **Failure modes (≈73–77):** remove the "Push fails" bullet's reliance on the pilot push; keep the commit-fails bullet. Note that push failures are surfaced by dispatch-lib's `_push_branch` post-flight, not the pilot.
- Preserve everything about plan generation, idempotent re-groom detection, commit-unconditionally, and the no-architect / no-callout / no-comment contract — those are unchanged.

**Patterns to follow:** the command's existing mika#-citation style; keep the content/workflow split framing from mika#1271 (pilot = content, dispatch-lib = git workflow) — this edit makes the command consistent with that split.

**Test scenarios:** `Test expectation: none — prose command file, not executable.` Verification is by inspection + the downstream behavioral effect. (The reproducer AC is satisfied behaviorally: with the push step gone, a re-dispatch on a branch where HEAD==origin/branch produces a clean groom because the pilot commits-or-noops and exits, and `_push_branch` no-ops.)

**Verification:**
- `grep -n "git push\|remote divergence detected\|abort to dispatch-lib for reconciliation" .claude/commands/mika-groom-plan-only.md` returns no pilot-push instruction and no abort string.
- The exit contract and "does NOT do" sections consistently state the pilot performs no push.

---

## System-Wide Impact

- **Autonomous dev-groom loop (primary beneficiary):** re-dispatches on an already-groomed ticket (HEAD==origin/branch) no longer false-abort. The groom proceeds: pilot commits-or-noops → architect converges → `_push_branch` no-ops or pushes revise commits. Directly unblocks the milestone-30 failure class.
- **No behavior change to `_push_branch`** → no risk to the impl (dev-pilot) push path, which already used `_push_branch` exclusively.
- **Ordering safety (verified):** the architect (`_iterate_groom_loop`) reads the **local** committed plan, not origin; `_push_branch` runs after it and before `_deliver_callback`, so origin receives the branch (plan + any revise commits) before the dispatch completes — the next dispatch's `_set_up_worktree` still finds `origin/$BRANCH`. Removing the pilot push does not strand the branch.
- **Deploy:** `dispatch-lib.sh` is copy-deployed (not in-binary); the command file is copied into pilot worktrees by `_set_up_worktree` (dispatch-lib.sh:359) from `mika-platform/.claude/commands/`. Both changes take effect via the normal `make deploy` / next-dispatch worktree refresh — no migration.

---

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| A consumer relies on the pilot having pushed before architect runs | Low | Verified architect reads local plan; `_push_branch` pushes after. No origin dependency before `_push_branch`. |
| `_push_branch` not cleanly invokable for a behavioral test | Medium | KTD-3: structural assertions are the guaranteed floor; behavioral test is additive-if-feasible, documented either way. |
| Prompt edit accidentally weakens the no-force-push safety language (mika#1318) | Low | Reframe force-push prohibition as "no pushes at all" — strictly stronger; keep the mika#1318 citation. |
| Stale copies of the command in existing pilot worktrees | Low | `_set_up_worktree` re-copies `.claude/commands/` on every dispatch (dispatch-lib.sh:359); no manual cleanup needed. |

---

## Cross-Repo Sequencing

1. **mika (primary):** U1 (comment) → U2 (test) → run `bash skills/bundled/_shared/test-dispatch-lib.sh`. Commit on `fix/1407/dispatch-lib-pilot-mis-diagnoses-branch`. PR closes mika#1407; cross-references the companion.
2. **mika-platform (companion):** U3 (prompt edit) on the same branch name, committed directly. Companion PR cross-references the mika PR (`Companion PR: senara-solutions/mika#<n>`).
3. Both PR bodies lead with the WHY (this Problem Frame) and the structural-over-prompt rationale (KTD-1).

---

## Acceptance Criteria Trace (mika#1407)

- **AC-1 (reproducer / clean groom under fix):** satisfied behaviorally by U3 — with the pilot push removed, HEAD==origin/branch + stale local main yields a clean groom (pilot commits-or-noops, exits; `_push_branch` no-ops). Shell-level reproduction added in U2 if `_push_branch` isolation is feasible.
- **AC-2 (three-state comparison named explicitly in the diagnostic logic or its prompt):** satisfied by U1 (in `_push_branch`) and reinforced by U3 (prompt no longer makes the flawed comparison).
- **AC-3 (test covers the conflated case):** satisfied by U2 — structural assertions that the push decision uses `origin/$BRANCH..HEAD` and not local `main`, plus the behavioral conflated-case test when feasible.

---

## Sources & Research

- mika#1407 (issue body — incident evidence, three-state framing, ACs).
- `skills/bundled/_shared/dispatch-lib.sh` — `_push_branch` (≈777–835), run flow (≈1687–1719), command-copy (≈359), dev-groom entry (≈1668–1681).
- `mika-platform/.claude/commands/mika-groom-plan-only.md` — push/abort protocol (≈52–71).
- `skills/bundled/dev-groom/system_prompt.md` — confirmed no push/abort logic (grep).
- `skills/bundled/_shared/test-dispatch-lib.sh` — harness style (source-introspection).
- mika#1271 (content/workflow split), mika#1268/#1364 (`_push_branch` introduction + push hardening), mika#1318 (force-push-destroyed-substrate, the reason the pilot must not force-push).
- Doctrine: `feedback_prompt_enforcement_fragile`, `feedback_structural_enforcement_layer_for_tool_requirements`.

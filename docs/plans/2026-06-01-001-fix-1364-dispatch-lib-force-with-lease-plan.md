---
title: "fix: dispatch-lib strands rebased impl behind stale remote tip (force-with-lease + rebase diagnostics)"
type: fix
status: active
date: 2026-06-01
issue: senara-solutions/mika#1364
target_repo: mika
branch: fix/1364/dispatch-lib-push-branch-lacks-force
---

# fix(dispatch-lib): force-with-lease the rebased branch + surface rebase failure mode

**Target repo:** `mika` (file: `skills/bundled/_shared/dispatch-lib.sh`)

---

## Summary

`dispatch-lib.sh` already rebases a reused remote branch onto `origin/main` before the pilot
runs (the rebase-or-abort guard added 2026-04-23, see origin learning below). The unaddressed
sibling gap: **after that rebase rewrites history, the push that should land it is a non-force
`git push`** — which is rejected as non-fast-forward because the remote tip still holds the
pre-rebase commits. The rebased work (including the `#1282` post-flight `wip()` rescue commit,
which shares this same push) is left local-only and the dispatch ends with no PR /
`callback_delivered_without_pr_url`. This plan makes the push divergence-aware
(`--force-with-lease`) and surfaces the rebase failure mode that is currently swallowed by
`2>/dev/null`, then adds regression coverage in `test-dispatch-lib.sh`.

This is a P0 substrate hot-path bash file. The fix is surgical: no new abstractions, no new
flags, no behavior change for the clean first-push / fast-forward cases.

---

## Problem Frame

### The strand mechanism (WHY)

1. `_set_up_worktree` (dispatch-lib.sh:189) reuses an existing remote branch: when
   `origin/$BRANCH` exists it bases the worktree on `origin/$BRANCH` — the *stale tip*, which
   may be days old and/or a prior `#1282` `wip()` rescue commit (dispatch-lib.sh:279-283,
   mika#1311 rationale comment).
2. The rebase-or-abort guard (dispatch-lib.sh:292-324) computes `BEHIND=rev-list --count
   HEAD..origin/main`; on `BEHIND > 0` it runs `git rebase origin/main 2>/dev/null`. On success
   the branch HEAD is now a chain of **new SHAs** replayed onto current `origin/main`. The local
   branch has **diverged** from `origin/$BRANCH` — the remote still points at the old pre-rebase
   tip, which is *not an ancestor* of the rebased HEAD.
3. The pilot runs against this correct, on-main base. If it leaves a dirty worktree with zero
   commits, `#1282` post-flight recovery commits `wip(...)` and bumps `POST_RUN_HEAD` so
   `_push_branch` will push it (dispatch-lib.sh:530-548).
4. `_push_branch` (dispatch-lib.sh:762-808) fetches `origin/$BRANCH` (line 781), sees HEAD is
   "ahead" (line 787 — true, because none of the rebased SHAs are on the stale remote tip), and
   runs `git push -u origin "$BRANCH"` — **non-force** (line 797). Git rejects it as
   non-fast-forward (the remote tip is not an ancestor of HEAD). The branch falls to the
   `Push: FAILED — commits remain local-only` arm (line 802-805).
5. Net: the real implementation (or the rescued `wip()`) is stranded local-only in a worktree
   that is cleaned up; the dispatch reports no PR. Matches mika#1364 evidence rows mika#855 and
   mika#1179 (`callback_delivered_without_pr_url`) and the mika#1172 "wip authored but no PR".

### Why the issue body's "suspected root cause" is one layer too shallow

The issue body hypothesizes the rebase "silently failed". Reading the code, the rebase guard is
present and correct; the gap is the **push after a successful rebase**. The issue *title*
(`_push_branch lacks --force-with-lease after rebase`) is the accurate conclusion. This plan
implements the title. AC#1 and AC#2 (rebase-before-pilot, REBASE_CONFLICT halt) are therefore
**already satisfied** by the existing guard — this plan verifies them with a test rather than
rebuilding them. See "Reconciliation with the issue ACs" below for the one intentional divergence.

### Origin learning (do not re-derive)

- `docs/solutions/logic-errors/stale-base-conflicting-prs-no-self-heal-2026-04-23.md` — the
  origin of the rebase-or-abort guard. It established `BEHIND > 0` rebasing and the
  `STATUS=REBASE_CONFLICT` discriminator, but did **not** make the subsequent push force-aware.
  #1364 is the unhealed remainder of that same fix.
- `docs/solutions/logic-errors/mid-session-duplicate-commit-pre-push-guard-2026-05-26.md` —
  the `_check_duplicate_commits` (mika#784) guard, which *also* rebases (dispatch-lib.sh:848)
  and so feeds the same divergence-then-non-force-push interaction.
- `docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md` — manual
  recovery of exactly this stranded-work class; this plan removes the need for it.

---

## Reconciliation with the issue ACs

| AC | Status after this plan | Note |
|----|------------------------|------|
| AC#1 — rebase-onto-`origin/main` before pilot when reusing a remote branch | **Already satisfied** (guard at :292-324); verified by U3 test | **Intentional divergence:** the AC says ">5 commits behind"; the existing guard rebases on `BEHIND > 0`. This plan **keeps `> 0`** — it is strictly safer (a 1-5 commit stale base produces the same strand) and is the established design. The AC's ">5" describes the observed 66-commit case, not a deliberate threshold. No threshold knob is introduced (KISS). |
| AC#2 — rebase failure → `STATUS=REBASE_CONFLICT` + halt, pilot never runs | **Already satisfied** (:315-322 `exit 1` precedes `_run_claude_pilot`); verified by U3 test | — |
| AC#3 — regression test in `test-dispatch-lib.sh` | **U3** | Plus a test for the title fix (rebased push lands). |
| AC#4 — rebase failure mode captured in task result, not lost to `2>/dev/null` | **U2** | Capture stderr from both rebase sites. |
| Title — `_push_branch` must force-with-lease the rebased history | **U1** | The core fix. |

---

## Key Technical Decisions

### KTD-1: Detect divergence in `_push_branch`, force-with-lease only then

`_push_branch` must distinguish three cases after its existing `fetch origin "$BRANCH"`:

- **First push** (`origin/$BRANCH` does not exist) → plain `git push -u` (unchanged).
- **Fast-forward / linear ahead** (`origin/$BRANCH` *is* an ancestor of `HEAD`) → plain
  `git push` (unchanged — no force needed, no risk).
- **Diverged** (`origin/$BRANCH` exists but is *not* an ancestor of `HEAD` — the rebase rewrote
  history) → `git push --force-with-lease=$BRANCH:origin/$BRANCH`.

Divergence test: `git merge-base --is-ancestor "origin/$BRANCH" HEAD` (exit 0 = ancestor =
fast-forwardable = plain push; non-zero = diverged = needs lease-force). This is a pure,
side-effect-free local query against refs already fetched at line 781.

**Why `--force-with-lease` and not `--force`:** the lease pins the expected remote value to the
SHA observed by the line-781 fetch. If the remote advanced between that fetch and the push
(concurrent dispatch, human push), the push aborts instead of clobbering. The explicit
`=$BRANCH:origin/$BRANCH` form names the expected ref rather than relying on the implicit
remote-tracking ref, so the safety check is unambiguous even if tracking config is unusual.

**Why this is safe to force here:** dispatch-lib *owns* dispatch branches. The only history on
`origin/$BRANCH` we are overwriting is the stale pre-rebase tip we deliberately rebased away.
We are not discarding anyone's reachable work — the rebase replayed those commits onto main
where divergence is non-conflicting, and conflicting cases never reach push (they halt at
REBASE_CONFLICT, U3).

### KTD-2: Surface the rebase failure mode (AC#4)

Both rebase sites currently run `git rebase origin/main 2>/dev/null`, discarding stderr:

- `_set_up_worktree` (:313) — on failure builds `STATUS=REBASE_CONFLICT` from the conflicted
  file list but never includes *why* the rebase failed (conflict vs. dirty-tree vs. other).
- `_check_duplicate_commits` (:848) — on failure just aborts and `return 1`s; the dedup-rebase
  failure reason is invisible.

Capture stderr to a temp file (the `_push_branch` line-796 `mktemp` idiom is the local
convention), and on failure append the captured text (and a coarse classification — `conflict`
when `--diff-filter=U` is non-empty, else `other`) to `RESULT`. This converts AC#4's "silent /
conflict / aborted" ambiguity into a logged discriminator without changing control flow.

### KTD-3: No change to the clean paths

First-push and fast-forward pushes are untouched. `BEHIND == 0` (already-fresh base) skips the
rebase entirely and the push is a normal ahead-push. This bounds blast radius to exactly the
reused-stale-branch path that #1364 is about.

---

## Implementation Units

### U1. Make `_push_branch` divergence-aware (force-with-lease)

**Goal:** Land a rebased branch onto its diverged stale remote tip instead of failing the push.
This is the core fix (issue title).

**Requirements:** Issue title; unblocks the strand behind AC#1/#2's already-correct rebase.

**Dependencies:** none.

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh` (modify `_push_branch`, ~:780-807)
- `skills/bundled/_shared/test-dispatch-lib.sh` (U3 covers tests)

**Approach:**
- After the existing `fetch origin "$BRANCH"` (:781) and the `rev-parse --verify origin/$BRANCH`
  existence branch (:784-789), compute the push mode per KTD-1 using
  `git merge-base --is-ancestor "origin/$BRANCH" HEAD`.
- When `origin/$BRANCH` does not exist → keep the current first-push `push -u` path.
- When it exists and is an ancestor of HEAD → keep plain `push`.
- When it exists and is *not* an ancestor → `push --force-with-lease=$BRANCH:origin/$BRANCH`.
- **Ancestry-probe-error fallback (F2 — failure semantics documented, not masked):** if
  `merge-base --is-ancestor` itself errors (shallow clone; ref verified by `rev-parse` but its
  SHA not in the local object store), fall back to plain push and add a comment at that branch:
  *"ancestry probe failed → plain push → may reject as non-fast-forward if remote diverged; work
  remains local, operator must manually rescue."* This is **current behavior, not a regression**,
  and it deliberately does NOT silently re-enable-then-hide the strand: on rejection it lands in
  the existing `Push: FAILED — commits remain local-only` arm, which is the visible signal. We
  treat unknown ancestry as non-diverged (no surprise blind force), accepting that the rare
  probe-error case keeps today's behavior rather than risking a force on uncertain state.
- Keep the existing `2>"$push_err"` capture, success/`Push: pushed`, and failure/`Push: FAILED`
  arms. On a lease-stale abort, the existing failure arm already surfaces git's message — extend
  the failure RESULT text to name "remote advanced since fetch (lease aborted)" when the push
  error contains the stale-info marker, so the operator can tell a lease abort from a perms
  failure.
- Leave the `_check_duplicate_commits` pre-push guard call (:773) ahead of this logic unchanged
  — it still runs first; its own rebase (U2) feeds the same divergence detection here.

**Technical design (directional, not implementation spec):**
```
# after fetch origin "$BRANCH"
if origin/$BRANCH does not exist:        push -u origin "$BRANCH"          # first push
elif merge-base --is-ancestor origin/$BRANCH HEAD:
                                         push origin "$BRANCH"             # fast-forward
else:                                    push --force-with-lease=$BRANCH:origin/$BRANCH origin "$BRANCH"
```

**Patterns to follow:** existing `push_err=$(mktemp ...)` capture idiom (:795-807); existing
`rev-parse --verify "origin/$BRANCH"` existence probe (:784).

**Test scenarios (implemented in U3):**
- Diverged (stale remote tip, branch rebased onto advanced main, non-conflicting): force-with-lease
  push succeeds; `origin/$BRANCH` ends pointing at the rebased HEAD; `RESULT` contains
  `Push: pushed`.
- First push (no `origin/$BRANCH`): plain `push -u` succeeds; no force used.
- Fast-forward (origin/$BRANCH is ancestor of HEAD, one new local commit): plain push succeeds;
  no force used.
- Lease abort (remote `origin/$BRANCH` advanced after the fetch to a value the lease doesn't
  expect): push fails, work remains local-only, `RESULT` carries `Push: FAILED` and the lease
  marker. (Asserts we did not blind-`--force`.)

**Verification:** `bash skills/bundled/_shared/test-dispatch-lib.sh` passes; the diverged-push
test fails on `main` (current code) and passes with this change.

---

### U2. Surface rebase failure mode at both rebase sites (AC#4)

**Goal:** Replace `git rebase origin/main 2>/dev/null` with stderr-capturing variants so the
rebase failure reason reaches `RESULT`/logs instead of `/dev/null`.

**Requirements:** AC#4.

**Dependencies:** none (independent of U1; both edit `_push_branch`/`_set_up_worktree` regions
but non-overlapping lines).

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh` (`_set_up_worktree` rebase :313-323;
  `_check_duplicate_commits` rebase :848-857)

**Phase 0 Pin — both-sites claim verified (F1, resolves the BLOCKING finding):** Read
`dispatch-lib.sh:843-858` as shipped (mika#784, CLOSED). Confirmed: `_check_duplicate_commits`
rebases at `:848` with the **same `2>/dev/null` suppression** as the `:313` setup site, and its
failure path is **distinct** — `git rebase --abort` (:854) + `echo "ERROR: duplicate-commit
rebase failed…"` (:855) + `return 1` (:857); there is **no** `STATUS=REBASE_CONFLICT`/`exit 1` and
no pre-existing stderr capture at this site. Conclusion: U2 applies to **both** sites (both
suppress stderr), but the two integrations are different and must not be conflated:
  - `:313` → surface captured reason into the `STATUS=REBASE_CONFLICT` `RESULT` block; preserve `exit 1`.
  - `:848` → surface captured reason into the existing `ERROR:` log + (optionally) `RESULT`;
    preserve `return 1`. This `return 1` propagates to `_push_branch`'s `Push: SKIPPED` arm
    (`:773-777`), so the dedup-rebase reason now explains *why* the push was skipped.
U2 does not introduce a second abort/return path at `:848` — it only swaps the stderr sink and
appends the reason to the path mika#784 already ships.

**Approach:**
- `_set_up_worktree`: capture rebase stderr to a temp file; on success keep the existing
  `Rebased ...` log; on failure include the captured stderr and a coarse mode classification
  (`conflict` if `diff --diff-filter=U` non-empty after the failed rebase, else `other`) in the
  `STATUS=REBASE_CONFLICT` `RESULT` block before `rebase --abort` resets the index. Preserve the
  existing `exit 1`.
- `_check_duplicate_commits`: same capture; on failure append the dedup-rebase stderr to the
  existing `ERROR:` log line (:855) and (optionally) to `RESULT` so the `Push: SKIPPED` path
  (:774-777) explains the underlying rebase failure. Preserve the existing `return 1` (:857) —
  do not convert it to `exit 1` or add a REBASE_CONFLICT block here.
- Capture order matters: read conflicted-file list / stderr **before** `rebase --abort` (abort
  resets the index) — the existing `_set_up_worktree` code already orders this correctly; mirror
  it in `_check_duplicate_commits`.

**Patterns to follow:** the existing conflicted-file capture-before-abort ordering at :316-317;
the `mktemp` stderr-capture idiom at :796.

**Test scenarios (implemented in U3):**
- Conflicting rebase in `_set_up_worktree`: `RESULT` contains `STATUS=REBASE_CONFLICT`, the
  conflicted filename, AND a non-empty captured-reason / mode token; the run exits non-zero
  before any pilot invocation marker is written.
- (Coarse) the classification token is `conflict` when there is a real merge conflict.

**Verification:** the REBASE_CONFLICT test asserts the reason token is present (absent on `main`).

---

### U3. Regression tests in `test-dispatch-lib.sh` (AC#3 + title)

**Goal:** Lock in both behaviors: pilot never runs on a stale base that fails to rebase, and a
rebased branch's push lands against a stale remote.

**Requirements:** AC#3; regression guard for U1 and U2.

**Dependencies:** U1, U2 (tests assert their behavior).

**Files:**
- `skills/bundled/_shared/test-dispatch-lib.sh` (add cases)

**Approach:**
- Build the fixture from **throwaway local git repos only** — a bare repo as `origin` plus a
  working clone, wired with `file://`/path remotes. **Never touch the real `origin`** (per the
  test-harness safety discipline that also forbids calling real broadcast/stop in tmux tests).
- Reuse whatever harness `test-dispatch-lib.sh` already provides for sourcing dispatch-lib
  functions in isolation; if functions assume globals (`$WORKTREE_DIR`, `$BRANCH`, `$SUB_REPO_DIR`,
  `$REPO`, `$RESULT`), set them to the fixture before calling.
- Simulate the stale-base state: create `origin/$BRANCH` at an old base, advance `origin/main`
  past it, then exercise the relevant function directly.

**Test scenarios:**
- **Stale base + conflicting main advance → halt (AC#2/AC#3):** advance main with a change that
  conflicts with the branch's commit; call the `_set_up_worktree` rebase guard path; assert
  `RESULT` contains `STATUS=REBASE_CONFLICT`, assert the function returns non-zero, and assert no
  pilot-invocation side effect occurred (no `_run_claude_pilot` marker / `PRE_RUN_HEAD` unset).
- **Stale base + non-conflicting main advance → rebased push lands (title) — via the FULL
  `_push_branch` call chain (F3):** advance main non-conflictingly; rebase the branch (guard
  succeeds); commit a new local commit; **call `_push_branch` end-to-end** (NOT an isolated push)
  so it runs `_check_duplicate_commits` (:773 → :848) *first*, then the U1 ancestry classification,
  then the force-with-lease push. Assert: the push succeeds via force-with-lease and
  `origin/$BRANCH` now equals local HEAD. This exercises the exact call chain the bug occurs in
  (dedup-rebase → ancestry → force-with-lease), not just the `:313` setup-rebase path.
  *(Fails on `main` today — non-force push rejected.)*
- **Dedup-rebase → diverged → force composition (F3 companion):** seed the branch with a commit
  that is patch-equivalent to one already on advanced main (so `_check_duplicate_commits` rebases
  at :848 and drops/replays it) PLUS one genuinely-new commit; call `_push_branch`; assert the
  post-dedup-rebase HEAD is classified diverged and the force-with-lease push lands the new commit.
  Covers the architect's composition concern directly.
- **First push (no `origin/$BRANCH`) → plain push, no force:** assert success and that
  force-with-lease was not used (e.g., the push command path taken, or remote created fresh).
- **Fast-forward ahead → plain push, no force:** `origin/$BRANCH` is ancestor of HEAD + one new
  commit; assert plain push success.
- **Rebase failure reason surfaced (AC#4):** in the conflicting-rebase case, assert `RESULT`
  carries a non-empty captured reason / mode token (not just the bare conflict file list).

**Test expectation:** these are the feature-bearing tests for U1/U2; each names input, action,
expected outcome above.

**Verification:** `bash skills/bundled/_shared/test-dispatch-lib.sh` exits 0 with the new cases;
the two title/AC#4 cases demonstrably fail when run against unmodified `_push_branch`/rebase code.

---

## Scope Boundaries

**In scope:** `_push_branch` push-mode logic; rebase-stderr capture at the two existing rebase
sites; regression tests. All within `skills/bundled/_shared/`.

**Out of scope (non-goals):**
- Changing *when* the worktree is based on `origin/$BRANCH` vs `origin/main`
  (dispatch-lib.sh:279-288) — the mika#1311 reuse design stays; this plan fixes the *push*, not
  the base-selection.
- Introducing a `>5`-commit threshold knob (see Reconciliation table — intentionally declined).
- Touching `#1282` post-flight recovery logic — it already routes through `_push_branch`, so it
  inherits the fix for free; verifying that is a test observation, not a code change here.

### Deferred to Follow-Up Work
- If the lease-abort path (concurrent remote advance) proves common in practice, a single
  fetch-and-retry could be added to `_push_branch`. Deferred: no evidence it occurs; dispatch
  runs are serial per agent (one claude-pilot session at a time), so concurrent pushes to the
  same dispatch branch are not expected.

---

## System-Wide Impact

- **Autonomous loop:** directly unblocks the strand class in mika#1364 (4 stranded dispatches in
  one session). `dispatch-lib.sh` is copy-deployed (not in-binary) — the fix is live after
  `make deploy` syncs the bundled-skill copy; main-merged ≠ live until then.
- **Affected dispatch paths:** both `dev-pilot` (impl) and `dev-groom` dispatches share
  `_push_branch`; `#1282` post-flight recovery shares it too. All three benefit.
- **No schema, no API, no env-var changes.** Pure bash control-flow within one shared lib.

---

## Risks & Mitigations

- **Risk: force-with-lease clobbers legitimate remote work.** Mitigation: lease pins the expected
  remote SHA to the line-781 fetch; only the diverged (post-rebase) case forces; conflicting
  cases never reach push (halt at REBASE_CONFLICT). Dispatch branches are dispatch-owned and
  serial.
- **Risk: `merge-base --is-ancestor` edge cases (shallow clone, missing ref).** Mitigation: the
  ref was just fetched (:781); fall back to plain push if the ancestry probe errors (treat
  unknown as non-diverged → no surprise force).
- **Risk: test harness touches real origin.** Mitigation: U3 mandates throwaway local
  bare-repo fixtures; explicit assertion in the test that the remote URL is a local path.
- **Risk: regression in the clean first-push/fast-forward path.** Mitigation: KTD-3 leaves those
  branches byte-identical; U3 covers both explicitly.

---

## Sources & Research

- Issue: `senara-solutions/mika#1364` (title + body + N=4 evidence table).
- Code (HEAD `81891bb6`): `skills/bundled/_shared/dispatch-lib.sh` — `_set_up_worktree`
  (:189, reuse :279-288, rebase guard :292-324), `_push_branch` (:762-808),
  `_check_duplicate_commits` (:810-858), `#1282` post-flight recovery (:530-548).
- Learnings: `docs/solutions/logic-errors/stale-base-conflicting-prs-no-self-heal-2026-04-23.md`
  (rebase-guard origin), `docs/solutions/logic-errors/mid-session-duplicate-commit-pre-push-guard-2026-05-26.md`
  (mika#784 dedup guard), `docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md`
  (manual recovery this fix obviates).
